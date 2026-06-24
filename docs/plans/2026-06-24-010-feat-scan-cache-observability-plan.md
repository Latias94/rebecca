---
title: "feat: Add scan cache observability"
type: "feature"
date: "2026-06-24"
---

# feat: Add scan cache observability

## Summary

The controlled scan-cache integration is explicit and safe, but cache behavior
was invisible while a cleanup plan was being built. This slice adds planner
progress events for scan-cache hits, misses, and skipped writes so tests and
human CLI progress can distinguish cache acceleration from normal scanning.

The events stay out of `CleanupPlan` JSON because cache use is a build-process
diagnostic, not a cleanup result contract.

## Requirements

- R1. Planner progress reports scan-cache hits with the estimated byte count.
- R2. Planner progress reports scan-cache misses with a stable reason label.
- R3. Planner progress reports scan-cache write skips while keeping plan
  building successful.
- R4. Human `clean` progress surfaces scan-cache events when progress is
  enabled.
- R5. JSON plan output remains unchanged.
- R6. Directory targets still emit no scan-cache events until directory cache
  reuse is supported.

## Verification

- `cargo fmt --all`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
