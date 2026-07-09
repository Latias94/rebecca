---
type: "Verification Evidence"
title: "Full disk governance and MFT productization"
description: "Verification evidence for the full disk governance and MFT productization refactor."
timestamp: 2026-07-09T10:54:56Z
tags: ["rebecca", "disk-governance", "mft", "release-gate", "dogfood"]
status: "passed"
related_plan: "docs\\plans\\2026-07-09-003-refactor-full-disk-governance-mft-plan.md"
git_branch: "main"
---

# Result

The implementation satisfied the plan's verification contract on Windows.
The final tree passed formatting, clippy, full nextest, release gate self-test, disk-governance dogfood self-test, diff whitespace check, and dependency policy audit.

# Evidence

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `git diff --check` | Passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Passed |
| `cargo nextest run --workspace --locked --no-fail-fast` | Passed, 993 tests passed |
| `pwsh -File scripts\\release\\run-release-gates.ps1 -SelfTest` | Passed; report under `target\\release-gates\\20260709-110740-47036\\release-gates-report.json` |
| `pwsh -File scripts\\dogfood\\run-disk-governance-dogfood.ps1 -SelfTest` | Passed; report under `target\\disk-governance-dogfood\\20260709-110740-49232\\disk-governance-dogfood-report.json` |
| `cargo deny check` | Passed; duplicate `hashbrown` warning remains non-failing and transitive |

# Notes

- Review-only workspace insights intentionally do not produce cleanup commands, purge targets, basket items, or execution candidates.
- Cleanable secondary evidence can retain `suggested_command` so human and machine consumers can display safe preview commands even when the primary advice is review-only.
- `inspect drive` is a guided wrapper over the disk-map engine, so it does not create a second scanner path.
- Volume context is nullable by design; missing volume context does not block inventory.
- The duplicate `hashbrown` warning comes through current `ratatui` and `jsonschema` dependency edges and does not indicate a security or license failure.

# Citations

- [Plan](../../../plans/2026-07-09-003-refactor-full-disk-governance-mft-plan.md)
- [Progress completion](../progress/2026-07-09T105456Z-full-disk-governance-complete.md)
