---
title: "feat: Add explicit Rebecca cache purge command"
type: "feat"
date: "2026-06-24"
---

# feat: Add explicit Rebecca cache purge command

## Summary

Rebecca now exposes storage lifecycle metadata, including a rebuildable cache
directory. The next safe operation is an explicit command for Rebecca's own
cache directory only. It should preview by default, require `--yes` for
deletion, and refuse to run if the configured cache path overlaps preserved
state, history, or configuration paths.

This follows the existing preview-first cleanup posture and Mole's habit of
making stateful/destructive operations visible before execution. It does not
broaden built-in cleanup rules or delete third-party application data.

## Requirements

- R1. Add `rebecca cache purge` for Rebecca's own configured cache directory.
- R2. Preview is the default; actual deletion requires `--yes`.
- R3. `--json` prints a stable report with mode, cache path, bytes, file count,
  directory count, and deleted flag.
- R4. Missing cache directories are a successful empty result.
- R5. The command refuses to operate when cache path overlaps config, state, or
  history paths.
- R6. Purging deletes direct cache contents while preserving the cache directory
  itself.

## Scope Boundaries

- In scope: Rebecca's own `AppPaths.cache_dir` only.
- Deferred: scan-cache formats, cache TTLs, cache invalidation keys, and
  third-party application cleanup.
- Out of scope: deleting config files, history files, state directories, or
  any user-selected arbitrary path.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core cache`
- `cargo nextest run -p rebecca-cli --test cli_cache`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
