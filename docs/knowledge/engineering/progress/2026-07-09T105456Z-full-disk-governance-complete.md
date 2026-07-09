---
type: "Work Progress"
title: "Full disk governance implementation complete"
description: "Completion record for the full disk governance and MFT productization plan."
timestamp: 2026-07-09T10:54:56Z
tags: ["rebecca", "ce-work", "mft", "disk-governance", "verification"]
status: "completed"
related_plan: "docs\\plans\\2026-07-09-003-refactor-full-disk-governance-mft-plan.md"
git_branch: "main"
---

# Summary

Implemented the full-disk governance and MFT productization plan.
Rebecca now has typed MFT fallback reasons, explicit fallback guidance, default NTFS feature enablement for the CLI build, guided `inspect drive`, volume context reporting, disk-usage semantics text, traversal-collected workspace insights, review-only cleanup advice with manual guidance, evidence-level preview commands, TUI review-only display boundaries, updated schemas/docs/skill guidance, and disk-governance dogfood release wiring.

# Details

- U1-U2: MFT fallback diagnostics are typed and propagated through provenance, human rendering, NDJSON backend fallback events, and dogfood reporting.
- U3: Disk-map reports carry nullable OS volume context and explain logical, allocated, and unique-byte semantics in human output.
- U4-U5: Workspace insights are collected from traversal evidence beyond bounded top entries and become `review-only` advice with no executable Rebecca command. Cleanable secondary evidence retains its own preview command so human output can still show the safe next Rebecca command without making review-only data executable.
- U6: `rebecca inspect drive <root>` wraps `inspect map` with guided full-drive defaults, cleanup advice on by default, and Windows NTFS/MFT as the default Windows backend.
- U7-U8: Release/dogfood scripts, API schemas, README, CLI API docs, changelog, and skill text now describe the guided full-disk workflow and review-only boundary.
- During final clippy, stale TUI fixture constructors were upgraded to the new `DiskMapReport` shape instead of keeping compatibility constructors.

# Verification

- `cargo fmt --all -- --check`
- `git diff --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo nextest run --workspace --locked --no-fail-fast` passed: 993 tests, 993 passed.
- `pwsh -File scripts\\release\\run-release-gates.ps1 -SelfTest`
- `pwsh -File scripts\\dogfood\\run-disk-governance-dogfood.ps1 -SelfTest`
- `cargo deny check` passed with a non-failing duplicate `hashbrown` warning from transitive `ratatui`/`jsonschema` dependencies.

# Citations

- [Plan](../../../plans/2026-07-09-003-refactor-full-disk-governance-mft-plan.md)
- [Verification evidence](../verification/2026-07-09-full-disk-governance-mft-productization.md)
