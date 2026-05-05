# AGENTS.md

Orientation for AI assistants working in this repo. Loaded automatically
by Codex; readable by other agents that respect this convention.

## What this crate is

`cargo-vibe` is the unified CLI orchestrator for the Rust AI vibe coding
toolchain. It wraps four independent tools (`cargo-context`, `diff-risk`,
`cargo-impact`, `spec-drift`) into six subcommands:

- `cargo vibe check` — run all health checks, aggregate results
- `cargo vibe fix` — context → LLM → risk veto → impact verify → drift check → retry
- `cargo vibe risk` — diff-risk wrapper
- `cargo vibe impact` — cargo-impact wrapper
- `cargo vibe drift` — spec-drift wrapper
- `cargo vibe context` — cargo-context wrapper

## How it works

`cargo-vibe` is a **thin shell** — it does not reimplement any tool's logic.
It delegates to subprocesses via `std::process::Command`. This keeps the
dependency surface minimal and avoids coupling the tool versions.

The `FixLoop` in `orchestrator.rs` is the key innovation: it orchestrates
the context → generate → veto → verify → drift check cycle with user interaction
at the LLM generation step.

## Key types

| Type | File | Role |
|------|------|------|
| `Cli` / `Commands` | `main.rs` | clap-based CLI with 6 subcommands |
| `FixLoop` | `orchestrator.rs` | Orchestrated feedback loop |
| `GateResult` | `orchestrator.rs` | `Passed` / `Failed{reason}` / `TimedOut{attempts}` |
| `AggregatedReport` | `report.rs` | Merged findings from all tools with severity/tool counts |

## Subprocess dependency

Every subcommand except `fix` delegates to an external binary:

| Subcommand | Requires on PATH |
|-----------|-----------------|
| `check` | `diff-risk`, `cargo-impact`, `spec-drift` |
| `risk` | `diff-risk` |
| `impact` | `cargo-impact` |
| `drift` | `spec-drift` |
| `context` | `cargo-context` |
| `fix` | All four |

Graceful degradation: if a tool is missing, `cargo-vibe` prints a warning
and skips that check. It never fails on a missing tool — except `fix`, which
requires at least `diff-risk` to function.

## Design invariants

- **Subprocess-only.** Never link against the other tools as libraries.
  Version skew between orchestrator and tools is intentional and acceptable.
- **Parse, don't validate.** When parsing JSON output from subprocesses,
  be forgiving — tolerate extra fields, missing optional fields. The
  `parse_impact_json` and `parse_drift_json` helpers are deliberately loose.
- **`FixLoop` requires user interaction.** The LLM generation step is
  manual (user pastes context + prompt into their LLM). This is by design —
  the orchestrator does not call LLM APIs directly.
- **Config is loaded once.** `load_project_config()` is called at startup
  and passed through to each handler. No re-reading config files mid-run.

## Local-verify triple before any commit

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Commit style

Follow the existing tool convention: conventional-ish commits that lead with
the *why*, name the affected files/modules, and call out deferred scope
explicitly. Short `fix:` / `chore:` one-liners are fine for tiny changes.

## Honest caveats

- **`fix` subcommand is scaffolding.** The full autonomous loop (LLM API
  integration, automatic retry with feedback) is not yet implemented.
  Currently it assembles context, waits for user to apply LLM changes,
  then runs the verification gates.
- **`cargo vibe check` aggregates findings at a low fidelity.** Diff-risk
  text output is parsed heuristically for findings. For production CI,
  prefer running each tool separately with `--format sarif` and uploading
  individual SARIF files.
- **No tool version checking.** If `diff-risk` 0.1 is installed and 0.3
  is required, `cargo-vibe` won't detect the mismatch. Install compatible
  versions manually.

## Where to file things

- Bugs / feature requests: https://github.com/asmuelle/cargo-vibe/issues
- Interop with other tools: same org; each tool has its own repo under
  `github.com/asmuelle/`

## Don't commit without

- `fmt` + `clippy -D warnings` + `test` all green
- Tested `cargo vibe check --help` and at least one subcommand manually
- A commit message that names the files/modules touched
