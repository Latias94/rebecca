# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Changed
- project artifact purge now requires explicit project context for built-in artifact kinds instead of accepting broad basename matches.
- known artifact directories now stop traversal even when they are not accepted as cleanup targets, reducing false positives from embedded toolchains and installed products.

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
