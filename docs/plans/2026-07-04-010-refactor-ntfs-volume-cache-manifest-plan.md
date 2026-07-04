---
title: "NTFS Volume Cache Manifest - Plan"
type: "refactor"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Volume Cache Manifest - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Add a versioned NTFS volume-index cache manifest model and store so future persistent MFT index reuse has a safe on-disk contract. |
| Authority | Builds on `docs/plans/2026-07-04-009-refactor-ntfs-volume-cache-boundary-plan.md`, which introduced typed volume identity and stable volume fingerprint generation. |
| Execution profile | Internal Rust refactor with deterministic tests and documentation; default scans keep the existing in-process cache behavior. |
| Stop condition | Stop if implementation needs to serialize full `MftIndex`, enable cross-process reuse by default, add live IO on cache hits, or widen cleanup authority. |
| Landing strategy | One conventional commit after focused NTFS tests, workspace check, clippy, workspace nextest, and diff hygiene pass. |

---

## Product Contract

Rebecca now knows what makes an NTFS volume-index cache subject safe to identify: device path, volume serial, record geometry, `$MFT` location, `$MFTMirr` location, and `$MFT` valid length.
The next step is to turn that in-memory fingerprint into a persistent manifest contract that can survive process exit without implying the actual MFT index is reusable yet.

This plan adds the manifest schema, stable file naming, load/store validation, and future USN checkpoint field.
It deliberately does not write or read full MFT index payloads.
The manifest is rebuildable optimization metadata under the cache lifecycle, not deletion evidence.

### Requirements

**Manifest schema**

- R1. A manifest records schema version, cache fingerprint generation, volume identity, NTFS geometry, MFT boundary fields, created time, last build source, and optional USN checkpoint.
- R2. Unknown or future manifest versions are rejected instead of reused.
- R3. Manifest validation requires exact fingerprint equality before any future USN validation can be considered.

**Manifest store**

- R4. The store derives files from the volume fingerprint generation under a dedicated NTFS cache subdirectory using stable lowercase hex names.
- R5. Writes use the same atomic replace pattern as scan-cache records and clean up temp files on failure.
- R6. Missing, unreadable, corrupted, stale, or wrong-generation manifests are treated as cache misses, not fatal scan errors.

**Runtime boundary**

- R7. `ScanEngine::new()` keeps current behavior and does not write manifest files.
- R8. Callers can construct a `ScanEngine` with an explicit NTFS manifest cache root for future runtime wiring.
- R9. Successful live NTFS index builds can write a manifest when a store is configured, but current cache hits must not perform extra live IO.

**Safety and docs**

- R10. Documentation and changelog must state that this is manifest groundwork only; cross-process MFT index payload reuse remains deferred.
- R11. Tests cover round-trip, validation rejection, corrupt/missing fallback, and non-default runtime wiring.

---

## Planning Contract

- KTD1. Keep manifest storage separate from full index payloads. A manifest is small JSON metadata and can be validated cheaply; an MFT index payload needs a separate format, privacy review, and USN freshness gate.
- KTD2. Make the store opt-in through an explicit `ScanEngine` constructor for this slice. The current CLI constructs `ScanEngine::new()` in many places, and silently resolving app paths inside the scan layer would make scan construction fallible and harder to reason about.
- KTD3. Reuse scan-cache's rebuildable cache semantics. Corrupt or stale metadata should degrade to a miss so the scanner can rebuild evidence.
- KTD4. Store optional USN checkpoint in the manifest now, but do not require it until live USN capture exists. This keeps the JSON shape stable for the next slice without pretending freshness can already be proven.

---

## Implementation Units

### U1. Add the manifest schema and validation model

- **Goal:** Define the persistent JSON contract for NTFS volume-index cache metadata.
- **Requirements:** R1, R2, R3, R10
- **Dependencies:** None
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Make the existing volume cache key and fingerprint serializable, add `NtfsVolumeIndexCacheManifest` with versioned fields, and expose validation that returns a small miss reason instead of panicking or treating stale manifests as usable.
- **Patterns to follow:** `ScanCacheRecord`, `ScanCacheUsnCheckpoint`, and the current `NtfsVolumeIndexFingerprint` tests.
- **Test scenarios:** A manifest round-trips through JSON; matching fingerprint validates; future version rejects; geometry, identity, MFT LCN, or MFT length mismatch rejects.
- **Verification:** Focused NTFS tests pass.

### U2. Add an opt-in manifest store

- **Goal:** Persist and load manifests from a dedicated NTFS cache directory without affecting default scans.
- **Requirements:** R4, R5, R6, R11
- **Dependencies:** U1
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add `NtfsVolumeIndexManifestStore` with `new`, `cache_file_for`, `load`, and `store`. Use lowercase hex generation file names, JSON serialization, parent directory creation, temp-file write, and replace semantics mirroring scan-cache.
- **Patterns to follow:** `ScanCacheStore::cache_file_for`, `store_measured_scan_with_policy`, `write_cache_file`, and scan-cache miss tests.
- **Test scenarios:** Store path uses `ntfs-volume-index/<generation>.json`; missing files return miss; corrupt JSON returns miss and prunes the file; stale generation returns miss; successful store/load round-trips the manifest.
- **Verification:** Focused NTFS tests pass.

### U3. Wire the store behind an explicit `ScanEngine` constructor

- **Goal:** Allow future CLI/runtime code to provide a cache root without changing `ScanEngine::new()` semantics.
- **Requirements:** R7, R8, R9, R11
- **Dependencies:** U1, U2
- **Files:** `crates/rebecca-core/src/scan.rs`, `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add an optional store to `WindowsNtfsMftIndexCache`, a `with_manifest_store` constructor, and `ScanEngine::with_ntfs_mft_manifest_cache_root`. When configured, successful builds write a manifest as best-effort caveat-free metadata; load decisions remain future work until payload serialization exists.
- **Patterns to follow:** Existing `WindowsNtfsMftIndexCache::default`, cacheable failure behavior, and `ScanEngine` constructor shape.
- **Test scenarios:** Default cache has no manifest store; explicitly configured engine creates a cache context with a store; manifest write failure is fallback-safe and does not change scan results.
- **Verification:** Focused NTFS tests and workspace check pass.

### U4. Refresh docs and engineering memory

- **Goal:** Keep the cache roadmap clear.
- **Requirements:** R10
- **Dependencies:** U1, U2, U3
- **Files:** `CHANGELOG.md`, `docs/performance/perf-matrix.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Add concise wording that names the manifest store and repeats that full MFT index serialization plus USN freshness validation remain deferred.
- **Patterns to follow:** Recent NTFS cache-boundary and scan-evidence entries.
- **Test scenarios:** Wording does not imply default persistent cache hits or cleanup authority.
- **Verification:** `git diff --check`.

---

## Verification Contract

| Gate | Command | Proves |
|---|---|---|
| Format | `cargo fmt --all --check` | Rust formatting stayed stable. |
| NTFS focused tests | `cargo nextest run -p rebecca-core scan::windows_ntfs_mft` | Manifest schema, store behavior, and existing NTFS behavior pass. |
| Workspace check | `cargo check --workspace` | Cross-crate compilation remains sound. |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Rust lint baseline remains clean. |
| Workspace tests | `cargo nextest run --workspace` | Existing CLI/core/parser contracts still pass. |
| Diff hygiene | `git diff --check` | No whitespace errors. |

---

## Definition of Done

- NTFS volume-index manifests have a versioned JSON schema with stable fingerprint validation.
- A dedicated manifest store can load, store, miss, and prune manifests deterministically.
- Default scanning remains unchanged unless a caller explicitly constructs `ScanEngine` with a manifest cache root.
- Successful configured live builds write manifest metadata without changing cleanup estimates, caveats, or deletion authority.
- Tests cover manifest round-trip, mismatch rejection, corrupt/missing fallback, and explicit store wiring.
- Changelog, performance docs, and engineering memory describe the manifest as groundwork for later index payload and USN freshness work.
