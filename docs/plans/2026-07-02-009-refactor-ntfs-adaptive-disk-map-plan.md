---
artifact_contract: "ce-unified-plan/v1"
artifact_readiness: "implementation-ready"
execution: "code"
product_contract_source: "ce-plan-bootstrap"
origin: "conversation"
created: "2026-07-02"
last_updated: "2026-07-02"
title: "NTFS Adaptive Disk Map Backend - Plan"
---

# NTFS Adaptive Disk Map Backend - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Refactor `inspect map --scan-backend windows-ntfs-mft-experimental` so ordinary directory roots use targeted NTFS traversal instead of full-volume indexing, while keeping full-volume MFT inventory as an explicit drive-root or diagnostic path. |
| User value | Users get WizTree-class source provenance and allocated-size metadata for small or medium NTFS roots without waiting for a whole-drive MFT index that can exceed practical live budgets. |
| Safety stance | Report-only. NTFS metadata remains inspection evidence, never deletion authority, and every timeout or traversal caveat must be visible. |
| Primary surfaces | `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`, `crates/rebecca-core/src/disk_map.rs`, CLI/API docs, NTFS dogfood tooling. |
| Stop conditions | Stop if a partial NTFS traversal is labeled exact, if cleanup estimates regress to full-volume indexing by default, if disk-map silently scans a different volume/root, or if restricted-license reference code is copied. |

## Product Contract

### Summary

The previous disk-map slice added a useful read-only CLI surface, but local elevated dogfood exposed the next architectural issue: explicit NTFS map mode currently builds a full-volume `MftIndex` even for a small root such as `docs/plans`. On the local E: drive, that exceeded the 20 second internal live metadata budget and also hit a 180 second no-budget script timeout.

The next slice should make the backend adaptive:

- Directory/file roots use targeted FSCTL record traversal and `$I30` expansion to compute totals and ranked entries for that root only.
- Drive roots keep the explicit full-volume inventory path because there is no smaller root scope.
- The CLI output must distinguish targeted map, full-volume map, and portable fallback through existing provenance fields.

### Requirements

- R1. `inspect map --scan-backend windows-ntfs-mft-experimental --root <non-drive-root>` must attempt a targeted NTFS disk-map path before full-volume inventory.
- R2. Targeted disk-map traversal must return the same metrics contract as portable disk-map: logical bytes, optional allocated bytes, file count, directory count, depth, kind, top entries, caveats, and provenance.
- R3. Targeted disk-map traversal must count each MFT record once, preserve hardlink/sequence/reparse caveats, and treat traversal budget exhaustion as fallback-capable rather than exact partial output.
- R4. Drive roots and explicit full-index diagnostic flows may still use full-volume `MftIndex`, but the source label must differ from targeted map output.
- R5. Unsupported, unelevated, stale-reference, timeout, or budget failures must fall back to portable recursive inventory with a diagnostic unless the error is cancellation or an unexpected internal failure.
- R6. Ordinary `clean`, `scan`, and `inspect space` behavior must remain targeted-first and must not regress to full-volume indexing without the existing diagnostic opt-in.
- R7. Dogfood must record whether `inspect-map` used targeted, sequential full-index, FSCTL-record full-index, or portable fallback.
- R8. Docs and changelog must state that NTFS disk-map is now adaptive and that full-volume inventory is reserved for drive-root or explicit diagnostic cases.

### Acceptance Examples

- AE1. Given an elevated NTFS host and `docs/plans`, when `inspect map --format json --scan-backend windows-ntfs-mft-experimental --root docs/plans --top 3` runs, then it should complete through a targeted NTFS backend source or fall back with an explicit targeted failure reason. It must not start a full-volume build just because the root is a directory.
- AE2. Given a targeted NTFS fixture with two large child directories, when the targeted map collector runs, then top entries are ranked by subtree logical bytes and include allocated bytes when all counted files provide them.
- AE3. Given a hardlinked or duplicate `$I30` child reference, when targeted map traversal runs, then the record is counted once and a bounded caveat explains the duplicate.
- AE4. Given a traversal budget breach, when `inspect map` runs through the experimental backend, then Rebecca falls back to portable recursive inventory and records the NTFS reason in `estimate_fallback_reason` and diagnostics.
- AE5. Given a drive root, when the experimental backend is selected, then Rebecca may use the full-volume map source and preserve existing full-index timeout semantics.
- AE6. Given `clean --dry-run --scan-backend windows-ntfs-mft-experimental`, when targeted traversal succeeds, then `estimate_backend_source` remains `windows-ntfs-mft-experimental-targeted-fsctl`.

## Planning Contract

### Key Technical Decisions

- KTD1. Promote targeted traversal from "summary only" to "summary plus optional top-entry collector".
  - Rationale: The existing targeted traversal already does the hard correctness work: direct record reads, attribute-list resolution, `$I30` expansion, sequence checks, reparse skipping, hardlink caveats, budgets, cancellation, and timing. Reusing it avoids a second NTFS traversal model.

- KTD2. Keep full-volume indexing only for drive-root inventory and explicit diagnostics.
  - Rationale: A directory map should not pay whole-drive cost. Full-volume indexing remains valuable for true volume-root questions and future persistent index work.

- KTD3. Use existing `DiskMapTopEntries` and `DiskMapBackendRoot` rather than inventing a parallel report shape.
  - Rationale: The public API is already correct. The refactor should improve backend selection, not create another contract.

- KTD4. Targeted NTFS map output is exact only if traversal completes within limits.
  - Rationale: A partial tree is dangerous for cleanup decisions. If the targeted traversal hits budget, stale root, unsupported metadata, or cancellation, the result must be fallback or error.

- KTD5. Do not add a persistent raw MFT cache in this slice.
  - Rationale: The current bottleneck is wrong strategy for scoped roots. Persistent full-volume caching is a separate design with invalidation, privacy, and corruption concerns.

### High-Level Design

```text
inspect map request
  -> backend selector
      -> portable recursive by default
      -> windows-ntfs-mft-experimental
          -> if root is a drive root: full-volume map path
          -> else: targeted FSCTL map path
                -> TargetedMftTraversal::collect_disk_map(root_ref, root_path, top_limit, max_depth)
                -> DiskMapBackendRoot
          -> fallback to portable recursive on fallback-capable NTFS errors
  -> existing renderers / API v1
```

The core change is to split `TargetedMftTraversal` traversal mechanics from the aggregation action:

- Keep `aggregate_subtree` as a thin wrapper for cleanup/inspect-space summaries.
- Add a targeted map collector that walks the same resolved records and pushes visible entries into `DiskMapTopEntries`.
- Preserve deterministic path construction from `$I30` names when traversing children. Root file paths use the requested path.
- Keep allocated byte aggregation optional with the existing "unknown if any counted file lacks allocated size" semantics.

### System-Wide Impact

| Area | Impact |
|---|---|
| `crates/rebecca-core/src/scan/windows_ntfs_mft.rs` | Add targeted disk-map collector, adaptive map source selection, source labels, and tests. |
| `crates/rebecca-core/src/disk_map.rs` | No public contract break expected; fallback diagnostics may include new source labels. |
| `crates/rebecca-core/tests/disk_map.rs` | Keep portable fallback regressions; add backend-selection test where possible through disabled-live path. |
| `scripts/ntfs/run-live-mft-dogfood.ps1` | Ensure reports distinguish targeted map source from full-index and fallback sources. |
| Docs / changelog | Document adaptive NTFS disk-map behavior and dogfood evidence. |

### Sources And Research

- `docs/plans/2026-07-02-007-refactor-ntfs-targeted-traversal-plan.md` established targeted FSCTL traversal as the normal live NTFS path.
- `docs/plans/2026-07-02-008-feat-ntfs-disk-map-plan.md` added the public disk-map surface and captured the full-volume timeout evidence.
- `docs/performance/perf-matrix.md` documents the 20 second internal timeout and 180 second no-budget timeout on the local E: drive.
- `repo-ref/go-ntfs`, `repo-ref/DiscUtils`, `repo-ref/gomft`, `repo-ref/python-ntfs`, `repo-ref/libfsntfs`, `repo-ref/ntfs-3g`, and `repo-ref/sleuthkit` remain behavior and boundary references only. Restricted-license code is not copied.

### Risks And Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Targeted map and targeted summary diverge. | Inconsistent totals between `inspect space` and `inspect map`. | Implement both through one traversal core or shared helper tests. |
| Path reconstruction from `$I30` entries is wrong. | Misleading ranked output. | Use requested root for root path, child `$I30` names for traversal paths, sequence checks, and caveats for parent-map fallback. |
| Full-volume map disappears for actual drive-root use. | Worse volume-level insight. | Gate full-index path on drive-root detection instead of removing it. |
| Budget fallback hides NTFS failure. | User cannot diagnose why targeted map was not used. | Preserve NTFS error text in fallback diagnostics and dogfood reports. |
| Tests become live-host dependent. | CI flakiness. | Keep unit tests fixture-backed with fake resolvers; live dogfood remains local evidence. |

## Implementation Units

### U1. Add Targeted Disk-Map Collector

- **Goal:** Extend `TargetedMftTraversal` so it can collect `DiskMapBackendRoot` data, not only `SubtreeSummary`.
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add a targeted traversal result builder that tracks metrics, caveats, visited records, current path/depth, and a bounded `DiskMapTopEntries`. Reuse record resolution, `$I30` child expansion, parent/sequence caveat helpers, allocated-byte aggregation, and cancellation checks.
- **Test scenarios:** directory with nested `$I30` child produces root totals and top child entry; file root produces one file entry at depth 0; max-depth hides deeper entries without changing totals; duplicate record is counted once with caveat.
- **Verification:** `cargo nextest run -p rebecca-core windows_ntfs_mft::tests`

### U2. Make Experimental Disk-Map Backend Adaptive

- **Goal:** Route non-drive roots through targeted disk-map traversal and drive roots through existing full-volume map.
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add `build_targeted_mft_disk_map` beside `build_targeted_mft_summary`. In `inspect_disk_map`, resolve volume and root identity once, detect drive root, and choose targeted or full-index path. Use source label `windows-ntfs-mft-experimental-targeted-fsctl` for targeted map output.
- **Test scenarios:** non-drive map source is targeted in unit-level fake traversal; full-index helper remains reachable for drive roots; fallback-capable targeted errors still allow portable fallback through existing `disk_map` adapter.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map windows_ntfs_mft::tests`

### U3. Preserve Contract And Fallback Semantics

- **Goal:** Ensure no partial NTFS result is labeled exact and existing cleanup scan behavior is unchanged.
- **Files:** `crates/rebecca-core/src/disk_map.rs`, `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`, tests.
- **Approach:** Keep `inspect_map` fallback policy unchanged for `PlatformUnavailable`, `ScanFailed`, and `SafetyBlocked`. Add tests for budget errors and source labels. Re-run CLI/API tests to prove payload shape stays stable.
- **Test scenarios:** targeted traversal budget error falls back to portable inventory; `inspect space` experimental source still uses targeted summary; `inspect map` JSON schema remains valid.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map`; `cargo nextest run -p rebecca --test cli_inspect --test cli_api`

### U4. Update Dogfood Tooling And Docs

- **Goal:** Make adaptive behavior visible to humans and release checks.
- **Files:** `scripts/ntfs/run-live-mft-dogfood.ps1`, `docs/performance/perf-matrix.md`, `docs/configuration.md`, `docs/release.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`, `CHANGELOG.md`
- **Approach:** Document source label expectations: targeted for scoped roots, sequential/FSCTL-record for drive-root full-volume map, portable fallback when unsupported or budgeted out. Update dogfood notes with the before/after expectation.
- **Verification:** `pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -SelfTest`

### U5. Full Verification And Commit

- **Goal:** Ship the adaptive disk-map backend as one coherent refactor commit.
- **Verification:**
  - `cargo fmt --all --check`
  - `cargo check --workspace`
  - `cargo nextest run -p rebecca-core windows_ntfs_mft::tests`
  - `cargo nextest run -p rebecca-core --test disk_map`
  - `cargo nextest run -p rebecca --test cli_inspect --test cli_api`
  - `cargo nextest run --workspace`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo check -p rebecca-core --benches`
  - `pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -SelfTest`
  - Elevated dogfood when available: `REBECCA_NTFS_MFT_INDEX_TIMINGS=1 pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -Root docs/plans -Mode inspect-map -Backend windows-ntfs-mft-experimental -Top 3 -TimeoutSeconds 60`
  - `git diff --check`

## Definition Of Done

| ID | Done Condition |
|---|---|
| DoD1 | Scoped `inspect map --scan-backend windows-ntfs-mft-experimental` attempts targeted NTFS traversal and does not build a full-volume index first. |
| DoD2 | Drive-root disk-map still has a full-volume path with explicit source provenance and existing timeout semantics. |
| DoD3 | Targeted disk-map totals, top entries, allocated bytes, depth filtering, caveats, and source labels are tested. |
| DoD4 | Fallback diagnostics remain visible and no partial NTFS traversal is reported as exact. |
| DoD5 | Docs, changelog, current-state memory, and dogfood guidance describe the adaptive backend. |
| DoD6 | Focused tests, full workspace tests, clippy, bench checks, dogfood self-test, and diff checks pass or any host-specific limitation is documented. |
