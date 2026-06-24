---
title: "feat: Configure scan cache freshness policy"
type: "feature"
date: "2026-06-24"
---

# feat: Configure scan cache freshness policy

## Summary

The previous slice introduced `ScanCachePolicy` as an internal seam. This
slice exposes the directory-record freshness window through `config.toml` while
keeping `--scan-cache` as the explicit runtime opt-in.

The default remains 300 seconds. The on-disk scan-cache v1 record format,
planner progress events, and `CleanupPlan` JSON remain unchanged.

## Requirements

- R1. Missing `[scan_cache]` config keeps the default 300-second directory
  freshness window.
- R2. `[scan_cache].directory_record_max_age_seconds` overrides the directory
  freshness policy used by `clean --scan-cache`.
- R3. Invalid policy values fail as config parse/validation errors before plan
  building.
- R4. `clean` reads the policy through core runtime configuration, not by
  parsing config directly in the CLI.
- R5. Existing app path resolution and `config paths` output remain unchanged.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core config`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
