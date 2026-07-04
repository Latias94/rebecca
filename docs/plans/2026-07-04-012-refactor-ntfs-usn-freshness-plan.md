# Refactor NTFS Volume Cache USN Freshness Plan

## Context

Rebecca can now persist full NTFS MFT volume-index payloads beside their manifests, but those payloads are only structurally validated. That is not enough for a cleanup CLI: a correct cache hit must prove the indexed volume has not changed since the payload was written, or it must rebuild conservatively.

This plan makes persistent volume-index reuse safe enough to wire into the explicit manifest-store path. It intentionally favors rebuilding over stale reads.

## Priorities

1. Capture live USN journal state at the NTFS volume boundary.
2. Persist a stable USN checkpoint only when the journal is unchanged across the full-index build.
3. Add a freshness-aware payload lookup that rejects payloads without a checkpoint or with an advanced/changed journal.
4. Enable explicit persistent payload hits only after structural validation and USN freshness validation both pass.
5. Document the remaining gap: this is exact unchanged-volume reuse, not USN change replay or incremental refresh.

## Work Units

### 1. Live USN Checkpoint Capture

- Add a read-only `FSCTL_QUERY_USN_JOURNAL` query to `LiveNtfsVolume`.
- Convert the live journal data into `ScanCacheUsnJournalState`.
- Sample the journal before and after full-index build.
- Store a `ScanCacheUsnCheckpoint` only when both samples have the same journal id and `next_usn`.

### 2. Fresh Payload Lookup

- Keep `load_index_payload` as a structural validator.
- Add a stricter freshness path that also validates the manifest USN checkpoint against current journal state.
- Treat missing checkpoint, journal id changes, range loss, or advanced `next_usn` as cache misses.
- Prune stale pairs when the persistent payload can never be a safe hit again.

### 3. Explicit Persistent Hit Wiring

- In `WindowsNtfsMftIndexCache::load_or_build`, try the explicit manifest store before rebuilding.
- Recompute the volume fingerprint from live NTFS volume data before loading any payload.
- Query the live USN journal and only return the cached `MftIndex` if the payload passes structural and freshness validation.
- Leave default scans unchanged when no manifest store is configured.

### 4. Tests And Docs

- Add unit tests for stable checkpoint detection and freshness miss reasons.
- Add manifest-store tests for payloads with and without USN checkpoints.
- Update `CHANGELOG.md` under Unreleased.
- Run focused NTFS tests, then the workspace test suite with nextest if available.

## Non-Goals

- No USN change-record replay in this slice.
- No incremental MFT patching.
- No default persistent cache root.
- No compatibility shim for old checkpoint-free payloads; they are allowed to miss and be replaced.

## Success Criteria

- Persistent payload hits are possible only when the volume fingerprint matches and the USN journal proves no volume changes occurred since build.
- Checkpoint-free or stale payloads rebuild instead of being used.
- Existing default behavior remains unchanged unless a manifest store is configured.
- Tests cover both the accepted unchanged-volume case and conservative stale cases.
