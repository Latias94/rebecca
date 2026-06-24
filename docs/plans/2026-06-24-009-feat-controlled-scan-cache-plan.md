---
title: "feat: Wire controlled scan cache into planner"
type: "feature"
date: "2026-06-24"
---

# feat: Wire controlled scan cache into planner

## Summary

Rebecca has a v1 scan-cache record format under the rebuildable cache
directory. This slice connects that store to cleanup plan building without
changing default behavior.

Reference projects point to two useful guardrails: Mole treats persistent scan
metadata as fingerprinted and optional, while BleachBit uses scan caching as a
local acceleration detail. Rebecca follows the conservative path here: planner
cache use is explicit, rebuildable, and soft-failing.

## Requirements

- R1. Existing planner entry points continue to build plans without scan cache.
- R2. A small planner context carries runtime concerns such as cancellation and
  optional scan-cache access.
- R3. `clean --scan-cache` explicitly enables scan-cache reads and writes.
- R4. v1 planner cache reuse is limited to regular-file target roots.
- R5. Directory target cache reuse is deferred until a stronger directory
  invalidation contract exists.
- R6. Cache misses, stale records, corrupted records, and write failures must
  not fail plan building.
- R7. CLI cache writes use the configured Rebecca cache directory.

## Scope Boundaries

- In scope: planner context seam, explicit CLI flag, file-target cache hit/write
  behavior, soft failure handling, and docs.
- Deferred: default cache use, directory target cache hits, TTLs, cache metrics,
  cross-process locking, and cache-hit user output.

## Verification

- `cargo fmt --all`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-cli clean_dry_run_does_not_write_scan_cache_by_default_for_file_targets clean_dry_run_scan_cache_flag_writes_file_target_cache`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
