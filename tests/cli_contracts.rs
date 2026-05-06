#![cfg(unix)]

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cargo-vibe")
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "cargo-vibe-{label}-{}-{nanos}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn minimal_path(fake_bin: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", fake_bin.display())
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write fake executable");
    let mut perms = fs::metadata(path)
        .expect("fake executable metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod fake executable");
}

fn run_git<I, S>(root: &Path, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_changed_git_repo() -> TempDir {
    let dir = TempDir::new("repo");
    fs::create_dir_all(dir.path().join("src")).expect("create src");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn value() -> u32 { 1 }\n",
    )
    .expect("write lib");
    run_git(dir.path(), ["init"]);
    run_git(dir.path(), ["config", "user.email", "ci@example.com"]);
    run_git(dir.path(), ["config", "user.name", "CI"]);
    run_git(dir.path(), ["add", "."]);
    run_git(dir.path(), ["commit", "-m", "initial"]);
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn value() -> u32 { 2 }\n",
    )
    .expect("modify lib");
    dir
}

fn run_cargo_vibe(args: &[&str], path: &str) -> Output {
    Command::new(bin())
        .args(args)
        .env("PATH", path)
        .output()
        .expect("run cargo-vibe")
}

#[test]
fn accepts_cargo_external_subcommand_argv() {
    let output = Command::new(bin())
        .args(["vibe", "check", "--help"])
        .output()
        .expect("run cargo-vibe help");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Run all health checks"));
}

#[test]
fn check_skips_missing_tools() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    let output = run_cargo_vibe(
        &[
            "--root",
            repo.path().to_str().unwrap(),
            "check",
            "--since",
            "HEAD",
        ],
        &minimal_path(fake.path()),
    );

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("diff-risk: not installed, skipped"));
    assert!(stderr.contains("cargo-impact: not installed, skipped"));
    assert!(stderr.contains("spec-drift: not installed, skipped"));
}

#[test]
fn strict_check_fails_on_diff_risk_threshold_failure() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    write_executable(
        &fake.path().join("diff-risk"),
        "#!/bin/sh\ncat >/dev/null\necho '  🚨 Auth / Permission Gate: src/lib.rs:1 — auth check removed'\nexit 1\n",
    );

    let output = run_cargo_vibe(
        &[
            "--root",
            repo.path().to_str().unwrap(),
            "check",
            "--strict",
            "--since",
            "HEAD",
        ],
        &minimal_path(fake.path()),
    );

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("diff-risk: FAILED"));
    assert!(stdout.contains("[auth_gate] src/lib.rs:1"));
}

#[test]
fn context_stdin_forwards_prompt_without_child_stdin_flag() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    write_executable(
        &fake.path().join("cargo-context"),
        "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--stdin\" ]; then\n    echo 'unexpected --stdin' >&2\n    exit 9\n  fi\ndone\ninput=$(cat)\nprintf 'received:%s\\n' \"$input\"\n",
    );

    let mut child = Command::new(bin())
        .args([
            "--root",
            repo.path().to_str().unwrap(),
            "context",
            "--stdin",
        ])
        .env("PATH", minimal_path(fake.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cargo-vibe context");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"why does this fail?\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("context output");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("received:why does this fail?"));
}

#[test]
fn context_propagates_child_failure() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    write_executable(
        &fake.path().join("cargo-context"),
        "#!/bin/sh\necho 'context failed' >&2\nexit 9\n",
    );

    let output = run_cargo_vibe(
        &["--root", repo.path().to_str().unwrap(), "context"],
        &minimal_path(fake.path()),
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("context failed"));
}

#[test]
fn explicit_config_can_disable_tools() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    let config = repo.path().join("cargo-vibe.toml");
    fs::write(
        &config,
        "[diff_risk]\nenabled = false\n[cargo_impact]\nenabled = false\n[spec_drift]\nenabled = false\n",
    )
    .expect("write config");

    let output = run_cargo_vibe(
        &[
            "--root",
            repo.path().to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "check",
            "--since",
            "HEAD",
        ],
        &minimal_path(fake.path()),
    );

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("diff-risk: disabled by config, skipped"));
    assert!(stderr.contains("cargo-impact: disabled by config, skipped"));
    assert!(stderr.contains("spec-drift: disabled by config, skipped"));
}

#[test]
fn explicit_config_extra_args_are_appended() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    let args_file = repo.path().join("impact-args.txt");
    let config = repo.path().join("cargo-vibe.toml");
    fs::write(
        &config,
        "[diff_risk]\nenabled = false\n[spec_drift]\nenabled = false\n[cargo_impact]\nextra_args = [\"--confidence-min\", \"0.9\", \"--cache\"]\n",
    )
    .expect("write config");
    write_executable(
        &fake.path().join("cargo-impact"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CARGO_IMPACT_ARGS_FILE\"\nprintf '{\"findings\": []}\\n'\n",
    );

    let output = Command::new(bin())
        .args([
            "--root",
            repo.path().to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "check",
            "--since",
            "HEAD",
        ])
        .env("PATH", minimal_path(fake.path()))
        .env("CARGO_IMPACT_ARGS_FILE", &args_file)
        .output()
        .expect("run cargo-vibe check");

    assert!(output.status.success());
    let args = fs::read_to_string(args_file).expect("read impact args");
    assert!(args.contains("--confidence-min\n0.9\n"));
    assert!(!args.contains("--confidence-min\n0.5\n"));
    assert!(args.contains("--cache\n"));
}

#[test]
fn bad_explicit_config_path_is_tool_error() {
    let fake = TempDir::new("fakebin");
    let missing = fake.path().join("missing.toml");

    let output = run_cargo_vibe(
        &["--config", missing.to_str().unwrap(), "check"],
        &minimal_path(fake.path()),
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read config"));
}

#[test]
fn fix_requires_diff_risk() {
    let repo = make_changed_git_repo();
    let fake = TempDir::new("fakebin");
    let mut child = Command::new(bin())
        .args([
            "--root",
            repo.path().to_str().unwrap(),
            "fix",
            "--prompt",
            "smoke",
            "--max-attempts",
            "1",
            "--since",
            "HEAD",
        ])
        .env("PATH", minimal_path(fake.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cargo-vibe fix");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("fix output");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("diff-risk"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("required"));
}
