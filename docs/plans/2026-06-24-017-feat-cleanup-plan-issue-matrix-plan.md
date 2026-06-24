---
title: "feat: Add cleanup plan issue matrix"
type: "feature"
date: "2026-06-24"
---

# feat: Add cleanup plan issue matrix

## Summary

Cache purge now exposes stable reason codes and an issue matrix. This slice
applies the same contract to normal cleanup plans so skipped, blocked, and
failed cleanup targets can be grouped by stable reason code in human output,
JSON output, and persisted history.

Raw `reason` strings remain available for local detail. The new fields are
additive: older history records that lack `summary.issue_matrix` or target
`reason_code` still deserialize.

## Requirements

- R1. Cleanup targets can carry a stable `reason_code` for skipped, blocked, and
  failed statuses.
- R2. Cleanup summaries aggregate issue targets into an `issue_matrix` keyed by
  status and reason code.
- R3. Planner, scan, and executor code assign reason codes at the source of the
  status transition.
- R4. Human `clean` output prints the issue matrix when issues exist.
- R5. JSON output includes target `reason_code` and summary `issue_matrix` as
  additive fields.
- R6. Older plan/history JSON without the new fields still loads.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core -E 'binary(model_contract)'`
- `cargo nextest run -p rebecca-core -E 'binary(executor_contract)'`
- `cargo nextest run -p rebecca-core overlapping_templates_are_deduplicated_before_sizing`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
