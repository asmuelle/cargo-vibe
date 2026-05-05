use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

mod orchestrator;
mod report;

use orchestrator::{FixLoop, GateResult};

/// cargo-vibe — unified AI-native development lifecycle for Rust.
///
/// Orchestrates four tools:
///   cargo context  — assemble token-budgeted context for LLMs
///   diff-risk      — score semantic risk of code changes
///   cargo impact   — find the blast radius of changes
///   spec-drift     — detect documentation/test/CI drift
#[derive(Parser, Debug)]
#[command(name = "cargo-vibe", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Project root directory.
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Path to .cargo-vibe.toml config.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run all health checks (risk + impact + drift).
    Check {
        /// Output format: text, json, or sarif.
        #[arg(long, default_value = "text")]
        format: String,
        /// Fail if any tool's threshold is exceeded.
        #[arg(long)]
        strict: bool,
        /// Git ref to diff against.
        #[arg(long, default_value = "HEAD")]
        since: String,
    },

    /// Score semantic risk of code changes (wraps diff-risk).
    Risk {
        /// Git ref to diff against.
        #[arg(long, default_value = "HEAD")]
        since: String,
        /// Risk threshold (0.0-10.0). Exit code 1 if exceeded.
        #[arg(long)]
        threshold: Option<f32>,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Find blast radius of changes (wraps cargo-impact).
    Impact {
        /// Git ref to diff against.
        #[arg(long, default_value = "HEAD")]
        since: String,
        /// Emit a cargo-nextest filter expression instead of report.
        #[arg(long)]
        test: bool,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Detect documentation/test/CI drift (wraps spec-drift).
    Drift {
        /// Git ref to diff against.
        #[arg(long)]
        diff: Option<String>,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: String,
        /// Exit non-zero only for drifts at this severity.
        #[arg(long, default_value = "notice")]
        deny: String,
    },

    /// Assemble token-budgeted context for LLMs (wraps cargo-context).
    Context {
        /// Preset: fix or feature.
        #[arg(long, default_value = "fix")]
        preset: String,
        /// Token budget.
        #[arg(long)]
        budget: Option<usize>,
        /// Read user prompt from stdin.
        #[arg(long)]
        stdin: bool,
    },

    /// Run the AI fix loop: context → LLM → risk check → impact verify → drift check.
    Fix {
        /// User prompt describing what to fix/implement.
        #[arg(long, short)]
        prompt: String,
        /// Maximum fix attempts.
        #[arg(long, default_value = "3")]
        max_attempts: usize,
        /// Risk threshold for auto-rejection.
        #[arg(long, default_value = "7.0")]
        risk_threshold: f32,
        /// Git ref to base the fix on.
        #[arg(long, default_value = "HEAD")]
        since: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Load project config if available
    let config = ai_tools_core::config::load_project_config(&cli.root);

    let result = match cli.command {
        Commands::Check { format, strict, since } => {
            run_check(&cli.root, &format, strict, &since, config.as_ref())
        }
        Commands::Risk { since, threshold, format } => {
            run_risk(&cli.root, &since, threshold, &format, config.as_ref())
        }
        Commands::Impact { since, test, format } => {
            run_impact(&cli.root, &since, test, &format, config.as_ref())
        }
        Commands::Drift { diff, format, deny } => {
            run_drift(&cli.root, diff.as_deref(), &format, &deny, config.as_ref())
        }
        Commands::Context { preset, budget, stdin } => {
            run_context(&cli.root, &preset, budget, stdin, config.as_ref())
        }
        Commands::Fix { prompt, max_attempts, risk_threshold, since } => {
            run_fix(&cli.root, &prompt, max_attempts, risk_threshold, &since, config.as_ref())
        }
    };

    match result {
        Ok(exit_code) => exit_code,
        Err(e) => {
            eprintln!("cargo-vibe: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run_check(
    root: &std::path::Path,
    format: &str,
    strict: bool,
    since: &str,
    config: Option<&ai_tools_core::config::VibeConfig>,
) -> Result<ExitCode> {
    let risk_threshold = config
        .and_then(|c| c.diff_risk.threshold)
        .or(config.and_then(|c| c.vibe.threshold))
        .unwrap_or(7.0);

    eprintln!("cargo-vibe: running health check against {since}...");
    let mut all_findings: Vec<ai_tools_core::finding::Finding> = Vec::new();

    // 1. Run diff-risk
    eprintln!("  [1/3] diff-risk...");
    if let Some(diff_output) = ai_tools_core::git_utils::unified_diff(root, since) {
        let tmp = std::env::temp_dir().join("cargo-vibe-diff.txt");
        std::fs::write(&tmp, &diff_output)?;
        let risk_output = Command::new("diff-risk")
            .args(["--threshold", &risk_threshold.to_string()])
            .stdin(Stdio::from(std::fs::File::open(&tmp)?))
            .output()
            .context("running diff-risk — is it installed? (cargo install diff-risk)")?;

        let risk_ok = risk_output.status.success();
        let risk_text = String::from_utf8_lossy(&risk_output.stdout);
        if !risk_ok {
            eprintln!("  diff-risk: FAILED (threshold {risk_threshold} exceeded)");
        } else {
            eprintln!("  diff-risk: passed");
        }
        if !risk_text.trim().is_empty() {
            all_findings.extend(parse_findings_from_text(&risk_text, "diff-risk"));
        }
    } else {
        eprintln!("  diff-risk: no diff available, skipped");
    }

    // 2. Run cargo-impact
    eprintln!("  [2/3] cargo-impact...");
    let impact_output = Command::new("cargo-impact")
        .args(["--since", since, "--format", "json", "--confidence-min", "0.5"])
        .current_dir(root)
        .output()
        .context("running cargo-impact — is it installed?")?;

    let impact_text = String::from_utf8_lossy(&impact_output.stdout);
    if impact_output.status.success() {
        let findings = parse_impact_json(&impact_text);
        let count = findings.len();
        all_findings.extend(findings);
        eprintln!("  cargo-impact: {count} finding(s)");
    } else {
        eprintln!("  cargo-impact: FAILED");
    }

    // 3. Run spec-drift
    eprintln!("  [3/3] spec-drift...");
    let mut drift_cmd = Command::new("spec-drift");
    drift_cmd.args(["--format", "json"]).current_dir(root);
    if let Some(_diff_ref) = None::<&str> {
        // Only add --diff if we have a diff reference
    }
    let drift_output = drift_cmd.output().context("running spec-drift — is it installed?")?;

    let drift_text = String::from_utf8_lossy(&drift_output.stdout);
    let drift_findings = parse_drift_json(&drift_text);
    let drift_count = drift_findings.len();
    all_findings.extend(drift_findings);
    eprintln!("  spec-drift: {drift_count} divergence(s)");

    // Render results
    let total = all_findings.len();
    let critical = all_findings
        .iter()
        .filter(|f| f.severity == ai_tools_core::finding::Severity::Critical)
        .count();
    let high = all_findings
        .iter()
        .filter(|f| f.severity == ai_tools_core::finding::Severity::High)
        .count();

    eprintln!("\n  Total: {total} finding(s) | Critical: {critical} | High: {high}");

    match format {
        "json" => {
            let report = report::AggregatedReport::new("cargo-vibe check", &all_findings);
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        "sarif" => {
            let renderer = ai_tools_core::sarif::SarifRenderer::new("cargo-vibe", "0.1.0");
            println!("{}", renderer.render(&all_findings));
        }
        _ => {
            for f in &all_findings {
                println!(
                    "{} [{}] {}:{}  {}",
                    f.severity.marker(),
                    f.rule.as_str(),
                    f.location.file.display(),
                    f.location.line,
                    f.message
                );
            }
        }
    }

    if strict && (critical > 0 || high > 0) {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_risk(
    root: &std::path::Path,
    since: &str,
    threshold: Option<f32>,
    _format: &str,
    config: Option<&ai_tools_core::config::VibeConfig>,
) -> Result<ExitCode> {
    let t = threshold.unwrap_or_else(|| {
        config
            .and_then(|c| c.diff_risk.threshold)
            .or(config.and_then(|c| c.vibe.threshold))
            .unwrap_or(7.0)
    });

    let diff = ai_tools_core::git_utils::unified_diff(root, since)
        .context("no git diff available — is this a git repository?")?;

    let tmp = std::env::temp_dir().join("cargo-vibe-diff.txt");
    std::fs::write(&tmp, &diff)?;

    let risk_output = Command::new("diff-risk")
        .args(["--threshold", &t.to_string()])
        .stdin(Stdio::from(std::fs::File::open(&tmp)?))
        .output()
        .context("running diff-risk — is it installed? (cargo install diff-risk)")?;

    let risk_text = String::from_utf8_lossy(&risk_output.stdout);
    let risk_err = String::from_utf8_lossy(&risk_output.stderr);
    if !risk_err.trim().is_empty() {
        eprintln!("{risk_err}");
    }
    print!("{risk_text}");

    if risk_output.status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn run_impact(
    root: &std::path::Path,
    since: &str,
    test: bool,
    format: &str,
    _config: Option<&ai_tools_core::config::VibeConfig>,
) -> Result<ExitCode> {
    let mut cmd = Command::new("cargo-impact");
    cmd.args(["--since", since]);
    if test {
        cmd.arg("--test");
    } else {
        cmd.args(["--format", format]);
    }
    cmd.current_dir(root);

    let output = cmd.output().context("running cargo-impact — is it installed?")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    if output.status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn run_drift(
    root: &std::path::Path,
    diff: Option<&str>,
    format: &str,
    deny: &str,
    _config: Option<&ai_tools_core::config::VibeConfig>,
) -> Result<ExitCode> {
    let mut cmd = Command::new("spec-drift");
    cmd.args(["--format", format]).args(["--deny", deny]);
    if let Some(d) = diff {
        cmd.args(["--diff", d]);
    }
    cmd.current_dir(root);

    let output = cmd.output().context("running spec-drift — is it installed?")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    if output.status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn run_context(
    root: &std::path::Path,
    preset: &str,
    budget: Option<usize>,
    stdin: bool,
    _config: Option<&ai_tools_core::config::VibeConfig>,
) -> Result<ExitCode> {
    let mut cmd = Command::new("cargo-context");
    cmd.args(["--preset", preset]);
    if let Some(b) = budget {
        cmd.args(["--max-tokens", &b.to_string()]);
    }
    if stdin {
        cmd.arg("--stdin");
    }
    cmd.current_dir(root);

    let output = cmd.output().context("running cargo-context — is it installed?")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(ExitCode::SUCCESS)
}

fn run_fix(
    root: &std::path::Path,
    prompt: &str,
    max_attempts: usize,
    risk_threshold: f32,
    since: &str,
    _config: Option<&ai_tools_core::config::VibeConfig>,
) -> Result<ExitCode> {
    let mut fix_loop = FixLoop::new(root, prompt, max_attempts, risk_threshold, since);

    eprintln!("cargo-vibe: starting fix loop with {max_attempts} max attempts...");
    eprintln!("  Risk threshold: {risk_threshold}");
    eprintln!("  Base ref: {since}");

    let result = fix_loop.run()?;

    match result {
        GateResult::Passed => {
            eprintln!("cargo-vibe: fix loop completed successfully.");
            Ok(ExitCode::SUCCESS)
        }
        GateResult::Failed { reason } => {
            eprintln!("cargo-vibe: fix loop failed: {reason}");
            Ok(ExitCode::from(1))
        }
        GateResult::TimedOut { attempts } => {
            eprintln!("cargo-vibe: fix loop exhausted {attempts} attempts without success.");
            Ok(ExitCode::from(1))
        }
    }
}

// ---- Helpers ----

fn parse_findings_from_text(text: &str, tool: &str) -> Vec<ai_tools_core::finding::Finding> {
    let mut findings = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("diff-risk") {
            continue;
        }
        // Try to parse lines like: "  🚨 [api_contract] src/lib.rs:42  Public API changed"
        if let Some(rest) = line
            .trim_start_matches(|c: char| c.is_whitespace() || "🚨⚠️🟡🔵✅🔥".contains(c))
            .strip_prefix('[')
        {
            if let Some(rule_end) = rest.find(']') {
                let _rule = &rest[..rule_end];
                let after = rest[rule_end + 1..].trim();
                if let Some(file_end) = after.find(|c: char| c == ':' || c.is_whitespace()) {
                    let file = &after[..file_end];
                    let rest_after_file = after[file_end..].trim();
                    let line_num: u32 = rest_after_file
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .unwrap_or(1);
                    let msg = rest_after_file
                        .trim_start_matches(|c: char| c.is_ascii_digit())
                        .trim()
                        .to_string();

                    let finding = ai_tools_core::finding::Finding::new(
                        ai_tools_core::finding::RuleId::Other,
                        ai_tools_core::finding::Severity::Medium,
                        ai_tools_core::finding::Confidence::Heuristic,
                        ai_tools_core::finding::Location::new(file, line_num),
                        msg,
                    )
                    .with_tool(tool);
                    findings.push(finding);
                }
            }
        }
    }
    findings
}

fn parse_impact_json(text: &str) -> Vec<ai_tools_core::finding::Finding> {
    #[derive(serde::Deserialize)]
    struct ImpactEnvelope {
        findings: Vec<ImpactFinding>,
    }
    #[derive(serde::Deserialize)]
    struct ImpactFinding {
        id: String,
        severity: String,
        #[allow(dead_code)]
        tier: String,
        evidence: String,
        #[serde(default)]
        suggested_action: Option<String>,
        #[serde(flatten)]
        kind: serde_json::Value,
    }

    let envelope: ImpactEnvelope = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    envelope
        .findings
        .into_iter()
        .map(|f| {
            let severity = match f.severity.as_str() {
                "high" => ai_tools_core::finding::Severity::High,
                "medium" => ai_tools_core::finding::Severity::Medium,
                "low" => ai_tools_core::finding::Severity::Low,
                _ => ai_tools_core::finding::Severity::Medium,
            };
            let rule = match f
                .kind
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("other")
            {
                "test_reference" => ai_tools_core::finding::RuleId::TestReference,
                "trait_impl" => ai_tools_core::finding::RuleId::TraitImpl,
                "ffi_signature_change" => ai_tools_core::finding::RuleId::FfiSignatureChange,
                "build_script_changed" => ai_tools_core::finding::RuleId::BuildScriptChanged,
                _ => ai_tools_core::finding::RuleId::Other,
            };
            let file = f
                .kind
                .get("test")
                .and_then(|v| v.get("file"))
                .or_else(|| f.kind.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            ai_tools_core::finding::Finding::new(
                rule,
                severity,
                ai_tools_core::finding::Confidence::Heuristic,
                ai_tools_core::finding::Location::new(file, 1),
                f.evidence,
            )
            .with_id(f.id)
            .with_tool("cargo-impact")
            .with_action(f.suggested_action.unwrap_or_default())
        })
        .collect()
}

fn parse_drift_json(text: &str) -> Vec<ai_tools_core::finding::Finding> {
    #[derive(serde::Deserialize)]
    struct DriftDivergence {
        rule: String,
        severity: String,
        location: DriftLocation,
        stated: String,
        #[allow(dead_code)]
        reality: String,
        risk: String,
    }
    #[derive(serde::Deserialize)]
    struct DriftLocation {
        file: String,
        line: u32,
    }

    let divergences: Vec<DriftDivergence> = match serde_json::from_str(text) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    divergences
        .into_iter()
        .map(|d| {
            let severity = match d.severity.as_str() {
                "critical" => ai_tools_core::finding::Severity::Critical,
                "warning" => ai_tools_core::finding::Severity::High,
                _ => ai_tools_core::finding::Severity::Medium,
            };
            let rule = match d.rule.as_str() {
                "symbol_absence" => ai_tools_core::finding::RuleId::SymbolAbsence,
                "compile_failure" => ai_tools_core::finding::RuleId::CompileFailure,
                "lying_test" => ai_tools_core::finding::RuleId::LyingTest,
                "ghost_command" => ai_tools_core::finding::RuleId::GhostCommand,
                _ => ai_tools_core::finding::RuleId::Other,
            };
            ai_tools_core::finding::Finding::new(
                rule,
                severity,
                ai_tools_core::finding::Confidence::Deterministic,
                ai_tools_core::finding::Location::new(d.location.file, d.location.line),
                format!("{} — expected: {}", d.risk, d.stated),
            )
            .with_tool("spec-drift")
        })
        .collect()
}
