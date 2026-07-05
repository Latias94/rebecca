---
title: "NTFS USN Replay VHD Dogfood - Plan"
type: refactor
date: 2026-07-05
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS USN Replay VHD Dogfood - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Add an isolated NTFS VHD dogfood wrapper so persistent NTFS/MFT cache hits can be verified on a small quiet volume instead of a large active workstation volume. |
| Authority | Never format or mutate an existing user volume; only operate on a newly created VHD path that does not already exist. |
| Stop conditions | Stop if the script cannot choose an unused drive letter, if DiskPart cannot create/attach the VHD, or if cleanup cannot be bounded to the created VHD artifact. |
| Execution profile | PowerShell wrapper, documentation, self-test, and one live elevated VHD run when available. |

---

## Product Contract

### Summary

The real E: dogfood run proved the full-index path is too large and active for stable persistent payload checkpoints.
This plan adds a small dynamic VHDX wrapper around the existing USN replay dogfood script.
The wrapper creates a fresh NTFS volume, runs the four-phase replay dogfood on that volume, detaches the VHD by default, and records wrapper metadata beside the inner dogfood report.

### Requirements

- R1. The wrapper creates a new dynamic VHDX and refuses to overwrite an existing VHD path.
- R2. The wrapper formats only the new VHD partition and assigns an unused drive letter.
- R3. The wrapper calls `run-ntfs-usn-replay-dogfood.ps1` with the fixture root on the mounted VHD and report output outside the VHD.
- R4. The wrapper detaches the VHD by default and preserves the VHD file for post-run inspection.
- R5. VHD deletion is explicit with `-RemoveVhd`, only after detach, and only when the VHD path is under the wrapper output directory.
- R6. Self-test validates command construction and path-boundary logic without requiring elevation or DiskPart.
- R7. README, release checklist, and changelog document the stable VHD path.

---

## Key Technical Decisions

- KTD1. Use DiskPart instead of `New-VHD`.
  DiskPart is available on more Windows installs than the Hyper-V PowerShell module.
- KTD2. Keep the VHD workflow as a wrapper instead of folding it into the core USN replay script.
  The inner script remains usable against any existing NTFS folder; the wrapper owns disk lifecycle.
- KTD3. Default to detach-but-keep.
  Keeping the VHD file gives release/debug evidence without leaving a mounted drive around.
- KTD4. Require explicit `-RemoveVhd`.
  This keeps destructive cleanup opt-in and bounded to the wrapper output directory.

---

## Verification Contract

| Gate | Command | Done signal |
|---|---|---|
| VHD wrapper self-test | `pwsh -File scripts/dogfood/run-ntfs-usn-replay-vhd-dogfood.ps1 -SelfTest` | Self-test passes without elevation. |
| USN dogfood self-test | `pwsh -File scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1 -SelfTest` | Parser and expectation tests pass. |
| Live VHD dogfood | `pwsh -File scripts/dogfood/run-ntfs-usn-replay-vhd-dogfood.ps1 -VhdSizeMB 256 -TimeoutSeconds 180 -IndexTimeoutSeconds 30` | On elevated Windows with DiskPart, report is produced; persistent-cache expectations pass or failure reason is recorded. |
| Formatting | `cargo fmt --check` | Rust formatting remains unchanged. |

---

## Definition of Done

- The VHD wrapper can run the existing USN replay dogfood on an isolated NTFS volume.
- The script does not overwrite or format existing user volumes.
- Documentation points release dogfood to the VHD path when persistent-cache evidence is required.
- Verification results and any host limitations are recorded.
