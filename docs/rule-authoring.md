# Rule Authoring

Built-in Rebecca rules live under `crates/rebecca-rules/rules/windows/` as TOML
files. Keep each rule small, explicit, and easy to audit.

## Current Shape

- One file per rule.
- Stable env-variable templates only.
- No globbing or profile discovery yet.
- Prefer paths that Windows users recognize immediately.

## Required Fields

- `id`
- `platform`
- `category`
- `name`
- `safety_level`
- `delete_policy`
- `targets`
- `provenance`

## Target Guidance

- Use `template` for stable cache directories.
- Use `exact-path` only for fixed paths that do not vary by environment.
- Avoid rules that depend on profile enumeration until glob support exists.

## Safety Guidance

- `safe`: disposable caches, shader caches, regenerated browser data.
- `moderate`: developer caches, diagnostic artifacts, package caches.
- `risky` and above: only when the user impact is well understood.

## Verification

- `cargo nextest run -p rebecca-rules`
- `cargo nextest run --workspace`
- `cargo run -p rebecca-cli -- scan`

## Provenance

- Do not copy GPL rule definitions or code into the catalog.
- Document the source of each rule in `provenance.notes`.
