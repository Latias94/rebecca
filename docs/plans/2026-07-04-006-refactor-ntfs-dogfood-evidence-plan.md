---
title: "NTFS Dogfood Mirror Evidence Report - Plan"
type: "refactor"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Dogfood Mirror Evidence Report - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Make inspect-map dogfood reports expose NTFS full-index and `$MFTMirr` evidence directly instead of forcing maintainers to inspect raw JSON payloads. |
| Authority | Builds on `docs/plans/2026-07-04-004-refactor-ntfs-raw-mirror-recovery-plan.md` and `docs/plans/2026-07-04-005-refactor-ntfs-live-mirror-source-plan.md`. |
| Execution profile | Small PowerShell report-schema and documentation change with parser/report self-tests. |
| Stop condition | Stop for any change that alters real scan behavior, treats mirror recovery as cleanup authority, or makes dogfood output depend on live NTFS access during self-test. |
| Landing strategy | One conventional commit after script self-test and diff checks pass. |

## Product Contract

Sequential full-index NTFS/MFT reads can now supply bounded `$MFTMirr` bytes to
the parser, but the dogfood report only exposed a raw caveat count. That is too
weak for release evidence: maintainers need to see whether a run used targeted
FSCTL, sequential full-index, FSCTL-record full-index, and whether mirror
recovery was used or unavailable.

This plan upgrades the dogfood evidence schema while keeping the CLI payload
unchanged. The report should preserve raw caveats and add derived, stable fields
that are easy to query from JSON, CSV, and Markdown.

## Requirements

- R1. Run summaries include a normalized backend source kind for targeted FSCTL, sequential full-index, and FSCTL-record full-index sources.
- R2. Run summaries include grouped caveat code counts derived from structured `estimate_caveats`.
- R3. NTFS mirror evidence includes separate `mft-mirror-record-used` and `mft-mirror-read-failed` counts plus a compact summary string.
- R4. CSV and Markdown reports expose the same evidence without requiring raw stdout JSON inspection.
- R5. `-SelfTest` covers caveat-code parsing, mirror evidence derivation, CSV projection, and Markdown projection without invoking Cargo or live NTFS APIs.

## Implementation Units

### U1. Add report-level NTFS evidence fields

- **Files:** `scripts/dogfood/run-inspect-map-report.ps1`
- **Approach:** Parse `estimate_caveats` values into stable caveat codes, group counts by code, derive `backend_source_kind`, `ntfs_full_index_source`, `ntfs_mirror_record_used_count`, `ntfs_mirror_read_failed_count`, and `ntfs_mirror_evidence`, and include them in JSON run summaries.
- **Test scenarios:** Structured caveat objects, JSON-string caveat objects from recursive extraction, full-index sequential backend source, and mixed mirror-used/read-failed caveats.
- **Verification:** `pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest`.

### U2. Project evidence into CSV, Markdown, and docs

- **Files:** `scripts/dogfood/run-inspect-map-report.ps1`, `scripts/dogfood/README.md`, `docs/performance/perf-matrix.md`, `docs/release.md`, `CHANGELOG.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Add flattened CSV columns, a Markdown backend-evidence section, and documentation describing caveat-code counts and mirror evidence as read-only diagnostics.
- **Verification:** `git diff --check`.

## Verification Contract

```powershell
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest
git diff --check
```

## Definition of Done

- Inspect-map dogfood JSON runs expose caveat-code counts and NTFS mirror/full-index evidence fields.
- Runs CSV contains flattened caveat-code and mirror evidence columns.
- Markdown summary has a backend evidence table.
- Docs and engineering memory name the new evidence fields and keep mirror recovery out of cleanup authority.
