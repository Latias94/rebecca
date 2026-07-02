---
artifact_contract: "ce-unified-plan/v1"
artifact_readiness: "implementation-ready"
execution: "code"
product_contract_source: "ce-plan-bootstrap"
origin: "conversation"
created: "2026-07-02"
last_updated: "2026-07-02"
title: "Portable Disk Map Streaming Refactor - Plan"
---

# Portable Disk Map Streaming Refactor - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Refactor the default portable `inspect map` implementation so it ranks entries through a streaming post-order traversal instead of materializing the full directory tree in memory. |
| User value | The default disk-map command remains useful on large source trees, build directories, and cache-heavy roots without requiring NTFS privileges or full-volume metadata access. |
| Safety stance | Report-only. The command never authorizes cleanup, never writes history, and preserves reparse-point boundaries. |
| Primary surfaces | `crates/rebecca-core/src/disk_map.rs`, `crates/rebecca-core/tests/disk_map.rs`, docs and changelog. |
| Stop conditions | Stop if totals change for ordinary roots, if `--max-depth` starts limiting totals, if reparse protection weakens, or if the implementation stores an unbounded rendered tree again. |

## Product Contract

### Summary

The NTFS backend now avoids full-volume indexing for scoped roots. The next bottleneck is the default backend that every host can use: portable `inspect map`.

Today portable map recursively builds `PortableDiskMapNode` values containing every child node, then walks that in-memory tree again to push visible entries into the bounded top heap. That is simple, but it is the wrong shape for a best-in-class cleanup CLI: a huge `target`, `node_modules`, package cache, or repository root can create a large transient tree even when the user only asked for `--top 20`.

This slice should keep the same public report contract while changing the implementation to stream:

- read a directory's children in deterministic order;
- recursively compute each child's metrics;
- push the child's final aggregate directly into `DiskMapTopEntries` if visible;
- return only aggregate metrics to the parent.

### Requirements

- R1. Portable `inspect map` must not retain a full `children: Vec<PortableDiskMapNode>` tree.
- R2. Totals must remain exact for files, directories, and logical bytes under ordinary readable roots.
- R3. `--top 0` must still preserve totals with no rendered entries.
- R4. `--max-depth` must limit only rendered/ranked entries, not traversal or totals.
- R5. Reparse-like roots and child entries must remain skipped and must not be traversed.
- R6. Entry ranking must remain deterministic for equal sizes.
- R7. The public JSON/NDJSON/human contracts must not change.
- R8. Tests must prove the streaming implementation does not require child-node materialization and preserves existing behavior.

### Acceptance Examples

- AE1. Given a root with two same-sized child directories, when `inspect map --top 2` runs, then ranking remains deterministic by path.
- AE2. Given a nested file below `depth = 2`, when `--max-depth 1` is set, then totals include the file while top entries omit the deeper node.
- AE3. Given `--top 0`, when the root contains files, then totals are nonzero and `top_entries` is empty.
- AE4. Given a root file, when portable map runs, then it reports that file as a depth-0 top entry.
- AE5. Given a reparse-like child, when portable map runs, then the child is not traversed and totals do not include its target.

## Planning Contract

### Key Technical Decisions

- KTD1. Keep the current portable backend contract and refactor only internals.
  - Rationale: The CLI/API surface is already correct; this slice should improve performance and memory behavior without creating a compatibility break.

- KTD2. Use post-order traversal rather than a two-phase node tree.
  - Rationale: Directory sizes are only known after children are measured. Post-order lets the implementation push exact aggregate entries while retaining only the active traversal stack and bounded top heap.

- KTD3. Keep deterministic per-directory sorting.
  - Rationale: Cleanup tools must be predictable. Sorting child paths before traversal preserves stable tie-breaking even when OS enumeration order changes.

- KTD4. Defer parallel portable disk-map traversal until after the streaming refactor.
  - Rationale: Removing full-tree materialization is the simpler architectural prerequisite. Parallelism can be added later with clearer memory bounds.

### High-Level Design

```text
inspect_portable_root
  -> metadata / root boundary checks
  -> if file: push file entry and return metrics
  -> if directory:
       read sorted direct children
       for each child:
          inspect_portable_node(root, child, depth)
             -> recursively returns DiskMapMetrics
             -> pushes visible entry after child metrics are complete
       return accumulated root metrics
```

The old `PortableDiskMapNode` type should disappear. If a small helper remains, it should contain only one completed entry's path, kind, depth, and metrics; it must not own child nodes.

### System-Wide Impact

| Area | Impact |
|---|---|
| `crates/rebecca-core/src/disk_map.rs` | Replace full-tree node materialization with streaming post-order aggregation. |
| `crates/rebecca-core/tests/disk_map.rs` | Keep existing behavior tests and add root-file / max-depth / top-zero coverage where missing. |
| `README.md`, `CHANGELOG.md`, `docs/performance/perf-matrix.md` | Document that portable map keeps bounded ranking memory. |

### Risks And Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Totals accidentally exclude directory descendants when `--max-depth` is set. | Wrong reclaim insight. | Tests must assert totals with hidden deeper entries. |
| Refactor changes tie ordering. | Flaky output and CLI tests. | Keep `DiskMapTopEntries` rank untouched and preserve sorted child traversal. |
| Reparse-point handling regresses. | Unsafe traversal outside requested tree. | Keep `symlink_metadata` and `is_reparse_like` checks before directory descent. |
| Error handling expands scope unexpectedly. | Compatibility surprise. | Preserve current fatal behavior for non-root unreadable entries in this slice; robust partial diagnostics can be a later plan. |

## Implementation Units

### U1. Remove Full Portable Node Tree

- **Goal:** Delete `PortableDiskMapNode.children` and the second-pass `push_node_if_visible` traversal.
- **Files:** `crates/rebecca-core/src/disk_map.rs`
- **Approach:** Replace `inspect_portable_node` so it returns `DiskMapMetrics` and pushes a completed `DiskMapEntry` after child metrics are aggregated. Keep a tiny local completed-entry helper only if it simplifies file/directory handling.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map`

### U2. Preserve Behavior With Focused Tests

- **Goal:** Lock public semantics while internals change.
- **Files:** `crates/rebecca-core/tests/disk_map.rs`
- **Approach:** Keep existing deterministic ranking, `--top 0`, missing root, fallback, and max-depth tests. Add or adjust root-file and nested max-depth assertions if needed.
- **Verification:** `cargo nextest run -p rebecca-core --test disk_map`; `cargo nextest run -p rebecca --test cli_inspect`

### U3. Update Docs And Changelog

- **Goal:** Make the default backend improvement visible.
- **Files:** `CHANGELOG.md`, `README.md`, `docs/performance/perf-matrix.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`
- **Approach:** State that portable disk-map ranking now uses bounded top-entry memory and no full in-memory report tree.
- **Verification:** `git diff --check`

### U4. Full Verification And Commit

- **Goal:** Ship the default disk-map streaming refactor as one logical commit.
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
| DoD1 | Portable disk-map traversal no longer stores child node trees. |
| DoD2 | Existing JSON/NDJSON/human output contracts remain unchanged. |
| DoD3 | Totals, ranking, `--top 0`, `--max-depth`, file roots, and reparse boundaries are covered by tests. |
| DoD4 | Docs and changelog describe the bounded-memory default map improvement. |
| DoD5 | Focused and full verification passes, and the change is committed. |
