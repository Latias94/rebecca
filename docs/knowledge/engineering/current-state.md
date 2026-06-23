---
type: "Current State"
title: "Current Engineering State"
description: "Short durable summary of the active engineering state."
tags: ["engineering-memory"]
timestamp: 2026-06-23T15:00:00Z
status: "active"
---

# Current State

- Goal: The Windows cleanup MVP is complete, and the next active plan is Steam cleanup expansion.
- Branch: feat/windows-cleanup-mvp
- Last verified: `cargo fmt --all`, `cargo nextest run -p rebecca-rules`, `cargo nextest run -p rebecca-core --test planner`, and `cargo nextest run -p rebecca-cli --test cli_scan` all passed on 2026-06-24. `cargo nextest run --workspace` last ran 83 tests successfully before this follow-up. The latest benchmark run before this discovery work measured `scan_report_1024_files` at roughly 18.6-20.8 ms and `scan_report_with_file_progress_1024_files` at roughly 14.3-14.8 ms on the current machine.
- Done: Initialized `docs/knowledge/engineering`; collected Mole, windows-cleaner-cli, CrunchyCleaner, BleachBit, and Bulk Crap Uninstaller references; created platform, architecture, workspace, privilege, registry, scan-engine, deletion/recovery, rule-provenance, and local-state ADRs; initialized the `rebecca` Cargo workspace and crates; created the Windows cleanup MVP implementation plan and later closed it as complete; implemented the MVP cleanup loop across core, rules, Windows adapter, CLI, history, and tests; changed Windows Recycle Bin execution to preserve directory targets and move their direct child entries; added planner and CLI regression tests for overlapping templates; externalized the built-in rules into TOML files under `crates/rebecca-rules/rules/windows/` with schema and provenance validation; added rule authoring guidance and expanded the built-in catalog to Chrome, DirectX shader cache, pip, VS Code, and Windows Error Reporting; added `glob-template` rule targets with bounded wildcard discovery; added Firefox profile cache and Windows thumbnail/icon cache rules; added reusable `RuleSelection` semantics and `scan --category/rule` filtering with grouped human output; expanded Chrome and Edge cache rules to cover explicit `Default` targets plus bounded `Profile *` Cache, Code Cache, and GPUCache targets; improved human cleanup plan output with human-readable byte sizes, largest estimated targets, and status-grouped target details; added target-level planner progress events, `indicatif` CLI progress reporting, and `clean --no-progress`; added JetBrains IDE cache coverage under `%LOCALAPPDATA%\\JetBrains\\<product><version>\\caches`; added Cargo dependency cache coverage for `%CARGO_HOME%` and default `%USERPROFILE%\\.cargo` cache subdirectories; added Discord cache coverage under `%APPDATA%\\discord`, `%APPDATA%\\discordptb`, and `%APPDATA%\\discordcanary`; added file-level scan progress events, scan cancellation tokens, and Ctrl+C cancellation wiring; added structured scan reports, scan failure classification, and a Criterion scan baseline benchmark; added conservative Steam client web cache coverage under `%LOCALAPPDATA%\\Steam\\htmlcache\\Default`; added Steam installation/library discovery, Steam-aware rule target kinds, and a `doctor steam` CLI command; added conservative Steam install download cache, Steam install library cache, and library downloading/temp cache rules with matching planner, CLI, and documentation coverage.
- In progress: Steam cleanup expansion follow-up work.
- Blocked: None.
- Next action: Continue with the next approved follow-up slice after Steam cleanup expansion.

# Citations
- [Mole reference](../../../repo-ref/Mole/README.md)
- [windows-cleaner-cli reference](../../../repo-ref/windows-cleaner-cli/README.md)
- [CrunchyCleaner reference](../../../repo-ref/CrunchyCleaner/README.md)
- [BleachBit reference](../../../repo-ref/bleachbit/README.md)
- [Bulk Crap Uninstaller reference](../../../repo-ref/Bulk-Crap-Uninstaller/README.md)
- [Platform strategy decision](decisions/2026-06-23-windows-first-platform-strategy.md)
- [Session handoff](sessions/2026-06-23-documentation-foundation.md)
- [ADR index](../../../docs/adr/index.md)
- [Windows cleanup MVP plan](../../../docs/plans/2026-06-23-001-feat-windows-cleanup-mvp-plan.md)
- [README](../../../README.md)
- [Platform strategy ADR](../../../docs/adr/0001-platform-strategy.md)
- [Core runtime architecture ADR](../../../docs/adr/0002-core-runtime-architecture.md)
- [Workspace and module boundaries ADR](../../../docs/adr/0003-workspace-and-module-boundaries.md)
- [Windows privilege and registry model ADR](../../../docs/adr/0004-windows-privilege-and-registry-model.md)
- [Scan engine strategy ADR](../../../docs/adr/0005-scan-engine-strategy.md)
- [Deletion and recovery model ADR](../../../docs/adr/0006-deletion-and-recovery-model.md)
- [Rule catalog and license provenance ADR](../../../docs/adr/0007-rule-catalog-and-license-provenance.md)
- [Configuration and local state model ADR](../../../docs/adr/0008-configuration-and-local-state-model.md)
- [Rule authoring guide](../../../docs/rule-authoring.md)
