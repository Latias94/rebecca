---
title: "feat: Add bounded directory scan cache reuse"
type: "feature"
date: "2026-06-24"
---

# feat: Add bounded directory scan cache reuse

## Summary

The scan cache already accelerates regular-file targets and reports its
process-level progress. This slice extends the same contract to directory
targets, but only when the cached record is still fresh enough to trust.

The freshness rule stays local to the scan-cache module and keeps plan JSON
unchanged. Directory cache writes remain soft-failing, and expired directory
records fall back to a full rescan.

## Requirements

- R1. Fresh directory-target records can be reused as scan-cache hits when the
  root metadata still matches.
- R2. Expired directory-target records are treated as cache misses and force a
  rescan.
- R3. Regular-file scan-cache behavior remains unchanged.
- R4. Miss labels distinguish expired directory records from stale, corrupted,
  missing, and metadata-unavailable records.
- R5. Planner progress and CLI human progress keep reporting cache
  hit/miss/write-skipped events without changing plan JSON.
- R6. Directory cache writes remain soft-failing.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core scan_cache`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
