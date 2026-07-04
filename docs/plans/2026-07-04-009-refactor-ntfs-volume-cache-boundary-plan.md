---
title: "NTFS Volume Cache Boundary - Plan"
type: "refactor"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Volume Cache Boundary - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Make the live NTFS/MFT volume index cache use an explicit volume identity and persistent-reuse fingerprint so future USN-backed disk cache work starts from a safe boundary. |
| Authority | Follows `docs/plans/2026-07-04-008-perf-ntfs-scan-evidence-plan.md`, which made scan cost visible and left persistent USN/volume-index caching as the next design step. |
| Execution profile | Focused internal Rust refactor plus tests and docs; no cleanup authorization, deletion, or default backend behavior changes. |
| Stop condition | Stop if the slice requires serializing full MFT indexes, adding a user-visible cache switch, weakening fallback behavior, or reading extra live volume data on cache hits. |
| Landing strategy | One conventional commit after focused NTFS tests, formatting, clippy, workspace check, and diff hygiene pass. |

---

## Product Contract

Rebecca already has two cache lines:

- `scan_cache` persists ordinary path estimates with optional USN checkpoint validation.
- `WindowsNtfsMftIndexCache` keeps a per-process MFT volume index behind a string key of `device_path:volume_serial`.

The next performance leap is a persistent NTFS volume index cache, but unsafe reuse would be worse than a miss.
The first refactor must make the cache identity explicit and define the volume fingerprint that future persisted entries must match before reuse.
This slice does not persist full MFT indexes yet.
It codifies the reusable boundary and keeps the existing in-memory single-flight behavior intact.

### Requirements

**Cache identity**

- R1. The live NTFS/MFT index cache uses a typed volume identity key instead of an ad hoc string.
- R2. The identity key is based on the resolved NTFS device path and volume serial number.
- R3. The key remains internal and does not change CLI output or scan results.

**Persistent reuse boundary**

- R4. Successful index builds capture a volume fingerprint containing identity, record size, sector size, cluster size, `$MFT` LCN, `$MFTMirr` LCN, and `$MFT` valid data length.
- R5. The fingerprint exposes a stable generation value for future persistent cache manifests without using process-randomized Rust hash state.
- R6. A cached in-memory index is returned only when its fingerprint still matches the requested volume identity and contains a complete persistent boundary.

**Safety and documentation**

- R7. The refactor must not add live IO on cache hits, persist private path data, or change fallback semantics.
- R8. Tests must prove identity discrimination and fingerprint generation changes for geometry, MFT location, and MFT length changes.
- R9. Changelog and engineering memory must describe this as cache-boundary groundwork, not a finished persistent USN cache.

---

## Planning Contract

- KTD1. Keep cache-hit lookup cheap. Opening the volume to recompute geometry before every in-memory hit would erase the current single-flight benefit.
- KTD2. Treat geometry and MFT location as part of the persistent boundary even though in-memory reuse is keyed by identity today. Persistent entries need stricter proof because they can outlive the process.
- KTD3. Use a simple stable generation over canonical fields. This is a deterministic discriminator, not a security hash or public identifier.
- KTD4. Keep the future USN layer separate from this refactor. The generation says whether the volume layout still looks like the same cache subject; USN journal state will decide whether file changes invalidate an otherwise matching subject.

---

## Implementation Units

### U1. Replace string volume cache keys with typed identity

- **Goal:** Make the in-memory NTFS/MFT cache key explicit and less fragile.
- **Requirements:** R1, R2, R3
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add `NtfsVolumeIndexCacheKey`, switch `WindowsNtfsMftIndexCache` to `BTreeMap<NtfsVolumeIndexCacheKey, CachedNtfsVolumeIndexSlot>`, and have `NtfsVolumeCapabilities::cache_key` return the typed key.
- **Patterns to follow:** Existing `NtfsVolumeCapabilities`, `WindowsNtfsMftIndexCache::load_or_build`, and volume path tests.
- **Test scenarios:** Same device path and serial produce the same key; changing device path or serial produces a different key.
- **Verification:** `cargo nextest run -p rebecca-core scan::windows_ntfs_mft`.

### U2. Capture a stable volume-index fingerprint

- **Goal:** Define the future persistent reuse boundary in code.
- **Requirements:** R4, R5, R6, R7, R8
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add `NtfsVolumeIndexFingerprint` built from capabilities, validated NTFS record geometry, and `NTFS_VOLUME_DATA_BUFFER`. Compute a stable FNV-1a generation over schema version, identity, geometry, MFT LCNs, and MFT length. Store the fingerprint in `CachedNtfsVolumeIndex`, and have cache-hit reuse call a small `is_reusable_for` guard.
- **Patterns to follow:** `NtfsRecordGeometry::from_volume_data`, `mft_mirror_read_plan`, and current focused tests around NTFS volume data.
- **Test scenarios:** Generation is stable for identical fingerprints; it changes when record geometry changes; it changes when `$MFT`/`$MFTMirr` LCNs change; it changes when `$MFT` valid length changes; the in-memory reuse guard rejects a different identity.
- **Verification:** `cargo nextest run -p rebecca-core scan::windows_ntfs_mft`.

### U3. Refresh docs and memory

- **Goal:** Keep the roadmap accurate for the next persistent USN cache slice.
- **Requirements:** R7, R9
- **Files:** `CHANGELOG.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Add concise notes that typed volume identity and stable fingerprints are now in place, while index serialization and USN journal checkpoints remain deferred.
- **Patterns to follow:** Recent NTFS diagnostic and dogfood entries.
- **Test scenarios:** Wording does not imply a finished cross-process cache or changed cleanup authority.
- **Verification:** `git diff --check`.

---

## Verification Contract

| Gate | Command | Proves |
|---|---|---|
| Format | `cargo fmt --all --check` | Rust formatting stayed stable. |
| NTFS focused tests | `cargo nextest run -p rebecca-core scan::windows_ntfs_mft` | Typed cache key, fingerprint generation, and existing NTFS behavior still pass. |
| Workspace check | `cargo check --workspace` | Cross-crate compilation remains sound. |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Rust lint baseline remains clean. |
| Diff hygiene | `git diff --check` | No whitespace errors. |

---

## Definition of Done

- The in-memory NTFS/MFT cache no longer uses a string key internally.
- Successful NTFS/MFT index builds carry a stable volume-index fingerprint suitable for a future persistent manifest.
- Cache-hit reuse verifies the cached fingerprint still matches the requested volume identity and has a complete persistent boundary.
- Tests cover identity discrimination and fingerprint generation changes across geometry, MFT location, and MFT length.
- Changelog and engineering memory frame this as persistent USN cache groundwork, not a finished disk cache.
