# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Added
- `README.md` was restructured around a Mole-style product overview, quick start, safety design, and feature breakdown.
- Release workflow, SBOM, checksum, and preflight automation were added for GitHub Releases.

## [0.1.0]

### Added
- Windows-first cleanup CLI for system caches, app leftovers, and project artifacts.
- Plan-first `scan`, `clean`, `apps scan`, `apps clean`, `purge`, `cache purge`, `history`, `config paths`, `doctor permissions`, and shell completion commands.
- Built-in Windows rule catalog with owned provenance, protection policy, scan cache support, cleanup history, and machine-readable JSON / NDJSON output.
- Recovery-oriented execution through the Windows Recycle Bin instead of permanent deletion.
- Installer verification, release integrity docs, and security guidance for local cleanup operations.
