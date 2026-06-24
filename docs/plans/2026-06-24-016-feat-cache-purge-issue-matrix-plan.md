---
title: "feat: Add cache purge issue matrix"
type: "feature"
date: "2026-06-24"
---

# feat: Add cache purge issue matrix

## Summary

`cache purge` already reports lifecycle, preservation, and status counts. This
slice adds a stable issue matrix for skipped and failed entries so human output
and JSON output can explain why an entry was not purged without relying on raw
error strings as the only signal.

## Requirements

- R1. Skipped and failed cache entries carry a stable reason code.
- R2. Core reports aggregate skipped and failed entries into a stable issue
  matrix keyed by status and reason code.
- R3. Human `cache purge` output prints the issue matrix before per-entry
  details.
- R4. JSON output includes the new issue matrix as an additive field.
- R5. Successful preview/delete flows with no issues keep the existing output
  shape.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core cache`
- `cargo nextest run -p rebecca-cli cache`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
