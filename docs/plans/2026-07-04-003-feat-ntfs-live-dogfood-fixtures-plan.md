---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
title: "NTFS Live Dogfood Fixtures"
date: 2026-07-04
origin: ce-plan-bootstrap
---

# NTFS Live Dogfood Fixtures

## Goal Capsule

Rebecca already has parser-level NTFS correctness coverage and a backend comparison report. The next gap is repeatable live NTFS evidence for physical-usage semantics that only a real filesystem exposes: hardlinks, sparse allocation, compression, large flat directories, and fragmentation candidates. This plan adds a local fixture dogfood wrapper that creates those shapes under `target/`, runs the existing inspect-map backend report, and records a manifest describing which filesystem capabilities were actually available.

## Product Contract

- R1. The fixture dogfood command creates deterministic local sample trees without deleting user data or scanning implicit drive roots.
- R2. The fixture includes hardlink, sparse-file, compressed-file, large-directory, nested-directory, and fragmentation-candidate sections when the host filesystem supports them.
- R3. Host capability gaps are recorded as manifest caveats, not hidden or treated as success evidence.
- R4. The command reuses `scripts/dogfood/run-inspect-map-report.ps1` so backend comparison semantics stay centralized.
- R5. Generated fixture trees and reports stay under `target/` by default and are not committed.

## Planning Contract

- Keep this as script/docs work. Do not change cleanup deletion authority or parser semantics in this slice.
- The script must be safe on Windows NTFS and graceful on unsupported filesystems.
- Prefer deterministic small fixtures over large disk use.
- Use PowerShell APIs and existing repo script conventions; avoid long inline command strings in docs.

## Implementation Units

### U1. Add fixture dogfood wrapper

- **Files:** `scripts/dogfood/run-ntfs-fixture-dogfood.ps1`, `scripts/dogfood/README.md`
- **Approach:** Create a timestamped fixture root under `target/ntfs-dogfood-fixtures/` unless `-FixtureRoot` is supplied. Generate hardlink aliases, sparse allocation sample, compressed sample, large flat directory files, nested files, and fragmentation candidates. Write `ntfs-fixture-manifest.json` with section status, expected logical bytes, expected hardlink path count, and caveats. Then call the canonical inspect-map report script against the fixture root.
- **Test scenarios:**
  - `-SelfTest` creates a temporary target fixture and verifies the manifest plus expected section names without invoking the full CLI report.
  - Unsupported sparse/compression/hardlink operations become manifest caveats instead of unhandled exceptions.
  - The wrapper refuses a fixture root outside the repo when the path is not explicit.
- **Verification:** `pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 -SelfTest`.

### U2. Document live evidence workflow

- **Files:** `docs/performance/perf-matrix.md`, `docs/release.md`, `docs/knowledge/engineering/current-state.md`, `docs/knowledge/engineering/log.md`, `CHANGELOG.md`
- **Approach:** Document the fixture workflow as the preferred local hardlink/sparse/compressed NTFS dogfood path. Keep live output under `target/`. Explain that fragmentation is best-effort and must be interpreted as evidence, not a guaranteed disk-layout oracle.
- **Test scenarios:**
  - Docs point to the canonical fixture wrapper and inspect-map report script.
  - Changelog has a concise Unreleased entry.
- **Verification:** `git diff --check`.

## Verification Contract

Run:

```powershell
pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 -SelfTest
pwsh -File scripts/dogfood/run-ntfs-fixture-dogfood.ps1 -Repeat 1 -LargeFileCount 32 -Top 20 -DiagnosticLimit 0
pwsh -File scripts/dogfood/run-inspect-map-report.ps1 -SelfTest
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run -p rebecca --test cli_inspect --test cli_api
git diff --check
```

If the live fixture dogfood cannot exercise a host feature, record the manifest caveat and keep deterministic verification authoritative.

## Definition of Done

- The wrapper creates a fixture manifest and report directory from one command.
- The wrapper self-test passes without requiring permanent system changes.
- Docs and changelog describe the workflow and safety boundary.
- No generated fixture or report artifacts are staged.
- Verification Contract passes.
