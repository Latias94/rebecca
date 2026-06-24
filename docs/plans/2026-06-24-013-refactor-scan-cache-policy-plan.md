---
title: "refactor: Introduce scan cache policy seam"
type: "refactor"
date: "2026-06-24"
---

# refactor: Introduce scan cache policy seam

## Summary

Scan-cache directory freshness was previously validated directly against a
module constant. This slice introduces `ScanCachePolicy` as the seam that owns
the directory record freshness window, keeps the current 5-minute default, and
threads the policy through `PlanBuildContext`.

The on-disk v1 record format, CLI flags, progress events, and cleanup plan JSON
remain unchanged. The change prepares the cache contract for later
configuration without making that configuration part of this slice.

Reference projects informed the shape: Mole uses explicit cache refresh TTLs
for metadata, and BleachBit refreshes cached open-file scans on a short window.
Rebecca keeps the same idea local to the scan-cache module so callers only
choose a policy when they have a real reason to vary it.

## Requirements

- R1. `ScanCacheStore::load` keeps the default policy behavior for existing
  callers.
- R2. `ScanCacheStore::load_with_policy` allows tests and future callers to
  evaluate records with an explicit policy.
- R3. `PlanBuildContext` carries the scan-cache policy so planner behavior can
  vary without changing planner internals.
- R4. Directory records expire according to the supplied policy; regular-file
  records remain age-insensitive.
- R5. The on-disk scan-cache record format and cleanup plan JSON remain
  unchanged.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core scan_cache`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
