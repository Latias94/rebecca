---
title: "NTFS Full Index Budget Degradation - Plan"
type: "refactor"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Full Index Budget Degradation - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Keep parsed NTFS/MFT full-index evidence when `$I30` index-allocation expansion exhausts the live build budget after the record set is already available. |
| Authority | Builds on the dogfood evidence from `docs/plans/2026-07-04-006-refactor-ntfs-dogfood-evidence-plan.md` and the full-index timing guard described in `CHANGELOG.md`. |
| Execution profile | Small Rust backend behavior change with focused unit tests, documentation, and live dogfood evidence when the host permits it. |
| Stop condition | Stop if the change would trust incomplete stream bytes, suppress cancellation, make cleanup deletion depend on NTFS evidence, or remove portable fallback for earlier full-index failures. |
| Landing strategy | One conventional commit after focused NTFS tests, formatting, dogfood smoke, and diff checks pass. |

## Product Contract

Forced full-index dogfood showed that scoped targeted NTFS traversal is healthy, but explicit full-volume diagnostics can lose useful MFT evidence when the global budget expires after `$MFT` bytes have been read and records have been parsed. In the observed 60 second run, timeout happened during `resolve-index-allocations`; the command fell back to `portable-recursive`, so dogfood lost backend-source, mirror, and timing evidence that had already been gathered.

This plan changes only the late full-index budget behavior. If index-allocation expansion finishes after the budget, Rebecca should keep the `MftIndex` built from the available record set, attach a structured caveat, and keep cancellation authoritative. Earlier failures while opening the volume, reading `$MFT`, parsing records, or resolving the target still fall back as before.

### Requirements

- R1. A successful `NtfsRecordSet::resolve_with_stream_source` result is not discarded solely because the live build budget expired after the operation returned.
- R2. The backend emits a stable caveat when full-index index-allocation expansion exceeds the budget but still yields a usable index.
- R3. User cancellation remains a hard error and must not be downgraded into a caveat.
- R4. Earlier full-index timeout or platform failures continue to use the existing portable fallback path.
- R5. Documentation and release notes explain the new caveat as read-only diagnostic evidence, not cleanup authority.

## Planning Contract

- KTD1. Treat late `$I30` budget exhaustion as degraded evidence, not a source failure. The parser already records stream read failures as caveats; the core backend should preserve that record set when the only problem is the post-success budget check.
- KTD2. Keep the grace narrow. The grace applies after successful index-allocation resolution and only to the follow-up in-memory `MftIndex` build needed to surface the already-resolved records.
- KTD3. Preserve cancellation semantics. Any set cancellation token still aborts the command even if the budget-degraded path would otherwise continue.
- KTD4. Keep the caveat machine-readable. Use a stable code so dogfood reports can count it alongside `mft-index-build-timing`, mirror caveats, and fallback caveats.

## Implementation Units

### U1. Add late-budget measurement semantics

- **Goal:** Give the NTFS full-index builder a narrow way to measure a stage while checking timeout before it starts and checking only cancellation after a successful operation.
- **Requirements:** R1, R3, R4
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add a monitor helper that calls the existing budget check before the operation, records timings through the existing timing map, and uses `check_not_cancelled` rather than the full budget check after success. Use it only where the operation returns a usable evidence object.
- **Patterns to follow:** Existing `NtfsMftBuildMonitor::measure_checked`, `check_mft_build_progress`, and timeout tests in `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`.
- **Test scenarios:** An already-expired monitor still fails before resolution starts; a cancellation token set before or during resolution still fails.
- **Verification:** Focused `windows_ntfs_mft` tests pass.

### U2. Preserve full-index evidence after `$I30` expansion overruns budget

- **Goal:** Build and return `MftIndex` when index-allocation resolution returns successfully after crossing the live budget.
- **Requirements:** R1, R2, R3
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Update `build_mft_index_from_records` to use the new helper for `ResolveIndexAllocations`, add a `mft-index-allocation-budget-exhausted` caveat when `monitor.is_timed_out()` is true afterward, and allow the immediate in-memory `BuildMftIndex` stage to finish with cancellation-only checks.
- **Patterns to follow:** Existing `MFT_BUILD_TIMING_CAVEAT_CODE`, `ParseCaveat::new`, fake stream-source tests, and parser caveat preservation tests in the same module.
- **Test scenarios:** A slow fake `$INDEX_ALLOCATION:$I30` stream source returns a valid index plus the new caveat after the budget expires; existing stream-source caveats are preserved; cancellation continues to return `OperationCancelled`.
- **Verification:** Focused `windows_ntfs_mft` tests pass.

### U3. Document the caveat and refresh dogfood evidence

- **Goal:** Make the new degraded full-index mode visible in release notes and live evidence docs.
- **Requirements:** R2, R5
- **Files:** `CHANGELOG.md`, `docs/configuration.md`, `docs/performance/perf-matrix.md`, `docs/release.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Add concise Unreleased text and documentation that names `mft-index-allocation-budget-exhausted`, explains that targeted traversal remains the default path, and records the forced full-index dogfood evidence from this workstation.
- **Patterns to follow:** Prior NTFS caveat entries in `CHANGELOG.md`, `docs/configuration.md`, and the dogfood evidence wording from the 006 plan.
- **Test scenarios:** Documentation names the caveat consistently and does not imply deletion authority or raw-image mounting support.
- **Verification:** `git diff --check` passes.

## Verification Contract

```powershell
cargo nextest run -p rebecca-core scan::windows_ntfs_mft
cargo fmt --all --check
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest
git diff --check
```

When elevated live NTFS access is available, also run one forced full-index dogfood against `docs\plans` with `REBECCA_NTFS_MFT_FULL_INDEX_FALLBACK=1`, `REBECCA_NTFS_MFT_INDEX_TIMINGS=1`, and a 60 second budget. The run should keep `windows-ntfs-mft-experimental` evidence or report a remaining earlier-stage fallback with a clear reason.

## Definition of Done

- Late full-index index-allocation budget exhaustion preserves the parsed MFT index and emits `mft-index-allocation-budget-exhausted`.
- Cancellation remains an error, and earlier build-stage failures still fall back through the existing portable path.
- Focused Rust tests cover the degraded success path and cancellation boundary.
- Changelog and docs describe the caveat as diagnostic evidence only.
- Live dogfood evidence is collected or a host-specific reason for skipping it is recorded in the final summary.
