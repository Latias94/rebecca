---
title: "NTFS USN Replay Dogfood - Plan"
type: refactor
date: 2026-07-05
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# NTFS USN Replay Dogfood - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Make the real `inspect map` CLI path able to opt into persistent NTFS/MFT volume-index cache reuse, then dogfood USN replay hits and misses with a local fixture. |
| Authority | Keep persistent cache writes explicit and diagnostic-only; default CLI scans must not create long-lived MFT payloads. |
| Stop conditions | Stop if the CLI path cannot isolate cache state per run, if the dogfood script cannot distinguish persistent hits from rebuilds, or if implementation would make NTFS/MFT cache state deletion authority. |
| Execution profile | Code implementation plus PowerShell dogfood scripts, docs, changelog, self-tests, and focused Rust tests. |

---

## Product Contract

### Summary

USN replay now lets a persisted NTFS/MFT payload survive unrelated volume changes, but the canonical `inspect map` command still creates a plain `ScanEngine`.
That means real CLI dogfood cannot exercise the persistent payload store unless tests call the core API directly.
This plan adds an explicit diagnostic opt-in, then creates a dogfood fixture that warms the payload, mutates an unrelated directory, verifies a persistent-cache hit, mutates the target subtree, verifies a rebuild, and verifies a post-rebuild hit.

### Requirements

**CLI behavior**

- R1. `inspect map --scan-backend windows-ntfs-mft-experimental` can opt into a persistent NTFS/MFT volume-index store with `REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE=1`.
- R2. Without the opt-in, `inspect map` keeps the current default behavior and does not create persistent MFT payloads.
- R3. The opt-in uses the normal Rebecca app cache directory so dogfood can isolate it with `REBECCA_CACHE_DIR`.

**Dogfood evidence**

- R4. A new dogfood script creates target and unrelated same-volume fixture subtrees and runs warm, unrelated-change, target-change, and post-rebuild phases.
- R5. Each phase records stdout, stderr, duration, backend source labels, source kind, logical totals, and cache file counts.
- R6. The script exits non-zero when expected persistent-cache hit or rebuild phases are not observed, unless explicitly allowed for exploratory collection.

**Docs and tests**

- R7. Self-tests cover report parsing and phase expectation logic without requiring live NTFS.
- R8. The dogfood README, release checklist, configuration docs, and Unreleased changelog document the env var and command.

### Scope Boundaries

- No default persistent cache writes.
- No new public stable cache flag in the CLI help surface.
- No incremental payload mutation.
- No changes to cleanup deletion authority.

---

## Planning Contract

### Key Technical Decisions

- KTD1. Put the cache root on `DiskMapRequest`, not on CLI-only globals.
  This keeps the core inspect-map API testable and lets future callers choose the same cache boundary explicitly.
- KTD2. Gate the CLI cache root with an environment variable instead of a user-facing flag.
  The feature is diagnostic and experimental; ordinary users should not accidentally persist large MFT payloads.
- KTD3. Reuse the app cache directory rather than inventing a parallel path.
  Existing runtime env isolation already gives dogfood a clean per-run cache by setting `REBECCA_CACHE_DIR`.
- KTD4. Keep the USN replay dogfood script separate from the generic backend comparison script.
  This flow mutates the filesystem between phases and has cache-specific pass criteria.
- KTD5. Teach generic inspect-map reporting about the `persistent-cache` backend source so existing release evidence remains readable.

---

## Implementation Units

### U1. Core inspect-map cache-root plumbing

- **Goal:** Let `DiskMapRequest` carry an optional NTFS/MFT manifest cache root and build the scan engine from it.
- **Requirements:** R1, R2, R3
- **Files:** `crates/rebecca-core/src/disk_map.rs`
- **Approach:** Add an optional cache-root field plus builder, and use `ScanEngine::with_ntfs_mft_manifest_cache_root` only when the request provides it.
- **Verification:** Focused Rust unit test proves the request carries the root; existing map tests cover default behavior.

### U2. CLI opt-in environment gate

- **Goal:** Enable the cache root in `inspect map` only for the experimental NTFS/MFT backend and only when the diagnostic env var is truthy.
- **Requirements:** R1, R2, R3
- **Files:** `crates/rebecca/src/inspect.rs`
- **Approach:** Parse `REBECCA_NTFS_MFT_VOLUME_INDEX_CACHE`; when enabled, load runtime config and set the request cache root to `runtime_config.app_paths.cache_dir`.
- **Verification:** Unit tests cover truthy parsing; focused CLI inspect-map tests continue to pass.

### U3. Dogfood source classification and USN replay script

- **Goal:** Produce repeatable evidence for persistent-cache hits and target-subtree invalidation.
- **Requirements:** R4, R5, R6, R7
- **Files:** `scripts/dogfood/run-inspect-map-report.ps1`, `scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1`
- **Approach:** Add `persistent-cache` source classification to generic reports. Add a dedicated script with isolated runtime env, deterministic fixture creation, phase expectations, JSON/Markdown output, raw stdout/stderr capture, and self-test parser checks.
- **Verification:** PowerShell self-tests pass without live NTFS; live dogfood is best-effort on the current workstation.

### U4. Docs and release notes

- **Goal:** Document how to run and interpret the new diagnostic path.
- **Requirements:** R8
- **Files:** `scripts/dogfood/README.md`, `docs/configuration.md`, `docs/release.md`, `CHANGELOG.md`
- **Approach:** Add the env var, dogfood command, expected source kinds, and failure interpretation under existing NTFS sections.
- **Verification:** Diff review confirms the changelog entry is under Unreleased.

---

## Verification Contract

| Gate | Command | Done signal |
|---|---|---|
| Generic dogfood parser | `pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest` | Self-test passes. |
| Fixture dogfood parser | `pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 -SelfTest` | Self-test passes. |
| USN dogfood parser | `pwsh -File scripts/dogfood/run-ntfs-usn-replay-dogfood.ps1 -SelfTest` | Self-test passes. |
| Formatting | `cargo fmt` | No formatting errors. |
| Focused core tests | `cargo nextest run -p rebecca-core disk_map` | Focused tests pass. |
| Focused CLI tests | `cargo nextest run -p rebecca inspect` | Focused tests pass. |
| Workspace regression | `cargo nextest run --workspace` | All workspace tests pass. |

---

## Definition of Done

- `inspect map` can explicitly use the persistent NTFS/MFT volume-index store through isolated app cache paths.
- Default scans remain free of persistent MFT payload writes.
- Dogfood evidence can prove warm build, unrelated-change hit, target-change miss, and post-rebuild hit phases.
- Documentation and changelog explain the diagnostic workflow.
- Verification gates pass or any live-NTFS limitation is recorded with the report path.
