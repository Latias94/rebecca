---
title: "NTFS Scan Performance Evidence - Plan"
type: "perf"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Scan Performance Evidence - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Make NTFS/MFT scan performance evidence machine-readable enough to separate raw `$MFT` read cost, record parsing cost, and stream-backed `$I30` expansion cost in dogfood reports. |
| Authority | Follows `docs/plans/2026-07-04-007-refactor-ntfs-full-index-budget-degrade-plan.md`, which showed full-index diagnostics can still hit the host-level script timeout even after late budget degradation. |
| Execution profile | Focused Rust and PowerShell diagnostics work with tests and documentation; no deletion behavior changes. |
| Stop condition | Stop if metrics require extra live reads, change cleanup authorization, expose private paths beyond existing dogfood artifacts, or make default machine output noisier when timing diagnostics are disabled. |
| Landing strategy | One conventional commit after focused NTFS tests, dogfood self-test, formatting, clippy, and diff checks pass. |

---

## Product Contract

Rebecca already emits stage timings through `mft-index-build-timing` and timeout messages, but those messages do not say how much evidence each stage processed.
The current dogfood report can tell that a full-index diagnostic timed out during `resolve-index-allocations`, but it cannot answer whether the expensive work was raw `$MFT` byte reading, record parsing, FSCTL record probing, or `$I30` stream expansion fanout.

This plan adds stable read-only metrics to the existing NTFS/MFT timing evidence and teaches dogfood reports to extract them.
It does not build the persistent USN Journal volume cache yet.
That larger cache needs these metrics first so its design is grounded in measured bottlenecks.

### Requirements

**NTFS build evidence**

- R1. The live NTFS/MFT build monitor records stable counters for processed records, raw `$MFT` bytes, `$MFTMirr` bytes, targeted record attempts/successes, full-index FSCTL record attempts/successes, and stream-source reads used by stream-backed attribute evidence.
- R2. Timing caveats, timeout messages, and budget-degraded caveats include the counters in a compact machine-parseable form when metrics are available.
- R3. Metrics are gathered from operations already being performed; the implementation must not add extra filesystem reads just to collect diagnostics.
- R4. Cancellation and build-budget semantics remain unchanged.

**Dogfood reporting**

- R5. `scripts/dogfood/run-inspect-map-report.ps1` extracts NTFS stage timings and build metrics from structured caveats and fallback reasons into JSON, CSV, and Markdown report fields.
- R6. Dogfood self-test covers repeated caveat extraction, timing extraction, metric extraction, CSV projection, and Markdown rendering.

**Safety and documentation**

- R7. The new evidence is read-only diagnostic data and must not affect cleanup estimates, cleanup advice, deletion eligibility, or rule ranking.
- R8. Changelog, performance docs, and engineering memory explain how to use the new metrics and keep persistent USN cache as deferred design work.

---

## Planning Contract

- KTD1. Reuse the existing timing caveat instead of adding a new default output surface. Operators who opt into `REBECCA_NTFS_MFT_INDEX_TIMINGS=1` should get richer evidence; normal users should not see extra caveats.
- KTD2. Keep counters in `NtfsMftBuildMonitor`, not in dogfood-only script state. The Rust backend owns the facts; scripts should only normalize them for reports.
- KTD3. Format metrics as sorted `key=value` pairs under a `metrics=` suffix. This matches the existing `completed_timings=` convention and lets dogfood parse timeout fallback reasons and successful caveats through one path.
- KTD4. Count only facts already known at the boundary: bytes returned by live volume reads, parsed record counts after successful source selection, FSCTL attempts/successes, and stream-source reads. Do not infer storage throughput from missing or failed bytes.
- KTD5. Defer persistent USN Journal cache implementation. The next cache plan should use these metrics to decide whether to cache full volume indexes, targeted subtree records, or per-directory freshness.

---

## Implementation Units

### U1. Add NTFS build counters to the monitor

- **Goal:** Extend `NtfsMftBuildMonitor` with typed counters and a shared build-summary formatter.
- **Requirements:** R1, R2, R4
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add a small `NtfsMftBuildMetric` enum and counter map beside the existing timing map. Provide increment helpers and a summary method that can render timings alone, metrics alone, or both. Replace ad hoc timeout/timing/budget strings with the shared summary.
- **Patterns to follow:** Existing `NtfsMftBuildStage`, `format_timing_summary`, `mft_build_timeout_reports_active_stage_and_completed_timings`, and `mft_build_timing_caveat_is_opt_in` tests.
- **Test scenarios:** Empty monitor emits no metrics suffix; timing caveat includes metrics after counters are recorded; timeout message includes `metrics=` when counters exist; budget-degraded caveat includes the same summary.
- **Verification:** `cargo nextest run -p rebecca-core scan::windows_ntfs_mft`.

### U2. Wire counters to live NTFS read paths

- **Goal:** Record useful performance facts without extra IO.
- **Requirements:** R1, R3, R4, R7
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Increment raw `$MFT` byte and chunk counters after successful sequential reads, `$MFTMirr` byte counters after successful mirror reads, parsed-record counters after the chosen record source or targeted resolver succeeds, targeted record attempt/success counters in the targeted resolver, full-index FSCTL attempt/success counters in the FSCTL source loop, and stream-source read/byte counters in `LiveNtfsIndexStreamSource::read_bytes_at`.
- **Patterns to follow:** Existing sequential source, FSCTL source, live stream source, and fake stream-source tests in `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`.
- **Test scenarios:** Fake stream source increments stream counters through the monitor; selected record sources record parsed-record counts once; metrics do not record failed pre-start work; cancellation still returns cancellation instead of a metric caveat.
- **Verification:** `cargo nextest run -p rebecca-core scan::windows_ntfs_mft`.

### U3. Extract NTFS metrics in dogfood reports

- **Goal:** Make the richer backend evidence visible without manually inspecting raw stdout JSON.
- **Requirements:** R5, R6
- **Files:** `scripts/dogfood/run-inspect-map-report.ps1`
- **Approach:** Add parser helpers that scan caveat values and fallback reason strings for `completed_timings=` and `metrics=` suffixes. Store joined strings on each run summary, project them to CSV, and add them to the Markdown backend-evidence table.
- **Patterns to follow:** Existing `Get-CaveatCodeCounts`, `Join-CaveatCodeCounts`, `Get-BackendSourceKind`, `Get-NtfsMirrorEvidence`, `Convert-RunsForCsv`, and `Invoke-SelfTest`.
- **Test scenarios:** Self-test sample with an `mft-index-build-timing` caveat yields stable timing and metric strings; CSV includes both fields; Markdown includes both fields without breaking existing mirror evidence checks.
- **Verification:** `pwsh -NoLogo -NoProfile -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest`.

### U4. Refresh docs and engineering memory

- **Goal:** Keep operators and future agents aligned on the new diagnostic surface and the deferred USN cache direction.
- **Requirements:** R7, R8
- **Files:** `CHANGELOG.md`, `docs/performance/perf-matrix.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Add concise Unreleased and performance-doc wording that names the new `metrics=` evidence and clarifies that persistent USN Journal caching remains the next design step, not part of this slice.
- **Patterns to follow:** Existing NTFS timing, mirror evidence, and budget-degradation entries in the same files.
- **Test scenarios:** Docs describe diagnostics as evidence only and do not imply cleanup authority or default full-index use.
- **Verification:** `git diff --check`.

---

## Verification Contract

| Gate | Command | Proves |
|---|---|---|
| Format | `cargo fmt --all --check` | Rust formatting stayed stable. |
| NTFS core tests | `cargo nextest run -p rebecca-core scan::windows_ntfs_mft` | Monitor counters, NTFS source accounting, cancellation, and existing NTFS behavior still pass. |
| Dogfood report self-test | `pwsh -NoLogo -NoProfile -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest` | Timing and metric extraction project correctly to JSON/CSV/Markdown. |
| Workspace type check | `cargo check --workspace` | Cross-crate API changes compile. |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Rust lint baseline remains clean. |
| Diff hygiene | `git diff --check` | No whitespace errors. |

When elevated live NTFS access is available, run one targeted dogfood on `docs\plans` with `REBECCA_NTFS_MFT_INDEX_TIMINGS=1` and confirm `ntfs_mft_stage_timings` plus `ntfs_mft_build_metrics` appear in `inspect-map-report.json` for the experimental backend.

---

## Definition of Done

- NTFS timing evidence includes stable machine-parseable `metrics=` fields when counters are present.
- Raw `$MFT`, `$MFTMirr`, FSCTL, parsed-record, and index-stream counters are wired from existing operations only.
- Dogfood JSON, CSV, and Markdown expose normalized NTFS stage timings and build metrics.
- Tests cover counter formatting and dogfood extraction.
- Changelog, performance docs, and engineering memory explain the diagnostic surface and keep persistent USN caching deferred.
- No abandoned experimental code or compatibility-only wrappers remain in the diff.
