---
artifact_contract: "ce-unified-plan/v1"
artifact_readiness: "implementation-ready"
execution: "code"
product_contract_source: "ce-plan-bootstrap"
origin: "conversation"
created: "2026-07-02"
last_updated: "2026-07-02"
title: "Disk Map Partial Diagnostics - Plan"
---

# Disk Map Partial Diagnostics - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Make default `inspect map` resilient to child-level metadata, directory-read, and directory-entry failures by returning partial results with explicit diagnostics instead of failing the whole command. |
| User value | Users still get useful ranked disk insight when one cache directory, protected folder, or racing file disappears during traversal. |
| Safety stance | Report-only and conservative. Unreadable or unstable entries are skipped, never estimated optimistically, and every skipped subtree is surfaced as a diagnostic. |
| Primary surfaces | `crates/rebecca-core/src/disk_map.rs`, `crates/rebecca-core/tests/disk_map.rs`, CLI inspect tests, docs/changelog. |
| Stop conditions | Stop if root-level failures become silent, if unreadable subtrees are counted as exact bytes, if diagnostics are omitted from JSON/NDJSON, or if cleanup authorization changes. |

## Product Contract

### Summary

Portable `inspect map` now streams aggregation without retaining a full tree. The next hardening slice is failure semantics.

The current portable implementation treats a non-root child metadata/read-dir/read-entry failure as a fatal `ScanFailed` error. That is too brittle for a cleanup CLI inspecting real user machines: cache directories can be permission-protected, files can disappear during traversal, and app caches can mutate while Rebecca is reading them. A best-in-class report should preserve exactness for scanned entries while clearly marking skipped areas.

### Requirements

- R1. Missing or unreadable root paths must keep the existing root status behavior: root missing or root metadata failure produces a skipped root, not a fake partial scan.
- R2. Child metadata failures must not abort the whole report; they must add `metadata-read-skipped` diagnostics and skip that entry's bytes.
- R3. Child directory open failures must add `directory-read-skipped` diagnostics and skip that subtree's bytes.
- R4. Directory entry iteration failures must add `directory-entry-read-skipped` diagnostics and continue with remaining entries.
- R5. Reparse-like child entries must add `reparse-point-skipped` diagnostics and must not traverse the target.
- R6. Root totals must remain conservative: scanned bytes are exact for scanned entries, skipped entries contribute zero, and diagnostics explain the gap.
- R7. Human, JSON, and NDJSON output must continue to include diagnostics through the existing disk-map report model.
- R8. Tests must cover child-level skip diagnostics without requiring platform-specific ACL setup.

### Acceptance Examples

- AE1. Given a root containing a child path that disappears between directory enumeration and metadata read, when `inspect map` runs, then the report succeeds, totals exclude that child, and diagnostics include `metadata-read-skipped`.
- AE2. Given a directory entry read error, when traversal continues, then diagnostics include `directory-entry-read-skipped` and other readable siblings still contribute to totals.
- AE3. Given an unreadable child directory, when map traversal reaches it, then diagnostics include `directory-read-skipped`, the child subtree contributes zero, and the command still exits successfully.
- AE4. Given a reparse-like child, when portable map runs, then diagnostics include `reparse-point-skipped` and no target bytes are counted.
- AE5. Given a missing root, when map runs, then the existing `root-missing` skipped-root behavior remains unchanged.

## Planning Contract

### Key Technical Decisions

- KTD1. Keep root failures distinct from child failures.
  - Rationale: A missing root means the requested unit was not inspected. A failed child means the requested root was partially inspected and must be diagnostic-rich.

- KTD2. Use existing `DiskMapDiagnosticKind` variants.
  - Rationale: The report model already has the right vocabulary; this slice should wire it rather than expand the API.

- KTD3. Treat skipped child bytes as zero, not unknown estimates.
  - Rationale: The CLI must not overstate reclaim potential. Diagnostics provide uncertainty; totals remain exact for what was actually scanned.

- KTD4. Add test hooks only if natural filesystem races are not deterministic.
  - Rationale: Tests should not depend on Windows ACL privileges. If needed, isolate failure injection behind a crate-private walker trait or small test-only hook.

### High-Level Design

```text
inspect_portable_root
  -> root metadata remains strict/skipped
  -> inspect_portable_node(..., diagnostics)
       metadata failure -> diagnostic + zero metrics
       reparse child -> diagnostic + zero metrics
       read_dir failure -> diagnostic + zero metrics or directory-only metrics
       read_dir entry failure -> diagnostic + continue
       readable child -> exact metrics
```

The public report stays the same; only the internal traversal changes from fatal child errors to diagnostic child skips.

### System-Wide Impact

| Area | Impact |
|---|---|
| `crates/rebecca-core/src/disk_map.rs` | Thread diagnostics through portable traversal and convert child-level IO failures into skip diagnostics. |
| `crates/rebecca-core/tests/disk_map.rs` | Add deterministic child-failure tests, preferably through natural missing-after-enumeration fixtures or a crate-private test walker. |
| `crates/rebecca/src/render/inspect.rs` | No contract change expected; verify human output still renders diagnostics clearly. |
| Docs / changelog | Document partial diagnostics for default disk maps. |

### Risks And Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Partial results are mistaken for complete inventory. | User underestimates used space. | Diagnostics must be emitted and visible in machine/human output. |
| Tests require OS-specific permission tricks. | Flaky CI. | Prefer deterministic injection over ACL manipulation. |
| Root failures accidentally become partial successes. | Bad UX and unclear errors. | Keep root path handling outside the child-level skip path. |
| Too many diagnostics become noisy. | Large unreadable trees flood output. | If needed, add bounded diagnostic summarization in a later slice; this slice can preserve existing vector behavior for correctness. |

## Implementation Units

### U1. Thread Diagnostics Through Portable Traversal

- **Goal:** Make child-level traversal able to record diagnostics without aborting.
- **Files:** `crates/rebecca-core/src/disk_map.rs`
- **Approach:** Pass `&mut Vec<DiskMapDiagnostic>` through `inspect_portable_directory_root` and `inspect_portable_node`. Convert child metadata/read-dir/read-entry errors into diagnostics and zero metrics while preserving root-level skipped-root handling.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map`

### U2. Deterministic Partial-Failure Tests

- **Goal:** Prove partial diagnostics without relying on host permissions.
- **Files:** `crates/rebecca-core/tests/disk_map.rs`
- **Approach:** Use deterministic race-style fixtures where possible, or introduce a crate-private/test-only walker seam to inject metadata/read-dir/read-entry failures.
- **Verification:** Tests cover metadata skip, directory read skip, entry read skip, reparse child skip where practical, and unchanged missing-root behavior.

### U3. CLI And Docs

- **Goal:** Keep output contracts stable and document partial diagnostics.
- **Files:** `crates/rebecca/tests/cli_inspect.rs`, `README.md`, `CHANGELOG.md`, `docs/performance/perf-matrix.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** Add or adjust CLI assertions only if render output changes. Update docs to say default disk maps can return conservative partial reports with diagnostics for unreadable or racing children.

### U4. Verification And Commit

- **Verification:**
  - `cargo fmt --all --check`
  - `cargo check --workspace`
  - `cargo nextest run -p rebecca-core --test disk_map`
  - `cargo nextest run -p rebecca --test cli_inspect --test cli_api`
  - `cargo nextest run --workspace`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo check -p rebecca-core --benches`
  - `pwsh -File scripts/ntfs/run-live-mft-dogfood.ps1 -SelfTest`
  - `git diff --check`

## Definition Of Done

| ID | Done Condition |
|---|---|
| DoD1 | Child-level metadata/read-dir/read-entry failures produce diagnostics and do not abort the whole disk-map report. |
| DoD2 | Root-level missing/unreadable behavior remains explicit and unchanged. |
| DoD3 | Reparse child skips are diagnostic-visible and non-traversing. |
| DoD4 | Machine and human output contracts stay stable. |
| DoD5 | Focused and full verification passes, and the change is committed. |
