# Changelog

All notable changes to Rebecca will be documented in this file.

## [Unreleased]

### Added
- Human CLI output now surfaces decision summaries, reclaimable bytes, copyable next commands, cleanup-advice command summaries, cache-doctor health, and stderr-only TTY progress behavior so dry-run, inspect, and doctor flows are easier to act on.
- Human cleanup progress now uses compact phase, counter, cache, byte, and throughput messages while keeping spinner output on stderr and machine progress events unchanged.
- `rebecca inspect map` human output now ranks top entries with logical-size share, ASCII usage bars, compact long paths, and a `--screen-reader` mode that keeps the same facts without visual bars.
- `rebecca inspect map` human output now supports `--full-path`, `--no-bars`, and `--bar-width <COLUMNS>`, and requested map groups now show rank, share, and the same visual bars as top entries.
- Dry-run cleanup human output now explains required opt-ins included in the next command, gives pre-execution resolution hints for skipped or blocked targets, and points active-process warning plans at `rebecca doctor active-processes`.
- README now includes trimmed human-output examples for dry-run decisions, ranked disk maps, and cache doctor, with help-contract tests covering the primary user-facing CLI surfaces.
- `rebecca catalog` now provides a unified read-only catalog for cleanup rules, project artifact policies, warning gates, safety categories, and action kinds using the unified `rebecca.cli.v1` machine envelope.
- `rebecca inspect space` now provides read-only disk space insight with root totals, largest entries, diagnostics, JSON output, and NDJSON completion events.
- `rebecca inspect space` and `rebecca inspect map` now expose stderr-only human progress, `--no-progress`, and `--progress-detail target|file` controls while keeping JSON, table, and final human stdout clean.
- Inspect NDJSON now streams `started`, bounded `inspect-progress`, final report, and `completed` events with monotonic sequence numbers; file-level inspect progress remains opt-in with `--progress-detail file`.
- `rebecca inspect space --diagnostic-limit <N>` now bounds raw diagnostic samples while keeping complete diagnostic summary counts; use `--diagnostic-limit 0` for summary-only output.
- `rebecca inspect map` now provides a read-only ranked disk map with logical bytes, optional allocated bytes, depth-bounded top entries, diagnostics, JSON output, and NDJSON completion events.
- `rebecca inspect map --format ndjson` now streams bounded `map-entry` and `map-group` events before the final `inspect-map` completion payload so scripts and future TUI surfaces can consume ranked disk-map rows incrementally.
- `rebecca inspect map --table csv|tsv` now exports flat disk-map table rows for totals, roots, ranked entries, and requested groups without wrapping them in the JSON/NDJSON API envelope.
- `rebecca inspect map --table-row total|root|entry|group` now filters CSV/TSV table output to selected row kinds while keeping the default full-table export unchanged.
- `rebecca inspect map --min-logical-bytes`, `--entry-kind file|directory|other`, and `--path-contains` now filter ranked entries without changing root totals, groups, or diagnostic summaries.
- `rebecca inspect map --cleanup-advice` now annotates ranked entries with read-only cleanup advice from the cleanup rule catalog, project artifact policy, and protection policy, including cleanability status, matched rule/protection/artifact facts, required opt-ins, and a PowerShell-quoted dry-run command hint in table exports.
- `rebecca inspect map --advice-status cleanable|maybe-cleanable|contains-cleanable|protected|unknown` now filters ranked entries by cleanup advice status and implicitly enables cleanup advice.
- `rebecca inspect map --diagnostic-limit <N>` now bounds raw diagnostic samples while keeping complete diagnostic summary counts; use `--diagnostic-limit 0` for summary-only output.
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
- built-in rule catalog validation now also requires positive target-shape basis, rejects wide profile/drive-root glob discovery, and enforces shape-derived warnings such as `broad-discovery`, `privileged-location`, and `source-boundary`.
- `rebecca catalog validate` now exposes built-in rule and safety catalog health checks for maintainers in human, JSON, and NDJSON output modes.
- CI now runs `cargo deny check` and explicit `rebecca catalog validate` gates, with compatible dependency upgrades and a checked-in dependency policy.
- Cleanup planning now deduplicates equivalent existing directories by filesystem identity and keeps directory scan cache records valid for their configured freshness window despite root metadata churn.
- Dry-run cleanup previews now use the rebuildable scan cache by default, with `--no-scan-cache` available when a fully fresh estimate is preferred; `--yes` execution remains fresh-scan by default.
- The core performance matrix now includes an ordinary-rule planning benchmark for many directory targets.
- Scan-cache records now use a compact v1 format with scan backend, optional backend source, estimate confidence, optional filesystem identity fields, and USN checkpoint placeholders.
- Cleanup execution backends can now receive revalidated, non-overlapping safe batches while still returning per-target outcomes.
- `clean --scan-backend windows-native` now opts into a Windows native directory enumeration backend for cleanup plan estimates, with portable fallback when the native path is unsupported.
- The core performance matrix now includes a Windows native scan selection scenario for many-small-file fixtures.
- `rebecca-ntfs` now provides read-only NTFS MFT record parsing, fixup validation, file-name/data-size extraction, reparse detection, subtree aggregation, fixture tests, and a generated-record parser benchmark.
- `clean --scan-backend windows-ntfs-mft-experimental` now attempts read-only live NTFS/MFT metadata estimates on supported local NTFS volumes, reports estimate caveats, and falls back to a safe directory scanner when unsupported or unprivileged; full-volume MFT index reuse is reserved for explicit diagnostic fallback.
- `windows-ntfs-mft-experimental` now tries a read-only sequential `$MFT::$DATA` source before the per-record `FSCTL_GET_NTFS_FILE_RECORD` source, reads bounded aligned chunks, and keeps per-record FSCTL plus directory scanners as structured fallback paths.
- `rebecca-ntfs` now models NTFS records as owned parser DTOs with file-reference sequence numbers, attributes, streams, data runs, attribute-list entries, resident `$I30` directory index entries, logical/allocated/initialized size fields, parser caveats, and a generated parse-plus-index benchmark path.
- `rebecca-ntfs` now expands runlist-backed nonresident `$INDEX_ALLOCATION:$I30` streams, validates INDX fixups and VCN identity before trusting entries, and merges direct attribute-list `$INDEX_ALLOCATION` extension streams without recursive attribute-list expansion.
- `rebecca-ntfs` now reads fragmented `$INDEX_ALLOCATION:$I30` streams through a sequential chunk reader and covers multi-record fragmented directory indexes with deterministic fixtures.
- `rebecca-ntfs` now preserves `$I30` index-entry child VCNs and traverses reachable `$INDEX_ALLOCATION:$I30` nodes from `$INDEX_ROOT` instead of promoting every valid allocation record as directory evidence.
- `rebecca-ntfs` now exposes resolver-assisted single-record expansion so live targeted traversal can lazily merge direct `$ATTRIBUTE_LIST` extension `$DATA` and `$INDEX_ALLOCATION:$I30` streams without requiring a full MFT record set.
- `rebecca-ntfs` now enforces MFT record used-size and attribute-envelope bounds, resolves direct `$ATTRIBUTE_LIST` extension `$FILE_NAME`, `$STANDARD_INFORMATION`, and `$INDEX_ROOT:$I30` metadata into base records, and includes optional cargo-fuzz targets for MFT records, attribute lists, `$I30` indexes, and runlists.
- `rebecca-ntfs` now resolves bounded nonresident `$ATTRIBUTE_LIST` streams through the parser stream-source boundary, rejects recursive attribute-list expansion, and reports structured caveats for unsupported or invalid attribute-list streams.
- `rebecca-ntfs` now exposes typed directory-edge provenance and sequence confidence for `$FILE_NAME`, `$INDEX_ROOT:$I30`, and `$INDEX_ALLOCATION:$I30` relationships while retaining rejected stale edges as diagnostic facts.
- `rebecca-ntfs` now provides mirror-aware MFT record parsing so callers with bounded `$MFTMirr` bytes can recover corrupt or truncated primary `$MFT` records with an explicit `mft-mirror-record-used` caveat while preserving primary records as authoritative when valid.
- Experimental NTFS/MFT sequential full-index reads now pass bounded live `$MFTMirr` system-record bytes into the parser as best-effort recovery evidence; mirror read failures are caveated with `mft-mirror-read-failed` while primary `$MFT` parsing continues.
- Experimental NTFS/MFT disk-map metrics now expose parser-backed allocated bytes plus record-identity-based unique logical and unique allocated bytes when the backend has evidence, while keeping unknown values nullable and logical ordering as the default.
- NTFS fuzzing now has committed seed corpora for MFT records, attribute lists, `$I30` indexes, and runlists plus a `scripts/fuzz/run-ntfs-fuzz-smoke.ps1` smoke runner that validates target compilation even when `cargo-fuzz` is unavailable.
- Dogfood and performance scripts now emit richer machine-readable reports: inspect-map dogfood includes throughput, allocated/unique comparison states, caveat counts, and repeat stats; the performance matrix writes JSON, CSV, and Markdown reports and supports a successful `-SkipRun` report-generation smoke path.
- Cleanup, inspect, and scan-cache estimate provenance can now include structured `estimate_backend_evidence` for backend timings, counters, and cache hit/miss/write-skip reasons, and the performance matrix now has a baseline comparator with pass/regression/improvement/skipped/missing classifications.
- CLI API v1 docs and schemas now cover `cache-inventory`, `cache-doctor`, `cache-prune-report`, and `estimate_backend_evidence`, including a cache doctor example and local-path sharing guidance.
- `scripts/release/run-release-gates.ps1` now provides a local release-facing gate that records formatting, clippy, workspace tests, dependency policy, catalog validation, cache inspect, dry-run cleanup, performance, and inspect-map dogfood evidence under one report directory.
- The manual `Release Gates` GitHub Actions workflow now runs the shared release gate wrapper, uploads release evidence artifacts, and can compare full benchmark output against a prior `release-gates` artifact baseline.
- Inspect-map dogfood reports now expose backend source kind, caveat code counts, full-index source flags, and NTFS mirror evidence fields in JSON, CSV, and Markdown so sequential `$MFTMirr` recovery can be audited without inspecting raw stdout payloads.
- Inspect-map dogfood reports now extract NTFS/MFT `completed_timings=` and `metrics=` evidence into `ntfs_mft_stage_timings` and `ntfs_mft_build_metrics`, covering raw `$MFT` bytes, `$MFTMirr` bytes, parsed records, targeted record probes, full-index FSCTL probes, and stream-source reads when timing diagnostics are enabled.
- Experimental NTFS/MFT volume-index caching now uses a typed volume identity and stable volume fingerprint generation internally, establishing the persistent USN/volume-index cache reuse boundary without changing scan output or cleanup authority.
- Experimental NTFS/MFT volume-index caching now has an opt-in versioned manifest store for persistent cache metadata, while full MFT index payload reuse and USN freshness capture remain deferred.
- Experimental NTFS/MFT volume-index caching now writes versioned `MftIndex` payload files beside configured manifests and can reuse them after live USN replay proves no journaled changes touched the requested target subtree.
- `inspect map --scan-backend windows-ntfs-mft-experimental` can now opt into the persistent NTFS/MFT volume-index store with `REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE=1`, and `scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1` verifies warm-build, unrelated-change replay hit, target-change rebuild, and post-rebuild hit phases with isolated cache evidence.
- `scripts/dogfood/run-ntfs-usn-replay-vhd-dogfood.ps1` now creates an isolated dynamic NTFS VHDX scratch volume for stable persistent-cache and USN replay dogfood evidence, detaching the VHD by default while keeping logs and reports under `target/`.
- Persistent NTFS/MFT volume-index diagnostics now expose `mft-persistent-cache-miss` and `mft-persistent-cache-write-skipped` estimate caveats so dogfood reports explain rebuilds or skipped payload writes without trace logs.
- `scripts/dogfood/run-ntfs-fixture-dogfood.ps1` now creates local NTFS-focused fixture trees for hardlinks, sparse files, compressed files, large directories, nested directories, and fragmentation candidates before running the canonical inspect-map backend comparison report.
- Experimental NTFS/MFT estimates now aggregate through a sequence-aware `MftIndex` that preserves hardlink path candidates, resolves direct `$ATTRIBUTE_LIST` extension-record `$DATA` and `$INDEX_ALLOCATION` streams, cross-checks resident and nonresident `$I30` directory entries, and counts each physical record once per subtree.
- The live experimental NTFS/MFT backend now feeds raw volume reads into the parser crate's stream-source boundary so large-directory index allocation can supplement parent edges while cancellation, source provenance, and portable fallback behavior remain owned by `rebecca-core`.
- The live experimental NTFS/MFT backend now uses targeted per-record FSCTL traversal by default, reading only records reachable from the requested target through `$I30` directory indexes instead of building a full-volume MFT index for ordinary estimates.
- The NTFS parser benchmark now covers parse-plus-index, stream-backed `$INDEX_ALLOCATION:$I30` expansion, and fragmented runlist reads with deterministic generated fixtures.
- `scripts/dogfood/run-inspect-map-report.ps1` now collects local `inspect map` backend evidence under `target/inspect-map-dogfood/`, derives JSON/CSV/Markdown reports from one JSON scan per backend run, exits non-zero for run failures or backend comparison mismatches by default, and includes a `-SelfTest` parser/report check.
- `inspect map --cleanup-advice` now annotates discovered app-leftover cache entries with `source: app-leftover`, structured installed-app context, app-data source, deletion style, and an `apps clean --dry-run` suggested command; CSV/TSV advice rows include matching `cleanup_app_*` context columns.
- Cleanup, purge, and `inspect space` estimate provenance now include optional `estimate_backend_source` values such as `windows-ntfs-mft-experimental-targeted-fsctl`, `windows-ntfs-mft-experimental-sequential`, and `windows-ntfs-mft-experimental-fsctl-record` so wrappers can distinguish the actual experimental source from the public backend selector.
- `inspect map --scan-backend windows-ntfs-mft-experimental --group-by ...` now produces grouped disk-map output from the MFT traversal itself, using parsed `$FILE_NAME` modification FILETIME and stable NTFS record identities instead of falling back only because groups were requested.
- `inspect map` now accepts `--sort logical|allocated|files|unique` for ranked entries and `--group-sort logical|allocated|files|unique` for grouped distributions, preserving logical-byte ordering as the default.
- Scan-cache records now have a USN Journal validation model for checkpoint, journal id, range availability, and target-subtree change invalidation; missing USN support falls back to the normal cache policy.
- Cleanup rule targets now expose explicit search semantics in the manifest parser and catalog output, and glob discovery can reuse a per-plan directory enumeration index for compatible rules.
- Cleanup, purge, and `inspect space` outputs now expose additive v1 estimate provenance fields (`estimate_backend`, `estimate_backend_source`, `estimate_confidence`, `estimate_fallback_reason`, and `estimate_caveats`) while keeping `estimate_source` stable.
- `inspect space --scan-backend <BACKEND>` now accepts the same scan backend selectors as cleanup dry-runs for read-only space estimates.
- Cleanup execution now exposes a shared `execution_report` with per-action status, attempted paths, reclaimed bytes, pending reclaim bytes, skipped/failed/shadowed byte totals, and non-fatal execution warnings; `cache purge` now includes the same execution report alongside its cache-specific summary.
- `rebecca cache inspect`, `rebecca cache doctor`, and `rebecca cache prune` now expose Rebecca cache inventory, stale/corrupt record recommendations, namespace filtering, dry-run previews, and execution reports for targeted cache metadata pruning.

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
- `inspect space --format ndjson` and `inspect map --format ndjson` now use the streaming inspect lifecycle contract instead of final-only inspect event streams.
- `inspect lint --top` now bounds rendered duplicate, large-file, empty-file, and empty-directory sections while summary counters still reflect the full inventory.
- Scan traversal now reuses walker entry type information where possible while preserving root metadata checks and reparse-point protection.
- `history --limit` now loads only the bounded tail of non-empty history records before building the history projection.
- Scan-cache writes now use atomic replacement without strict file sync on the default hot path; strict sync remains available as an internal policy option.
- Scan-cache lookups now accept exact v1 records produced by portable, Windows native, or experimental NTFS/MFT scanners when root fingerprint and identity still match, and preserve optional backend-source provenance for cache hits.
- The performance matrix report schema now carries `backend_source_expectation`; live NTFS source timing is opt-in with `REBECCA_PERF_MATRIX_LIVE_NTFS=1` so default benchmark runs stay deterministic.
- Windows cleanup execution now batches Recycle Bin moves through the platform trash backend when possible and falls back to per-target reconstruction if a batch operation cannot report clean success.
- Experimental NTFS/MFT index construction now avoids quadratic directory-entry checks for large `$I30` indexes, makes fallback edges path-searchable, and builds live volume indexes outside the shared cache mutex.
- Experimental NTFS/MFT live volume index construction now has a default 20 second build budget, tunable with `REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS`, per-command single-flight volume builds, cached unavailable outcomes, timeout messages that name the active build stage and completed timings, split sequential `$MFT` read-versus-parse timings, and opt-in successful timing caveats via `REBECCA_NTFS_MFT_INDEX_TIMINGS=1`.
- Experimental NTFS/MFT live build diagnostics now include compact `metrics=` counters beside completed timings for processed records, sequential `$MFT` read chunks/bytes, `$MFTMirr` read chunks/bytes, targeted record attempts/successes, full-index FSCTL record attempts/successes, and stream-source read counts/bytes without adding extra filesystem reads.
- Experimental NTFS/MFT disk-map runs now surface build stages and counters through the inspect progress stream, including stage start/finish and cumulative backend metrics.
- Experimental NTFS/MFT sequential `$MFT` parsing now reads 8 MiB chunks and parses bounded chunk windows on a dedicated MFT parse pool, preserving cancellation and fallback behavior while reducing live record-parse time; full-volume raw `$MFT` reads remain the dominant budget cost on large or busy volumes.
- Experimental NTFS/MFT full-volume index construction is now an explicit diagnostic fallback controlled by `REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK=1`; the default experimental path is targeted traversal with backend source `windows-ntfs-mft-experimental-targeted-fsctl`.
- Experimental NTFS/MFT cleanup estimates still keep cleanup byte totals on logical unnamed `$DATA` streams, while disk-map surfaces can now expose parser-backed allocated and unique physical metrics when available; stream-backed NTFS records now derive allocated bytes from covering data runs so sparse and compressed files report physical cluster usage instead of header logical allocation, with `mft-data-run-allocated-by-cluster` caveats when that cluster-allocation evidence differs from the attribute header value.
- Experimental NTFS/MFT `$I30` expansion now reads allocation records by requested child VCN and reports unsupported multi-buffer-per-cluster geometry as a bounded caveat instead of using flat sequential offsets as authority.
- `inspect map --scan-backend windows-native` now uses the real Windows native directory inventory path for ranked disk maps instead of recording a portable fallback on supported local paths.
- `inspect map --scan-backend windows-native` now fills `allocated_bytes` from Windows file allocation metadata when available, while preserving logical-byte totals and falling back to nullable allocation for files whose allocation cannot be read.
- `inspect map --scan-backend windows-native` now adds structured `estimate_caveats` for compressed files, sparse files, hardlinked files, and skipped reparse points so machine consumers can explain why logical bytes, allocated bytes, and unique physical usage may diverge.
- `inspect map --scan-backend windows-native` now reports nullable `unique_logical_bytes` and `unique_allocated_bytes` by deduplicating stable Windows file ids, so hardlinked paths remain visible in path-ranked totals while unique physical usage is available when the host API exposes file identity metadata.
- `inspect map` now accepts repeated `--group-by extension|depth|age` plus `--group-limit` to emit bounded file distribution groups alongside ranked entries, with the same logical, allocated, and unique-byte accounting as the rest of the disk-map report.
- `inspect map` defaults to portable recursive inventory, while `--scan-backend windows-ntfs-mft-experimental` now uses targeted NTFS/MFT traversal for scoped roots and reserves full-volume MFT inventory for drive roots or explicit full-index diagnostics.
- Portable `inspect map` ranking now uses streaming post-order aggregation and a bounded top-entry heap instead of materializing the full directory tree before rendering entries.
- Portable `inspect map` now returns conservative partial reports with diagnostics when child entries cannot be read, child metadata disappears, child directories are unreadable, or child reparse points are skipped.
- `inspect space` human output now prints diagnostic grouped counts before bounded raw diagnostic samples.
- Cleanup execution now revalidates targets immediately before backend dispatch, shadows child targets covered by a parent delete to avoid double counting, skips reparse-point traversal during glob discovery, refuses preserve-root deletion when a child is reparse-like, and reports history write failures as execution warnings instead of failing after deletion.
- `inspect map` human output now prints diagnostic grouped counts before bounded raw diagnostic samples.

### Breaking
- warning-bearing cleanup targets are now blocked by default until their named warning is allowed with `--allow-warning <WARNING>`.
- the `rebecca.cli.v2` machine API namespace and docs were removed before release; the richer catalog and inspect payloads now emit under `rebecca.cli.v1`.
- `scripts/ntfs/run-live-mft-dogfood.ps1` was removed before release; use `scripts/dogfood/run-inspect-map-report.ps1` for repeatable live backend comparison evidence.
- `inspect artifacts` is the canonical project artifact insight command; `purge inspect` remains as a compatibility alias for existing command users.
- `cache purge` machine output no longer uses `mode: "delete"`; callers must handle `mode: "recoverable-delete"` and `mode: "permanent-delete"` plus the new `pending_reclaim_bytes`, `recoverably_deleted_entries`, and `permanently_deleted_entries` summary fields.
- Project artifact reclaim-limit skips now use `reason_code: "reclaim-limit-satisfied"` and can leave skipped targets with `estimate_source: "not-measured"` because later candidates are no longer measured after the limit is satisfied.
- The pre-release scan-cache format was reset to the single v1 identity format because no released consumers depend on the earlier experimental record numbering.
- `inspect-space` and `inspect-map` machine payloads now include `diagnostic_summary`; `diagnostics` is a bounded raw sample list rather than the authoritative count of all diagnostic observations.

### Fixed
- Machine JSON and NDJSON output now treats closed stdout pipes as a clean exit instead of panicking, so wrappers can safely stop reading early.
- `FSCTL_GET_NTFS_FILE_RECORD` outputs with kernel-deprotected or already-applied NTFS update-sequence fixups now parse correctly, allowing targeted live MFT estimates to use per-record FSCTL data without falling back to directory scanners on valid records.
- Experimental NTFS/MFT full-index diagnostics no longer discard parsed MFT evidence solely because stream-backed `$INDEX_ALLOCATION:$I30` expansion crosses the live build budget after records are available; those runs now stop further stream reads, keep cancellation as a hard error, and emit `mft-index-allocation-budget-exhausted`.
- Experimental NTFS/MFT targeted traversal and `$I30` cross-checking no longer promote DOS 8.3 directory aliases into visible traversal paths, preventing short-name duplicates from doubling disk-map file counts and logical bytes.
- Experimental NTFS/MFT estimate caveats now summarize repeated full-volume parse errors and cap repeated per-target caveat samples so JSON output remains bounded on real volumes.
- Experimental NTFS/MFT directory-index fallback caveats now stay attached to the affected parent child edge, so `$I30` parent-map supplements remain visible in subtree estimate caveats without duplicating child-entry caveats.
- Valid nonresident `$INDEX_ALLOCATION:$I30` directory indexes are no longer reduced to a caveat-only outcome; invalid, unreadable, sparse, compressed, or encrypted index allocation streams now produce bounded caveats without adding trusted child edges.
- Unreachable, cyclic, out-of-range, or VCN-mismatched `$INDEX_ALLOCATION:$I30` records are no longer counted as trusted large-directory children merely because they are present in the allocation stream.
- App-leftover cleanup advice no longer marks missing or reparse-like cache targets as cleanable; `inspect map --cleanup-advice` now aligns app-leftover advice with the planner/executor existing-target protection boundary.

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
