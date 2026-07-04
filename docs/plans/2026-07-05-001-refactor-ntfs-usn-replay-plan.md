---
title: "NTFS USN Replay Freshness - Plan"
type: refactor
date: 2026-07-05
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS USN Replay Freshness - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Let explicit persistent NTFS/MFT volume-index payloads survive unrelated USN journal activity by replaying live USN records against the requested target subtree before reuse. |
| Authority | Preserve cleanup safety and exactness first; prefer rebuilding over using stale MFT data. |
| Stop conditions | Stop if replay cannot prove target-subtree cleanliness, if the USN range is unavailable, or if implementation would make NTFS/MFT a default cleanup dependency. |
| Execution profile | Code implementation in `crates/rebecca-core/src/scan/windows_ntfs_mft.rs` with focused NTFS cache tests and full workspace nextest. |

---

## Product Contract

### Summary

The current persistent NTFS/MFT payload cache only hits when the volume USN `next_usn` is unchanged.
That is safe but too strict: unrelated changes elsewhere on the volume force a full MFT rebuild.
This plan upgrades the freshness gate so a payload can be reused when the live USN range is readable and no change record touches the requested target subtree.

### Requirements

**Freshness behavior**

- R1. A persistent MFT payload may be reused when the volume fingerprint matches, the journal id matches, the recorded range is readable, and replayed USN changes do not touch the requested target subtree.
- R2. A persistent MFT payload must miss and rebuild when the journal changed, the requested range is no longer readable, the target file id is unavailable, or any replayed change touches the target subtree.
- R3. A checkpoint-free payload remains unusable for persistent reads.

**Safety and scope**

- R4. USN replay is a read-only validation step; it does not patch or mutate the stored `MftIndex`.
- R5. Root-volume queries remain conservative because any volume change is inside the root subtree.
- R6. NTFS/MFT estimates remain measurement-only and never become deletion authority.

### Scope Boundaries

- No incremental MFT payload mutation.
- No default persistent cache root.
- No cross-crate API surface unless needed for testability.
- No support for USN V3 128-bit file ids in this slice; unsupported records force conservative miss.

---

## Planning Contract

### Key Technical Decisions

- KTD1. Pass target identity into the full-index cache boundary.
  `load_or_build` currently knows only the volume, but target-aware replay needs the target record id.
  The callers already resolve `FileIdentity`, so this is an internal API refactor rather than new IO.
- KTD2. Keep structural payload validation separate from freshness validation.
  `load_index_payload` should still prove the file pair is well-formed; a new replay-aware path should decide whether it is authoritative for a target.
- KTD3. Use the persisted `MftIndex` only as ancestry evidence.
  USN records provide file id and parent id; the cached index can walk known parent chains to determine whether a changed record belonged to the requested subtree at cache time.
  If the chain is missing or ambiguous, rebuild.
- KTD4. Treat journal advance without readable changes as stale.
  Exactness depends on seeing every record from the checkpoint through current `next_usn`.
- KTD5. Re-check the USN journal after replay before accepting a hit.
  If the journal keeps advancing during replay, retry a bounded number of times and then rebuild instead of serving an index proven only against an older observation.

### System-Wide Impact

This change affects the experimental NTFS/MFT fallback and inspect-map paths that explicitly configure a manifest store.
Default no-store scans keep their existing in-memory cache behavior.
The implementation should improve repeated cross-process diagnostic reuse without changing cleanup deletion semantics.

---

## Implementation Units

### U1. Add live USN range reader

- **Goal:** Read bounded USN V2 records from `FSCTL_READ_USN_JOURNAL` and convert them into `ScanCacheUsnChange`.
- **Requirements:** R1, R2, R4
- **Dependencies:** None
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add a live volume helper that starts at the manifest checkpoint `next_usn`, reads chunks until it reaches current `next_usn` or EOF, parses the output buffer header and USN V2 records, and returns range-unavailable on malformed, unsupported, or over-budget output.
- **Patterns to follow:** Existing `DeviceIoControl` helpers in `LiveNtfsVolume`; existing `ScanCacheUsnChange` model in `crates/rebecca-core/src/scan_cache.rs`.
- **Test scenarios:** Valid synthetic USN buffer parses one or more V2 records; unsupported major versions are rejected; truncated buffers are rejected; negative USNs are rejected.
- **Verification:** Focused tests prove parser behavior without requiring a live NTFS volume.

### U2. Validate target subtree cleanliness

- **Goal:** Decide whether replayed changes touch the requested target subtree using cached `MftIndex` ancestry.
- **Requirements:** R1, R2, R5
- **Dependencies:** U1
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add a helper that enriches each USN change with ancestor ids by walking parent references from the cached `MftIndex`.
  Return clean only when every change can be classified and none touches the target.
  Treat target root queries as changed when any replayed change exists.
- **Patterns to follow:** `ScanCacheUsnCheckpoint::validate_journal_range`; `MftIndex::get`; existing target record id checks in the backend.
- **Test scenarios:** Unrelated sibling changes stay clean; direct target change invalidates; descendant change invalidates via ancestors; missing parent chain invalidates; volume-root target invalidates on any change.
- **Verification:** Unit tests cover clean and conservative miss paths.

### U3. Wire replay-aware persistent payload hits

- **Goal:** Use USN replay before accepting a persisted `MftIndex` for a specific target.
- **Requirements:** R1, R2, R3, R4
- **Dependencies:** U1, U2
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Change `WindowsNtfsMftIndexCache::load_or_build` and callers to include target record id and whether the target is the volume root.
  Keep the existing unchanged-`next_usn` fast path.
  When `next_usn` advanced, load the payload structurally, read USN records, validate target cleanliness, and only then return the persistent cache hit.
  Re-query the journal after replay so concurrent journal writes cannot slip between validation and cache reuse.
- **Patterns to follow:** Existing manifest/payload lookup and stale-prune behavior.
- **Test scenarios:** A structurally valid payload with unrelated changes hits; target-touching changes miss; unreadable range misses; no checkpoint misses.
- **Verification:** Focused `ntfs_volume_index` tests prove replay-aware lookup, followed by full workspace nextest.

### U4. Update release notes

- **Goal:** Document the behavior change in the Unreleased changelog.
- **Requirements:** R1, R2
- **Dependencies:** U3
- **Files:** `CHANGELOG.md`
- **Approach:** Replace the unchanged-volume-only wording with target-aware USN replay wording.
- **Test scenarios:** Test expectation: none -- documentation-only.
- **Verification:** Diff review confirms the changelog is under Unreleased.

---

## Verification Contract

| Gate | Command | Done signal |
|---|---|---|
| Formatting | `cargo fmt` | No formatting diff remains. |
| Focused NTFS cache tests | `cargo nextest run -p rebecca-core ntfs_volume_index` | All focused tests pass. |
| Workspace regression | `cargo nextest run --workspace` | All workspace tests pass. |

---

## Definition of Done

- U1-U4 are implemented without leaving old compatibility shims or unused code.
- Persistent payload reuse is target-aware and conservative on uncertainty.
- Existing default scan behavior remains unchanged without a manifest store.
- Changelog records the new replay-aware freshness gate.
- Verification Contract gates pass.
