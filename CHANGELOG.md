# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Added
- Executed `clean`, `purge`, `apps clean`, and `plan run` commands can now write a cleanup receipt with `--receipt <FILE>`. The receipt records what command ran, whether data went to trash or was permanently deleted, target outcomes, execution totals, and next steps such as `rebecca trash empty --yes`.
- `clean`, `purge`, and `apps clean` can now save dry-run cleanup plans with `--save-plan <FILE>`. New `rebecca plan inspect` and `rebecca plan run` commands let users review a saved plan, revalidate target metadata, and execute it later with `--yes`; stale targets are skipped with `saved-plan-target-changed`.
- `rebecca skills install`, `skills path`, and `skills remove` manage the packaged `rebecca-disk-cleaner` agent skill. The default install root is `~/.agents/skills`, with `--agent codex`, `--destination`, `--dry-run`, `--force`, and `delete`/`uninstall` aliases for other agent setups.
- `rebecca trash empty` previews or empties the system trash from Rebecca. On Windows it uses the Recycle Bin and supports `--drive C` or `--drive E`; normal cleanup still moves files to trash by default, and `--permanent` bypasses trash for `clean`, `purge`, or `apps clean`.

### Changed
- Help, README examples, `clean`/`purge` summaries, `inspect map`, and the TUI result screen now start with the action a user can take next: preview, move to trash, permanently delete, or empty trash.
- Long-running inspect and cleanup progress now shows clearer scan counters, rates, current scope, and cancellation hints; the TUI cleanup basket is now presented as a Reclaim Basket with selected-scope sizes before preview.
- The TUI cleanup workbench, saved-plan execution, and inspect cleanup advice now share the same rule loading, protected-path checks, scan-cache wiring, and cleanup execution safeguards as the CLI. TUI rendering, snapshots, mouse hit-testing, and replay also share one frame view of the disk map, so what users see and what actions select stay aligned.

## [0.3.0] - 2026-07-08

### Breaking
- Cleanup rule files now use shared family manifests under `rules/cleanup/<id>.toml` with per-platform blocks; the old top-level `platform` and `[[targets]]` shape is gone.
- Built-in safety data moved to shared Windows, Linux, and macOS safety blocks, and protected critical paths now use the generic `critical-path` reason code.
- `scan` shows cleanup rules for the current host by default. Use `catalog --platform <platform>` when you need to inspect another platform's catalog.
- Warning-gated targets are blocked until you pass the matching `--allow-warning <WARNING>`.
- Pre-release compatibility surfaces were removed: `rebecca.cli.v2`, `purge inspect`, `purge --list-artifacts`, and `scripts/ntfs/run-live-mft-dogfood.ps1`.
- Project artifact machine payloads use `portable.project-artifact-*` rule IDs, and inspect diagnostics now put complete counts in `diagnostic_summary` while keeping `diagnostics` as a bounded sample.
- `cache purge` machine output uses `recoverable-delete` or `permanent-delete` modes and reports pending reclaim separately from permanently reclaimed bytes.

### Added
- Linux and macOS are now real cleanup targets, with curated rules for temp files, browser caches, desktop app caches, Steam data, package manager caches, developer caches, thumbnails, logs, Homebrew, CocoaPods, and Xcode cache data.
- `rebecca tui` and the short alias `rebecca i` open an interactive cleanup workbench with root picking, disk-map navigation, Treemap drilldown, type and extension views, mouse support, live progress, preview, and typed confirmation before deletion.
- `inspect map` is much more useful for "what is using space here?" questions: it can show allocated bytes, unique bytes when the backend can prove them, filters, groups, CSV/TSV export, cleanup advice, compact human output, and screen-reader-friendly output.
- `inspect space`, `inspect artifacts`, and `inspect lint` cover read-only space reports, project artifact reports, duplicate groups, large files, empty files, and empty directories without writing cleanup history.
- Cleanup, purge, app cleanup, and cache cleanup now share recoverable-trash execution, execution reports, warning summaries, history warnings, and safer pre-delete target checks.
- Dry-run cleanup previews use the rebuildable scan cache by default; pass `--no-scan-cache` when you want a fresh measurement.
- `catalog` now lists cleanup rules, project artifact policies, warning gates, safety categories, action kinds, and `catalog validate` health checks.
- Shell completions can be generated for Bash, Zsh, Fish, PowerShell, and Elvish. Release archives and GitHub Releases include the generated completion files plus checksums.
- Machine output is easier to consume: structured parse errors, NDJSON progress events, bounded diagnostics, estimate provenance, backend evidence, schemas, and examples are documented under CLI API v1.
- The experimental Windows NTFS/MFT backend can read targeted MFT records for cleanup estimates and disk maps, including allocated and unique metrics, directory index evidence, attribute-list expansion, mirror recovery caveats, persistent cache experiments, and dogfood/performance reports.
- Rebecca ships a `rebecca-disk-cleaner` Codex skill and installer helper for preview-first cleanup workflows.

### Changed
- Cleanup execution prefers recoverable trash unless a command explicitly opts into permanent Rebecca cache deletion.
- Project artifact cleanup is stricter: targets need project context, known artifact directories stop traversal, reclaim-limit measurement stops after enough ranked candidates are measured, and missing configured roots become diagnostics instead of fake targets.
- Safety knowledge is selected from the requested platform instead of falling back to a Windows-shaped default.
- `clean`, `apps clean`, `purge`, `cache prune`, and `cache purge` reject `--dry-run --yes` before reading config or touching cache state.
- `inspect map` defaults to a streaming portable inventory. Windows native and NTFS experimental backends report provenance and fallback details instead of hiding uncertainty.
- Human progress stays on stderr, while JSON, NDJSON final payloads, CSV, and TSV output keep stdout clean.
- Scan-cache records use a compact v1 format with backend, source, confidence, logical-byte semantics, file identity fields, and USN placeholders. Stale cross-backend estimates no longer look like valid cache hits.
- MSRV is now Rust 1.95.0, and CI/release checks include dependency policy, catalog validation, macOS cleanup smoke, release gates, and crates.io publish-order validation.

### Fixed
- Closed stdout pipes no longer panic in JSON or NDJSON mode, so wrappers can stop reading early.
- Ubuntu tests and Linux-target clippy no longer trip over Windows-only cleanup rules or Windows-only NTFS support code.
- The release workflow now publishes `rebecca-safety` and `rebecca-ntfs` before crates that depend on them.
- App-leftover cleanup advice no longer marks missing or reparse-like cache targets as cleanable.
- Several NTFS/MFT parser edge cases no longer inflate or drop disk-map totals, including kernel-deprotected records, DOS 8.3 aliases, invalid or truncated index allocation, stale parent edges, repeated parse errors, and stream budget exhaustion.

## [0.2.0]

### Added
- project artifact cleanup targets now include a `project_artifact` explanation object in JSON and history output with the matched context, project root, and anchor path.
- CLI API docs now include a representative `purge` JSON example.

### Changed
- project artifact purge now requires explicit project context for built-in artifact kinds instead of accepting broad basename matches.
- known artifact directories now stop traversal even when they are not accepted as cleanup targets, reducing false positives from embedded toolchains and installed products.
- tag-driven releases now publish crates.io packages and cargo-dist GitHub Release assets from the same `release.yml` workflow.

### Fixed
- `purge --format json` and NDJSON completion events now report the `purge` command instead of `clean`.

## [0.1.1]

### Added
- `rebecca` now serves as the user-facing package name for both the CLI and the Rust library surface.

### Changed
- the CLI package and cargo-dist release assets were renamed from `rebecca-cli` to `rebecca`.
- the `rebecca` package now combines the CLI binary and the curated Rust library facade over `rebecca-core`, `rebecca-rules`, and `rebecca-windows`.

## [0.1.0]

### Added
- Windows-first cleanup CLI for system caches, app leftovers, and project artifacts.
- Plan-first `scan`, `clean`, `apps scan`, `apps clean`, `purge`, `cache purge`, `history`, `config paths`, `doctor permissions`, and shell completion commands.
- Built-in Windows rule catalog with owned provenance, protection policy, scan cache support, cleanup history, and machine-readable JSON / NDJSON output.
- Recovery-oriented execution through the Windows recoverable trash instead of permanent deletion.
- Installer verification, release integrity docs, and security guidance for local cleanup operations.
- `README.md` was restructured around a Mole-style product overview, quick start, safety design, and feature breakdown.
- cargo-dist release workflow, checksum, and preflight automation were added for GitHub Releases.
- Workspace crate metadata, dual MIT OR Apache-2.0 licensing files, and crates.io publish automation were added for release readiness.
- Release archives now include the changelog and license files.

### Changed
- GitHub Actions release and CI workflows now use upgraded checkout and artifact actions.
- Release documentation now covers both GitHub Release verification and crates.io installation.

### Fixed
- Rust 1.85 CI compatibility was restored by avoiding unstable let-chain syntax in planner and Steam library parsing code.
