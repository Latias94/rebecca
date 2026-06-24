---
title: "feat: Add scan cache human summary"
type: "feature"
date: "2026-06-24"
---

# feat: Add scan cache human summary

## Summary

Scan-cache progress events now expose hits, misses, and skipped writes while a
plan is being built. This slice keeps counting those events after the progress
spinner is disabled or cleared, then prints a compact summary in human cleanup
plan output.

The summary remains a CLI diagnostic. It does not become part of
`CleanupPlan`, and JSON output stays unchanged.

## Requirements

- R1. Human `clean` output summarizes scan-cache hits, misses, and skipped
  writes when any cache activity happened.
- R2. Counting works even when `--no-progress` suppresses the spinner.
- R3. JSON output remains unchanged and does not gain scan-cache fields.
- R4. The summary stays out of `CleanupPlan` core serialization.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run -p rebecca-cli --test output`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
