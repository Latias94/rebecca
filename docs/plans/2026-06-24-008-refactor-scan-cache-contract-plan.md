---
title: "refactor: Define scan cache contract"
type: "refactor"
date: "2026-06-24"
---

# refactor: Define scan cache contract

## Summary

Rebecca now has a rebuildable cache directory and an explicit `cache purge`
operation. The next safe step is to define the future scan-cache file contract
without changing scan behavior yet. The cache must be versioned, rebuildable,
and invalidated from root metadata so stale estimates are not treated as durable
state.

This slice keeps the scanner hot path untouched. It adds a typed store and
tests so later planner work can opt into caching against a known format.

## Requirements

- R1. Scan cache entries live under `AppPaths.cache_dir`.
- R2. The scan cache file format is versioned as `1`.
- R3. Cache records store the scanned root path, root metadata fingerprint,
  scan report, and write time.
- R4. Reads return stale/missing/corrupted as cache misses, not hard failures.
- R5. Unsupported future cache versions are treated as stale.
- R6. Cache writes create parent directories and write JSON atomically enough
  for local CLI use.
- R7. Docs explain that scan cache is rebuildable and can be purged safely.

## Scope Boundaries

- In scope: cache path naming, v1 JSON record shape, stale detection, and tests.
- Deferred: planner integration, TTLs, multiple cache namespaces, compression,
  or cross-process locking.
- Out of scope: preserving scan cache across `cache purge`.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core scan_cache`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
