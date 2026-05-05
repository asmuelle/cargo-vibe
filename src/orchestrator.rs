use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of a fix loop run.
pub enum GateResult {
    Passed,
    Failed { reason: String },
    TimedOut { attempts: usize },
}

/// Orchestrated fix loop: context → LLM → risk check → impact verify → drift check.
///
/// This implements the feedback loop described in the improvement plan:
/// 1. Build context pack
/// 2. Present context + prompt to LLM (user handles this part)
/// 3. Check risk of the diff
/// 4. Verify impact (run affected tests)
/// 5. Check for spec drift
/// 6. If anything fails, feed results back and retry
pub struct FixLoop {
    root: PathBuf,
    prompt: String,
    max_attempts: usize,
    risk_threshold: f32,
    since: String,
}

impl FixLoop {
    pub fn new(
        root: &Path,
        prompt: &str,
        max_attempts: usize,
        risk_threshold: f32,
        since: &str,
    ) -> Self {
        Self {
            root: root.to_path_buf(),
            prompt: prompt.to_string(),
            max_attempts,
            risk_threshold,
            since: since.to_string(),
        }
    }

    /// Run the fix loop. This is a guided process where the user is prompted
    /// at each step. The loop produces structured feedback for an LLM at
    /// each stage.
    pub fn run(&mut self) -> Result<GateResult> {
        for attempt in 1..=self.max_attempts {
            eprintln!("\n═══ Attempt {attempt}/{} ═══", self.max_attempts);

            // Step 1: Build context for the LLM
            eprintln!("[1/4] Building context...");
            let context = self.build_context()?;
            eprintln!("      Context: {} chars ready for LLM", context.len());

            // Step 2: User pastes context + prompt into LLM and applies changes
            eprintln!("[2/4] Context assembled. Paste it into your LLM with this prompt:");
            eprintln!("{}", "─".repeat(60));
            eprintln!("{}", self.prompt);
            eprintln!("{}", "─".repeat(60));
            eprintln!("      Press Enter after applying LLM changes (or 'q' to quit)...");

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if input.trim() == "q" {
                return Ok(GateResult::Failed {
                    reason: "user quit".to_string(),
                });
            }

            // Step 3: Risk check
            eprintln!("[3/4] Checking risk...");
            let diff = ai_tools_core::git_utils::unified_diff(&self.root, &self.since);
            let risk_passed = match diff {
                Some(diff_text) => {
                    if diff_text.trim().is_empty() {
                        eprintln!("      No changes detected — nothing to check.");
                        true
                    } else {
                        self.check_risk(attempt)
                    }
                }
                None => {
                    eprintln!("      No diff available, skipping risk check.");
                    true
                }
            };

            if !risk_passed {
                eprintln!("      Risk threshold exceeded. The LLM should generate a safer alternative.");
                if attempt < self.max_attempts {
                    eprintln!("      Feed this back to the LLM and try again.");
                    continue;
                } else {
                    return Ok(GateResult::Failed {
                        reason: "risk threshold exceeded after all attempts".to_string(),
                    });
                }
            }
            eprintln!("      Risk: passed");

            // Step 4: Impact + drift check
            eprintln!("[4/4] Verifying impact and drift...");
            let (impact_passed, drift_passed) = self.verify()?;

            if impact_passed && drift_passed {
                eprintln!("      All checks passed!");
                return Ok(GateResult::Passed);
            }

            if !impact_passed {
                eprintln!("      Impact check: some tests may be affected.");
            }
            if !drift_passed {
                eprintln!("      Drift check: docs/tests/CI may be stale.");
            }

            if attempt < self.max_attempts {
                eprintln!("      Feed the failures back to the LLM and try again.");
            }
        }

        Ok(GateResult::TimedOut {
            attempts: self.max_attempts,
        })
    }

    fn build_context(&self) -> Result<String> {
        // Try to use cargo-context if available
        let output = Command::new("cargo-context")
            .args(["--preset", "fix"])
            .current_dir(&self.root)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                Ok(String::from_utf8_lossy(&out.stdout).to_string())
            }
            _ => {
                // Fallback: build minimal context manually
                let mut ctx = String::new();
                ctx.push_str("# Project Context\n\n");

                if let Some(diff) =
                    ai_tools_core::git_utils::unified_diff(&self.root, &self.since)
                {
                    if !diff.trim().is_empty() {
                        ctx.push_str("## Recent Changes\n\n```diff\n");
                        // Truncate to reasonable size
                        let diff = if diff.len() > 4000 {
                            format!("{}...\n(truncated)", &diff[..4000])
                        } else {
                            diff
                        };
                        ctx.push_str(&diff);
                        ctx.push_str("\n```\n\n");
                    }
                }

                ctx.push_str("## Instructions\n\n");
                ctx.push_str(&self.prompt);
                Ok(ctx)
            }
        }
    }

    fn check_risk(&self, attempt: usize) -> bool {
        let diff = match ai_tools_core::git_utils::unified_diff(&self.root, &self.since) {
            Some(d) => d,
            None => return true,
        };

        let tmp = std::env::temp_dir().join(format!("cargo-vibe-fix-{attempt}.diff"));
        if std::fs::write(&tmp, &diff).is_err() {
            return true;
        }

        match Command::new("diff-risk")
            .args(["--threshold", &self.risk_threshold.to_string()])
            .stdin(std::process::Stdio::from(
                std::fs::File::open(&tmp).unwrap_or_else(|_| {
                    panic!("failed to open temp diff file")
                }),
            ))
            .output()
        {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                if !text.trim().is_empty() {
                    eprintln!("{}", text);
                }
                out.status.success()
            }
            Err(_) => {
                eprintln!("      diff-risk not available — skipping risk check.");
                true
            }
        }
    }

    fn verify(&self) -> Result<(bool, bool)> {
        let impact_ok = match Command::new("cargo-impact")
            .args(["--since", &self.since, "--fail-on", "high"])
            .current_dir(&self.root)
            .output()
        {
            Ok(out) => {
                if !out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout);
                    if !text.trim().is_empty() {
                        eprintln!("{}", text);
                    }
                    false
                } else {
                    true
                }
            }
            Err(_) => {
                eprintln!("      cargo-impact not available — skipping.");
                true
            }
        };

        let drift_ok = match Command::new("spec-drift")
            .args(["--format", "json", "--deny", "warning"])
            .current_dir(&self.root)
            .output()
        {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                let count = text.matches("\"rule\"").count();
                if count > 0 {
                    eprintln!("      spec-drift: {count} divergence(s) found.");
                    if !text.trim().is_empty() && text.len() < 2000 {
                        eprintln!("{}", text);
                    }
                }
                out.status.success()
            }
            Err(_) => {
                eprintln!("      spec-drift not available — skipping.");
                true
            }
        };

        Ok((impact_ok, drift_ok))
    }
}

/// Orchestrator for running all checks in sequence.
#[allow(dead_code)]
pub struct Orchestrator {
    root: PathBuf,
}

impl Orchestrator {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Check if all tools are available.
    pub fn check_availability(&self) -> Vec<(&'static str, bool)> {
        vec![
            (
                "diff-risk",
                Command::new("diff-risk")
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false),
            ),
            (
                "cargo-impact",
                Command::new("cargo-impact")
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false),
            ),
            (
                "spec-drift",
                Command::new("spec-drift")
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false),
            ),
            (
                "cargo-context",
                Command::new("cargo-context")
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false),
            ),
        ]
    }
}
