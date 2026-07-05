---
title: "NTFS Persistent Cache Observability - Plan"
type: refactor
date: 2026-07-05
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Persistent Cache Observability - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Expose persistent NTFS/MFT cache miss and payload-write-skip reasons as bounded estimate caveats so dogfood reports explain cache behavior without requiring trace logs. |
| Authority | Do not weaken cache freshness rules; uncertain cache state must still rebuild or skip persistent writes. |
| Stop conditions | Stop if diagnostics would leak local device paths beyond existing report paths, duplicate unbounded messages, or make default no-store scans noisier. |
| Execution profile | Core Rust refactor, focused NTFS tests, dogfood parser reuse, changelog/docs update. |

---

## Requirements

- R1. When a manifest store is enabled and persistent payload lookup misses, the next rebuilt estimate includes a bounded `mft-persistent-cache-miss` caveat with a stable reason label.
- R2. When persistent payload write is skipped because no stable USN checkpoint is available, the rebuilt estimate includes `mft-persistent-cache-write-skipped`.
- R3. When persistent payload write fails for an IO/serialization reason, the estimate includes the same write-skipped code with a sanitized message.
- R4. Persistent-cache hits preserve existing payload caveats and do not add a miss caveat.
- R5. Default scans without a manifest store remain unchanged.
- R6. Dogfood/docs/changelog mention the new caveat evidence.

---

## Key Technical Decisions

- KTD1. Keep diagnostics in `ParseCaveat`.
  The existing JSON surface already bounds and serializes estimate caveats.
- KTD2. Return persistent-load diagnostics from `load_persistent_index` rather than tracing only.
  Rebuilt indexes can then carry the exact miss reason that forced rebuild.
- KTD3. Make payload writes return an optional caveat.
  The caller can append the write-skip reason before inserting the cache slot and before rendering output.
- KTD4. Use stable labels from existing miss enums.
  This avoids embedding volatile debug formatting in machine-readable fields.

---

## Verification Contract

| Gate | Command | Done signal |
|---|---|---|
| Formatting | `cargo fmt --check` | Rust formatting remains stable. |
| Focused tests | `cargo nextest run -p rebecca-core ntfs_volume_index` | Persistent cache tests pass. |
| Dogfood self-test | `pwsh -File scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1 -SelfTest` | Parser expectations still pass. |
| VHD wrapper self-test | `pwsh -File scripts/dogfood/run-ntfs-usn-replay-vhd-dogfood.ps1 -SelfTest` | Wrapper expectations still pass. |

---

## Definition of Done

- Cache miss/write-skip reasons are visible in normal inspect-map JSON caveats.
- Existing persistent-cache hit behavior remains unchanged.
- Dogfood docs tell users where to find these caveats.
- Focused tests and script self-tests pass.
