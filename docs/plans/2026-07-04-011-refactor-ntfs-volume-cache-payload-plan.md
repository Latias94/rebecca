---
title: "NTFS Volume Cache Payload - Plan"
type: "refactor"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Volume Cache Payload - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Add a versioned persistent NTFS volume-index payload contract and pair it with the existing manifest store. |
| Authority | Follows `docs/plans/2026-07-04-010-refactor-ntfs-volume-cache-manifest-plan.md`, which added the versioned manifest and opt-in store. |
| Execution profile | Internal Rust refactor with deterministic tests and documentation. |
| Stop condition | Stop before default cross-process index reuse, live USN journal reads, cleanup authority changes, or a custom binary payload format. |
| Landing strategy | One conventional commit after focused NTFS tests, workspace check, clippy, workspace nextest, and diff hygiene pass. |

---

## Product Contract

Rebecca can now identify a persistent NTFS volume-index subject and write a manifest for it.
The missing cache lifecycle piece is the actual index payload boundary: a small schema that can persist the `MftIndex` and its shared caveats, prove it belongs to the manifest fingerprint, and degrade to a rebuildable miss when anything is absent or stale.

This plan writes and validates payload files, but it does not use them for default cache hits.
That keeps stale persistent indexes out of cleanup estimates until live USN checkpoint capture and validation exist.

### Requirements

**Payload schema**

- R1. A payload records schema version, fingerprint generation, full fingerprint, build source, shared caveats, and serialized `MftIndex`.
- R2. Payload validation rejects future versions, wrong generation, wrong fingerprint, malformed JSON, and checksum mismatch.
- R3. Manifest metadata records the payload file identity, payload schema version, byte length, and checksum.

**Store behavior**

- R4. The manifest store writes payload files under the existing `ntfs-volume-index/` directory with deterministic generation-based names.
- R5. Payload files are written before manifests so a committed manifest never intentionally points at an unwritten payload.
- R6. Missing, unreadable, corrupted, stale, or checksum-mismatched payloads are misses and may prune rebuildable cache files.
- R7. Existing manifest-only records remain metadata misses for payload loading rather than fatal errors.

**Runtime boundary**

- R8. `ScanEngine::new()` remains unchanged and does not write persistent NTFS cache files.
- R9. The explicit manifest-cache constructor writes both manifest and payload after successful configured live full-index builds.
- R10. No production path reads a persistent payload as authoritative cleanup evidence until USN freshness validation is implemented.

**Docs**

- R11. Changelog, performance docs, and engineering memory must state that payload persistence exists but default cross-process reuse is still gated by USN freshness work.

---

## Planning Contract

- KTD1. Use JSON for the first payload contract because `rebecca-ntfs::MftIndex` and its child types already derive serde. A custom binary format can follow after correctness and freshness are proven.
- KTD2. Keep manifest and payload schemas separately versioned. The manifest is cache metadata; the payload is the large rebuildable index body.
- KTD3. Use payload checksum and byte length as corruption guards, not security claims. This is a local rebuildable cache, so corruption degrades to a miss and rebuild.
- KTD4. Do not enable persistent payload reads for scan results in this slice. The next slice should capture live USN checkpoints and validate the journal range before any disk payload hit becomes eligible.

---

## Implementation Units

### U1. Add payload schema and validation

- **Goal:** Define the versioned JSON payload that can carry a complete `MftIndex`.
- **Requirements:** R1, R2, R3
- **Dependencies:** None
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add payload constants, `NtfsVolumeIndexPayloadRef`, `NtfsVolumeIndexPayload`, checksum helpers, and validation methods. Keep fingerprint equality as the primary reuse boundary.
- **Patterns to follow:** `NtfsVolumeIndexCacheManifest`, `NtfsVolumeIndexFingerprint`, and serde-enabled `rebecca-ntfs::MftIndex`.
- **Test scenarios:** Payload round-trips through JSON with a fixture `MftIndex`; matching fingerprint validates; future version, stale generation, stale fingerprint, and checksum mismatch miss.
- **Verification:** `cargo nextest run -p rebecca-core scan::windows_ntfs_mft`.

### U2. Pair payload files with manifests

- **Goal:** Make the manifest store write, load, and prune manifest/payload pairs deterministically.
- **Requirements:** R3, R4, R5, R6, R7
- **Dependencies:** U1
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add `payload_file_for`, `load_index_payload`, and `store_index_payload`. Store payload JSON first, compute metadata from the actual bytes, then write the manifest with a payload reference. Treat manifest-only records as misses that can be upgraded by the next successful build.
- **Patterns to follow:** Existing manifest store miss handling and scan-cache rebuildable corruption semantics.
- **Test scenarios:** Store path uses `<generation>.index.json`; successful store/load round-trips manifest and payload; missing payload prunes the manifest; corrupt payload prunes both files; stale payload rejects without panicking.
- **Verification:** Focused NTFS tests pass.

### U3. Wire configured live builds to write payload pairs

- **Goal:** Persist payload pairs after configured full-index builds without changing default behavior or enabling disk hits.
- **Requirements:** R8, R9, R10
- **Dependencies:** U1, U2
- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`, `crates/rebecca-core/src/scan.rs`
- **Approach:** Replace best-effort manifest-only writes with best-effort manifest-plus-payload writes when `WindowsNtfsMftIndexCache` has a store. Before writing, load the pair to avoid rewriting valid payloads. Log and ignore store failures.
- **Patterns to follow:** Existing `CachedNtfsVolumeIndex::store_manifest` behavior and `ScanEngine::with_ntfs_mft_manifest_cache_root`.
- **Test scenarios:** Default cache has no store; configured cache can store a payload pair; an existing valid pair is treated as present; portable scans still work with the configured constructor.
- **Verification:** Focused NTFS tests and workspace check pass.

### U4. Refresh roadmap docs

- **Goal:** Keep the cache roadmap honest for future work.
- **Requirements:** R11
- **Dependencies:** U1, U2, U3
- **Files:** `CHANGELOG.md`, `docs/performance/perf-matrix.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** State that payload persistence is now in place for configured builds, while cross-process cache hits still require live USN checkpoint capture and validation.
- **Patterns to follow:** Recent NTFS cache-boundary and manifest entries.
- **Test scenarios:** Wording does not imply default persistent cache reuse or deletion authority.
- **Verification:** `git diff --check`.

---

## Verification Contract

| Gate | Command | Proves |
|---|---|---|
| Format | `cargo fmt --all --check` | Rust formatting stayed stable. |
| NTFS focused tests | `cargo nextest run -p rebecca-core scan::windows_ntfs_mft` | Payload schema, pair store behavior, and existing NTFS behavior pass. |
| Constructor focused test | `cargo nextest run -p rebecca-core scan::tests::scan_engine_ntfs_manifest_constructor_keeps_portable_scans_available` | Explicit cache-root constructor still preserves portable scans. |
| Workspace check | `cargo check --workspace` | Cross-crate compilation remains sound. |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Rust lint baseline remains clean. |
| Workspace tests | `cargo nextest run --workspace` | Existing CLI/core/parser contracts still pass. |
| Diff hygiene | `git diff --check` | No whitespace errors. |

---

## Definition of Done

- NTFS volume-index payloads have a versioned JSON schema with fingerprint, generation, byte length, and checksum validation.
- The manifest store can persist and reload a manifest/payload pair, while treating missing or bad payloads as rebuildable misses.
- Configured full-index builds write payload pairs best-effort; default scans still do not write persistent NTFS cache files.
- No production path uses persistent payloads as cleanup evidence before USN freshness validation.
- Tests cover payload round-trip, mismatch rejection, corrupt/missing payload fallback, and configured runtime wiring.
- Changelog, performance docs, and engineering memory describe payload persistence as groundwork for USN-backed reuse.
