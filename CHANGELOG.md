# Changelog

All notable changes to `cargo-vibe` are documented here.

## v0.1.0 - Production Candidate

Initial production-oriented release of the `cargo vibe` orchestrator for Rust AI coding workflows.

### Added

- Unified CLI wrappers for `diff-risk`, `cargo-impact`, `spec-drift`, and `cargo-context`.
- `cargo vibe check` aggregation with text, JSON, and SARIF output.
- Guided `cargo vibe fix` loop with manual LLM handoff and gated verification.
- `.cargo-vibe.toml` support for thresholds, tool enablement, token budgets, and per-tool extra args.
- Cargo external subcommand support for `cargo vibe ...`.
- Regression coverage for missing subprocesses, strict risk failures, config loading, stdin context handoff, and fix-loop risk gating.

### Production Notes

- `cargo vibe check` degrades gracefully when companion tools are absent.
- `cargo vibe fix` requires `diff-risk`; it will fail closed if the risk gate is unavailable.
- The LLM generation step is manual in this release. No LLM APIs are called by `cargo-vibe`.
- For production CI, prefer individual SARIF uploads from companion tools when high-fidelity tool-specific annotations are required.
