---
title: "NTFS Live MFT Mirror Source Integration - Plan"
type: "refactor"
date: "2026-07-04"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS Live MFT Mirror Source Integration - Plan

## Goal Capsule

| Field | Decision |
|---|---|
| Objective | Wire the parser-level `$MFTMirr` recovery contract into the live sequential `$MFT` record source without making mirror data a required dependency or cleanup authority. |
| Authority | Builds on `docs/plans/2026-07-04-004-refactor-ntfs-raw-mirror-recovery-plan.md` after its parser boundary landed. |
| Execution profile | Small Rust/core integration with deterministic unit tests and docs. |
| Stop condition | Stop for any behavior that makes mirror read failure fail the primary sequential source, invents mirror length beyond bounded system records, or affects deletion authorization. |
| Landing strategy | One conventional commit after focused core tests, workspace check, clippy, and diff checks pass. |

## Product Contract

The parser can now merge primary `$MFT` bytes with bounded `$MFTMirr` bytes. Live sequential volume reads should use that capability for the small system-record mirror range exposed by `FSCTL_GET_NTFS_VOLUME_DATA.Mft2StartLcn`.

The contract stays conservative: valid primary `$MFT` records remain authoritative, mirror reads are best-effort, mirror read failure is a caveat rather than a source failure, and recovered records keep the parser's `mft-mirror-record-used` caveat.

## Requirements

- R1. Sequential live `$MFT` parsing reads at most the first bounded system-record mirror window.
- R2. Mirror bytes are passed to `MftRecordReader::parse_records_from_with_mirror` only for primary chunks whose record ids overlap the mirror window.
- R3. Mirror read failure records a bounded caveat and continues primary `$MFT` parsing.
- R4. Successful primary records remain authoritative; mirror data is never used to override a valid primary record.
- R5. Tests cover mirror read planning, absent mirror data, and chunk/mirror overlap selection without requiring live volume access.

## Implementation Units

### U1. Attach mirror bytes to sequential parse chunks

- **Files:** `crates/rebecca-core/src/scan/windows_ntfs_mft.rs`
- **Approach:** Add a small `MftMirrorReadPlan` derived from `Mft2StartLcn`, record geometry, and a fixed system-record count. Read those bytes once during sequential source setup. Store an optional mirror chunk in `SequentialMftReadContext`, attach it only to overlapping primary chunks, and let `parse_sequential_mft_chunks` call the mirror-aware parser API.
- **Test scenarios:** mirror plan computes byte offset/length; zero `Mft2StartLcn` disables mirror reading; non-overlapping primary chunks do not carry mirror bytes; overlapping chunks do.
- **Verification:** `cargo nextest run -p rebecca-core scan::windows_ntfs_mft`.

### U2. Update docs and memory

- **Files:** `CHANGELOG.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Document that live sequential `$MFT` parsing now has best-effort mirror recovery, while targeted FSCTL traversal and broader raw-image mounting remain separate concerns.
- **Verification:** `git diff --check`.

## Verification Contract

```powershell
cargo fmt --all --check
cargo nextest run -p rebecca-core scan::windows_ntfs_mft
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

## Definition of Done

- Live sequential MFT chunks can carry bounded `$MFTMirr` recovery bytes into the parser.
- Mirror read failure does not force fallback away from the sequential source.
- Unit tests cover planning and chunk overlap logic.
- Changelog and engineering memory describe the new boundary and remaining targeted/raw-image gaps.
