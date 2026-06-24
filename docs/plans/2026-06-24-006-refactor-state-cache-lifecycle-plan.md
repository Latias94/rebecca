---
title: "refactor: Pin state and cache lifecycle policy"
type: "refactor"
date: "2026-06-24"
---

# refactor: Pin state and cache lifecycle policy

## Summary

Rebecca already resolves config, state, cache, and history paths. The next
contract risk is that those paths do not yet say which data must be preserved
and which data can be rebuilt. This slice adds a small typed lifecycle model so
future cache cleanup or scan-cache work does not accidentally treat history or
config as disposable.

The posture follows Mole's operational discipline: destructive or stateful
surfaces should be visible, dry-run friendly, and easy to audit. Rebecca keeps
its Windows storage layout and TOML config, but makes the storage lifecycle
machine-readable through `config paths --json`.

## Requirements

- R1. Config paths are classified as user-editable configuration.
- R2. State paths are classified as durable local state.
- R3. History is classified as append-only audit state and preserved.
- R4. Cache paths are classified as rebuildable cache.
- R5. `config paths --json` remains backward-compatible for existing path
  fields while adding machine-readable lifecycle metadata.
- R6. README, ADR, and durable engineering state describe the same policy.

## Implementation Units

### U1. Core Lifecycle Model

- Add typed storage ids, lifecycle classes, and retention policy.
- Derive a lifecycle inventory from `AppPaths`.
- Cover ordering and classification with core tests.

### U2. CLI Contract

- Add lifecycle metadata to `config paths --json`.
- Keep existing top-level path fields intact.
- Add a CLI regression for the rebuildable cache and preserved history entries.

### U3. Documentation Alignment

- Update README's Local State section.
- Extend ADR 0008 with lifecycle policy details.
- Refresh `docs/knowledge/engineering/current-state.md`.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core config`
- `cargo nextest run -p rebecca-cli --test cli_output`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
