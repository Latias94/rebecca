# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Added
- `rebecca catalog` now provides a unified read-only catalog for cleanup rules, project artifact policies, warning gates, safety categories, and action kinds using the `rebecca.cli.v2` machine envelope.
- `rebecca inspect space` now provides read-only disk space insight with root totals, largest entries, diagnostics, JSON output, and NDJSON completion events.
- `rebecca inspect artifacts` is now the canonical read-only project artifact insight command with JSON/NDJSON `inspect-artifacts` output, grouped totals, top targets, warning-gate awareness, reclaim-limit support, diagnostics, and no cleanup prompts or history writes.
- `rebecca inspect lint` now reports duplicate groups, large files, empty files, and empty directories without deleting files, remediating duplicates, or writing cleanup history.
- cleanup rule manifests now use Cleaner Manifest v1 with explicit warning declarations and future option/action shape support.
- the audited Windows safety catalog now owns safety categories, warning kinds, action kinds, and protected matcher data used by validation and catalog output.
- cleanup planning now supports explicit warning gates through `--allow-warning <WARNING>` and exposes warning summaries in machine output.
- project artifact policies now expose stable aliases, default age behavior, trim eligibility, deletion style, ranking metadata, and `--reclaim-limit-bytes` selection.
- CLI API v2 docs, schemas, and examples now cover catalog, inspect-space, inspect-artifacts, inspect-lint, and v2 error payload families.
- `purge inspect` now provides a read-only project artifact insight report with JSON/NDJSON v2 `inspect-artifacts` output, grouped totals, top targets, diagnostics, and no cleanup prompts or history writes.
- cleanup targets now expose `estimate_source` so machine consumers can distinguish fresh scans, scan-cache hits, unmeasured skipped/blocked targets, and legacy plans.
- project artifact cleanup plans now include `discovery_diagnostics` for partial discovery issues such as missing configured roots, unreadable directories, metadata failures, and skipped reparse points.
- `rebecca cache purge --permanent` now explicitly opts into irreversible Rebecca cache deletion after `--yes`.
- the built-in Windows catalog now includes narrow Zoom log, TeamViewer log, and VLC media cache rules derived from BleachBit behavior references and guarded by the `active-process` warning gate.
- the built-in Windows catalog now includes Chromium, Waterfox, Zen Browser, Thunderbird, and Adobe Reader cache rules with tests that keep history, cookies, mail, preferences, and document data out of scope.
- Chromium-family browser cache rules now include bounded shader, Dawn, extension-package, and legacy media cache directories while still excluding Network state, Safe Browsing journals, Preferences edits, history, cookies, sessions, and profile databases.
- built-in browser rules now fail catalog validation when targets leave the approved regenerable Chromium/Gecko cache boundary.
- built-in rule catalog validation now rejects filename/id drift, unsupported categories, non-canonical rule ids, untrimmed metadata, risky/dangerous built-in safety levels, and duplicate or non-canonical warning ids.
- `rebecca catalog validate` now exposes built-in rule and safety catalog health checks for maintainers in human, JSON, and NDJSON output modes.

### Changed
- `rebecca scan` now uses the unified catalog model internally while retaining the v1 `rule-catalog` output contract.
- `rebecca purge --list-artifacts` is now a compatibility listing generated from the same project artifact policy data as `rebecca catalog --kind project-artifact`.
- cleanup workflow internals now use explicit command/payload output contracts, shared CLI runtime cancellation, and dedicated human renderers instead of workflow-specific transport branches.
- planner and project artifact internals were split into focused modules, and configured purge roots now report stale or unreadable workspace entries as diagnostics while explicit `--root` values remain strict.
- project artifact discovery now applies policy ranking before reclaim-limit measurement so large cleanup plans can stop sizing lower-ranked candidates once the requested reclaim target is satisfied.
- v2 commands now use the command API registry for fatal JSON and NDJSON errors instead of the global v1 error envelope.
- `rebecca cache purge --yes` now moves rebuildable Rebecca cache entries to the Recycle Bin by default and reports pending reclaim bytes separately from permanently reclaimed bytes.
- Project artifact reclaim limits now stop measurement once ranked trim-eligible candidates satisfy the requested limit, leaving later candidates unmeasured instead of sizing the full candidate set first.
- Parallel project artifact and app-leftover measurement no longer buffers every file-level progress event before reporting target summaries.
- `inspect space --top` now keeps only a bounded top-entry accumulator while preserving exact root totals and deterministic output ordering.
- `inspect lint --top` now bounds rendered duplicate, large-file, empty-file, and empty-directory sections while summary counters still reflect the full inventory.
- Scan traversal now reuses walker entry type information where possible while preserving root metadata checks and reparse-point protection.
- `history --limit` now loads only the bounded tail of non-empty history records before building the history projection.

### Breaking
- warning-bearing cleanup targets are now blocked by default until their named warning is allowed with `--allow-warning <WARNING>`.
- new read-only cleanup-intelligence machine payloads and fatal errors use `rebecca.cli.v2`; consumers that assumed every machine envelope was `rebecca.cli.v1` must branch on `api_version`.
- `inspect artifacts` is the canonical project artifact insight command; `purge inspect` remains as a v2 compatibility alias for existing command users.
- `cache purge` machine output no longer uses `mode: "delete"`; callers must handle `mode: "recoverable-delete"` and `mode: "permanent-delete"` plus the new `pending_reclaim_bytes`, `recoverably_deleted_entries`, and `permanently_deleted_entries` summary fields.
- Project artifact reclaim-limit skips now use `reason_code: "reclaim-limit-satisfied"` and can leave skipped targets with `estimate_source: "not-measured"` because later candidates are no longer measured after the limit is satisfied.

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
