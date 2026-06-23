# Engineering Memory Update Log

## 2026-06-23
* **Initialization**: Created engineering wiki memory bundle.
* **Documentation baseline**: Collected reference projects and started Windows-first ADR drafting for the Rust CLI cleaner.
* **ADR baseline**: Added platform strategy and core runtime architecture ADRs; validated the engineering memory bundle.
* **Boundary ADRs**: Added workspace, Windows privilege/registry, and scan engine ADRs; updated memory bundle state.
* **Safety and state ADRs**: Added deletion/recovery, rule provenance, and configuration/local-state ADRs.
* **Workspace bootstrap**: Initialized the `rebecca` Rust workspace and verified it with `cargo fmt`, `cargo check --workspace`, and `cargo nextest run --workspace`.
* **State layout refinement**: Split local state and cache into distinct `state/` and `cache/` subdirectories under `%LOCALAPPDATA%\\Rebecca`.
* **MVP implementation plan**: Added `docs/plans/2026-06-23-001-feat-windows-cleanup-mvp-plan.md` to define the first Windows cleanup loop: owned rules, path expansion, safety policy, parallel scan, Recycle Bin execution, history, CLI output, and tests.
* **Windows cleanup MVP implementation**: Implemented the plan-first cleanup loop with typed rule targets, environment path expansion, safety policy, scanner, planner, execution backend contract, Windows Recycle Bin backend, history JSONL store, CLI commands, README, and 31 passing tests.
* **MVP commit**: Committed the Windows cleanup MVP with subject `feat: build windows cleanup mvp`; post-commit verification passed with `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo nextest run --workspace`.
* **Overlapping template dedupe fix**: Fixed planner deduplication so overlapping templates such as `%TEMP%` and `%LOCALAPPDATA%\\Temp` do not double-count the same path. Added regression tests and re-verified with 33 passing tests plus real dry-run smoke runs for `system`, `browser`, and `development --allow-moderate`.
