---
title: "refactor: Version the config schema contract"
type: "refactor"
date: "2026-06-24"
---

# refactor: Version the config schema contract

## Summary

Rebecca now loads `config.toml`, so the next conservative step is to pin the
schema version before more settings are added. The goal is a narrow contract:
current configs remain accepted, explicit `version = 1` is documented, and
unsupported versions fail clearly instead of being partially interpreted.

Mole's purge path config is a useful reference for this posture: missing or
empty config stays a default case, custom config is easy to inspect, and tests
pin the behavior at the command boundary. Rebecca keeps TOML and Windows app
paths, but follows the same rule that config behavior should be obvious and
regression-tested.

## Requirements

- R1. `config.toml` has a current schema version of `1`.
- R2. Missing `version` defaults to schema version `1` so the initial
  config-file slice remains compatible.
- R3. Unsupported versions fail with a file-scoped validation error.
- R4. Unknown keys and malformed TOML keep failing clearly.
- R5. README and ADR text show the versioned schema shape users should copy.
- R6. CLI tests prove unsupported versions are not silently accepted.

## Implementation Units

### U1. Core Schema Contract

- Add a `CONFIG_SCHEMA_VERSION` constant.
- Add `version` to `RebeccaConfig` with a default of `1`.
- Validate the parsed config before resolving app paths.

### U2. CLI Error Boundary

- Cover unsupported config versions through `rebecca config paths`.
- Keep malformed TOML coverage separate from semantic validation coverage.

### U3. Documentation Alignment

- Update README's Local State section with a copyable v1 TOML example.
- Mark the configuration/local-state ADR as accepted and record the versioning
  decision.
- Refresh durable engineering state after verification.

## Verification

- `cargo fmt --all --check`
- `cargo nextest run -p rebecca-core config`
- `cargo nextest run -p rebecca-cli cli_output`
- `cargo nextest run --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
