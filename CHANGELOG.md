# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Added
- `rebecca catalog` now provides a unified read-only catalog for cleanup rules, project artifact policies, warning gates, safety categories, and action kinds using the unified `rebecca.cli.v1` machine envelope.
- `rebecca inspect space` now provides read-only disk space insight with root totals, largest entries, diagnostics, JSON output, and NDJSON completion events.
- `rebecca inspect artifacts` is now the canonical read-only project artifact insight command with JSON/NDJSON `inspect-artifacts` output, grouped totals, top targets, warning-gate awareness, reclaim-limit support, diagnostics, and no cleanup prompts or history writes.
- `rebecca inspect lint` now reports duplicate groups, large files, empty files, and empty directories without deleting files, remediating duplicates, or writing cleanup history.
- cleanup rule manifests now use Cleaner Manifest v1 with explicit warning declarations and future option/action shape support.
- the audited Windows safety catalog now owns safety categories, warning kinds, action kinds, and protected matcher data used by validation and catalog output.
- cleanup planning now supports explicit warning gates through `--allow-warning <WARNING>` and exposes warning summaries in machine output.
- project artifact policies now expose stable aliases, default age behavior, trim eligibility, deletion style, ranking metadata, and `--reclaim-limit-bytes` selection.
- CLI API v1 docs, schemas, and examples now cover catalog, catalog-validation, inspect-space, inspect-artifacts, inspect-lint, and error payload families.
- `purge inspect` now provides a read-only project artifact insight report with JSON/NDJSON `inspect-artifacts` output, grouped totals, top targets, diagnostics, and no cleanup prompts or history writes.
- Cleanup workflow NDJSON now emits target-level progress by default and adds `--progress-detail file` for ordinary cleanup scans that need verbose `file-measured` events.
- cleanup targets now expose `estimate_source` so machine consumers can distinguish fresh scans, scan-cache hits, unmeasured skipped/blocked targets, and legacy plans.
- project artifact cleanup plans now include `discovery_diagnostics` for partial discovery issues such as missing configured roots, unreadable directories, metadata failures, and skipped reparse points.
- `rebecca cache purge --permanent` now explicitly opts into irreversible Rebecca cache deletion after `--yes`.
- the built-in Windows catalog now includes narrow Zoom log, TeamViewer log, and VLC media cache rules derived from BleachBit behavior references and guarded by the `active-process` warning gate.
- the built-in Windows catalog now includes Chromium, Waterfox, Zen Browser, Thunderbird, and Adobe Reader cache rules with tests that keep history, cookies, mail, preferences, and document data out of scope.
- Chromium-family browser cache rules now include bounded shader, Dawn, extension-package, and legacy media cache directories while still excluding Network state, Safe Browsing journals, Preferences edits, history, cookies, sessions, and profile databases.
- built-in browser rules now fail catalog validation when targets leave the approved regenerable Chromium/Gecko cache boundary.
- built-in rule catalog validation now rejects filename/id drift, unsupported categories, non-canonical rule ids, untrimmed metadata, risky/dangerous built-in safety levels, copied/derived external rule provenance, and duplicate or non-canonical warning ids.
- `rebecca catalog validate` now exposes built-in rule and safety catalog health checks for maintainers in human, JSON, and NDJSON output modes.
- CI now runs `cargo deny check` and explicit `rebecca catalog validate` gates, with compatible dependency upgrades and a checked-in dependency policy.
- Cleanup planning now deduplicates equivalent existing directories by filesystem identity and keeps directory scan cache records valid for their configured freshness window despite root metadata churn.
- Dry-run cleanup previews now use the rebuildable scan cache by default, with `--no-scan-cache` available when a fully fresh estimate is preferred; `--yes` execution remains fresh-scan by default.
- The core performance matrix now includes an ordinary-rule planning benchmark for many directory targets.
- Scan-cache records now use a compact v1 format with scan backend, estimate confidence, optional filesystem identity fields, and USN checkpoint placeholders.
- Cleanup execution backends can now receive revalidated, non-overlapping safe batches while still returning per-target outcomes.
- `clean --scan-backend windows-native` now opts into a Windows native directory enumeration backend for cleanup plan estimates, with portable fallback when the native path is unsupported.
- The core performance matrix now includes a Windows native scan selection scenario for many-small-file fixtures.
- `rebecca-ntfs` now provides read-only NTFS MFT record parsing, fixup validation, file-name/data-size extraction, reparse detection, subtree aggregation, fixture tests, and a generated-record parser benchmark.
- `clean --scan-backend windows-ntfs-mft-experimental` now exposes an opt-in experimental backend selector that reports a caveat and falls back to a safe directory scanner until live NTFS volume indexing is enabled.
- Scan-cache records now have a USN Journal validation model for checkpoint, journal id, range availability, and target-subtree change invalidation; missing USN support falls back to the normal cache policy.
- Cleanup rule targets now expose explicit search semantics in the manifest parser and catalog output, and glob discovery can reuse a per-plan directory enumeration index for compatible rules.

### Changed
- The project MSRV is now Rust 1.95.0, with CI and release workflows pinned to the same toolchain and dependency lower bounds refreshed to current compatible versions.
- Scan measurement now goes through an internal backend contract that records the portable recursive backend and exact estimate confidence for future native backends.
- Ordinary cleanup rule planning now stages rule candidates before measurement and sizes eligible unique targets on Rebecca's bounded scan pool while preserving duplicate-target skips, safety decisions, metadata, and deterministic output.
- `rebecca scan` now uses the unified catalog model internally while retaining the v1 `rule-catalog` output contract.
- `rebecca purge --list-artifacts` is now a compatibility listing generated from the same project artifact policy data as `rebecca catalog --kind project-artifact`.
- cleanup workflow internals now use explicit command/payload output contracts, shared CLI runtime cancellation, and dedicated human renderers instead of workflow-specific transport branches.
- planner and project artifact internals were split into focused modules, and configured purge roots now report stale or unreadable workspace entries as diagnostics while explicit `--root` values remain strict.
- project artifact discovery now applies policy ranking before reclaim-limit measurement so large cleanup plans can stop sizing lower-ranked candidates once the requested reclaim target is satisfied.
- all machine-mode commands now use the command API registry for fatal JSON and NDJSON errors instead of a global fallback envelope.
- `rebecca cache purge --yes` now moves rebuildable Rebecca cache entries to the Recycle Bin by default and reports pending reclaim bytes separately from permanently reclaimed bytes.
- Project artifact reclaim limits now stop measurement once ranked trim-eligible candidates satisfy the requested limit, leaving later candidates unmeasured instead of sizing the full candidate set first.
- Parallel project artifact and app-leftover measurement no longer buffers every file-level progress event before reporting target summaries.
- `inspect space --top` now keeps only a bounded top-entry accumulator while preserving exact root totals and deterministic output ordering.
- `inspect lint --top` now bounds rendered duplicate, large-file, empty-file, and empty-directory sections while summary counters still reflect the full inventory.
- Scan traversal now reuses walker entry type information where possible while preserving root metadata checks and reparse-point protection.
- `history --limit` now loads only the bounded tail of non-empty history records before building the history projection.
- Scan-cache writes now use atomic replacement without strict file sync on the default hot path; strict sync remains available as an internal policy option.
- Scan-cache lookups now accept exact v1 records produced by either the portable recursive scanner or the Windows native directory scanner when root fingerprint and identity still match.
- Windows cleanup execution now batches Recycle Bin moves through the platform trash backend when possible and falls back to per-target reconstruction if a batch operation cannot report clean success.

### Breaking
- warning-bearing cleanup targets are now blocked by default until their named warning is allowed with `--allow-warning <WARNING>`.
- the `rebecca.cli.v2` machine API namespace and docs were removed before release; the richer catalog and inspect payloads now emit under `rebecca.cli.v1`.
- `inspect artifacts` is the canonical project artifact insight command; `purge inspect` remains as a compatibility alias for existing command users.
- `cache purge` machine output no longer uses `mode: "delete"`; callers must handle `mode: "recoverable-delete"` and `mode: "permanent-delete"` plus the new `pending_reclaim_bytes`, `recoverably_deleted_entries`, and `permanently_deleted_entries` summary fields.
- Project artifact reclaim-limit skips now use `reason_code: "reclaim-limit-satisfied"` and can leave skipped targets with `estimate_source: "not-measured"` because later candidates are no longer measured after the limit is satisfied.
- The pre-release scan-cache format was reset to the single v1 identity format because no released consumers depend on the earlier experimental record numbering.

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
