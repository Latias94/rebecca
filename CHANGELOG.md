# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Added
- `purge inspect` now provides a read-only project artifact insight report with JSON/NDJSON `project-artifact-insight` output, grouped totals, top targets, diagnostics, and no cleanup prompts or history writes.
- cleanup targets now expose `estimate_source` so machine consumers can distinguish fresh scans, scan-cache hits, unmeasured skipped/blocked targets, and legacy plans.
- project artifact cleanup plans now include `discovery_diagnostics` for partial discovery issues such as missing configured roots, unreadable directories, metadata failures, and skipped reparse points.

### Changed
- cleanup workflow internals now use explicit command/payload output contracts, shared CLI runtime cancellation, and dedicated human renderers instead of workflow-specific transport branches.
- planner and project artifact internals were split into focused modules, and configured purge roots now report stale or unreadable workspace entries as diagnostics while explicit `--root` values remain strict.

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
- Recovery-oriented execution through the Windows Recycle Bin instead of permanent deletion.
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
