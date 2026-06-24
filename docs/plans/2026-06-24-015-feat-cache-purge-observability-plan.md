---
title: "feat: Improve cache purge observability"
type: "feature"
date: "2026-06-24"
---

# feat: Improve cache purge observability

## Summary

`cache purge` already follows the preview-first lifecycle contract. This slice
adds clearer reporting around what the command is acting on and what it did:
lifecycle metadata, whether the cache directory exists, whether the command
preserves the directory, and per-status counts for would-delete/deleted/skipped/
failed entries.

The command still only targets Rebecca's own rebuildable cache directory. It
does not broaden the scope of deletion, and the core JSON shape stays stable
with additive fields only.

## Requirements

- R1. Human `cache purge` output reports lifecycle, preservation, existence,
  and entry-status counts.
- R2. JSON output exposes the same lifecycle and preservation facts.
- R3. The command still defaults to preview and still preserves the cache
  directory itself.
- R4. Missing cache directories remain a successful empty result.
- R5. Refusal on preserved-path overlap remains unchanged.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core cache`
- `cargo nextest run -p rebecca-cli --test cli_cache`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
