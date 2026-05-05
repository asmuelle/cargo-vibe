# TOOLS.md

Tool catalog for AI assistants operating in this repo. Lists every
available surface — binaries, subcommands, flags, and test commands —
so agents can make tool calls without guessing at `--help` output.

## Binary

| Name | Path | Description |
|------|------|-------------|
| `cargo-vibe` | `src/main.rs` | Unified orchestrator CLI |

## Subcommands

### `cargo vibe check`

Run all health checks (risk + impact + drift).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--format` | `text` / `json` / `sarif` | `text` | Output format |
| `--strict` | flag | false | Fail if any high/critical findings |
| `--since` | string | `HEAD` | Git ref to diff against |
| `--root` | path | `.` | Project root directory |
| `--config` | path | (auto) | Path to `.cargo-vibe.toml` |

### `cargo vibe risk`

Score semantic risk of code changes (wraps `diff-risk`).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--since` | string | `HEAD` | Git ref to diff against |
| `--threshold` | float | (from config or 7.0) | Risk threshold |
| `--format` | string | `text` | Output format |
| `--root` | path | `.` | Project root |
| `--config` | path | (auto) | Path to `.cargo-vibe.toml` |

### `cargo vibe impact`

Find blast radius of changes (wraps `cargo-impact`).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--since` | string | `HEAD` | Git ref to diff against |
| `--test` | flag | false | Emit cargo-nextest filter expression |
| `--format` | string | `text` | Output format |
| `--root` | path | `.` | Project root |

### `cargo vibe drift`

Detect spec drift (wraps `spec-drift`).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--diff` | string | (none) | Git ref for diff-aware mode |
| `--format` | string | `text` | Output format |
| `--deny` | `notice` / `warning` / `critical` | `notice` | Fail threshold |
| `--root` | path | `.` | Project root |

### `cargo vibe context`

Assemble context pack (wraps `cargo-context`).

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--preset` | `fix` / `feature` | `fix` | Collector preset |
| `--budget` | usize | (unlimited) | Token budget |
| `--stdin` | flag | false | Read prompt from stdin |
| `--root` | path | `.` | Project root |

### `cargo vibe fix`

Run the AI fix loop.

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--prompt` / `-p` | string | (required) | Fix/feature description |
| `--max-attempts` | usize | `3` | Maximum fix iterations |
| `--risk-threshold` | float | `7.0` | Auto-reject threshold |
| `--since` | string | `HEAD` | Base git ref |
| `--root` | path | `.` | Project root |

## Key source files

| File | Role |
|------|------|
| `src/main.rs` | CLI parser (`Cli`, `Commands`), subcommand dispatch, finding parsers |
| `src/orchestrator.rs` | `FixLoop` with context→LLM→risk→impact→drift pipeline, `Orchestrator` |
| `src/report.rs` | `AggregatedReport` with severity/tool counts, JSON serialization |
| `Cargo.toml` | Dependencies: `clap`, `anyhow`, `serde`, `serde_json`, `ai-tools-core` |

## Test commands

```bash
# Build only (no tests yet — this is a shell orchestrator)
cargo build

# Check compilation
cargo check

# Lint
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt --all --check

# Run with --help to verify CLI parses correctly
cargo run -- --help
cargo run -- check --help
cargo run -- fix --help
```

## Build commands

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run a subcommand (requires tools on PATH)
cargo run -- check --since HEAD
cargo run -- risk --since HEAD --threshold 7.0
cargo run -- context --preset fix --budget 16000
```
