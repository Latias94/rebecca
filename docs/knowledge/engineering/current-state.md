---
type: "Current State"
title: "Current Engineering State"
description: "Short durable summary of the active engineering state."
tags: ["engineering-memory"]
timestamp: 2026-06-23T12:19:04Z
status: "active"
---

# Current State

- Goal: Continue from the implemented Windows cleanup MVP with TOML-backed browser/system/development cache rules, bounded glob/profile discovery, usable rule selection UX, and readable dry-run output.
- Branch: feat/windows-cleanup-mvp
- Last verified: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo nextest run --workspace` all passed on 2026-06-23. `cargo nextest run --workspace` ran 48 tests successfully. `cargo run -q -p rebecca-cli -- clean --dry-run --rule windows.chrome-cache --rule windows.edge-cache` showed human-readable byte totals, a largest-targets section, and status-grouped target details; on the current machine it reported 12 targets, 3 allowed, 9 skipped, and no blocked or failed targets.
- Done: Initialized `docs/knowledge/engineering`; collected Mole, windows-cleaner-cli, CrunchyCleaner, BleachBit, and Bulk Crap Uninstaller references; created platform, architecture, workspace, privilege, registry, scan-engine, deletion/recovery, rule-provenance, and local-state ADRs; initialized the `rebecca` Cargo workspace and crates; created the Windows cleanup MVP implementation plan; implemented the MVP cleanup loop across core, rules, Windows adapter, CLI, history, and tests; changed Windows Recycle Bin execution to preserve directory targets and move their direct child entries; added planner and CLI regression tests for overlapping templates; externalized the built-in rules into TOML files under `crates/rebecca-rules/rules/windows/` with schema and provenance validation; added rule authoring guidance and expanded the built-in catalog to Chrome, DirectX shader cache, pip, VS Code, and Windows Error Reporting; added `glob-template` rule targets with bounded wildcard discovery; added Firefox profile cache and Windows thumbnail/icon cache rules; added reusable `RuleSelection` semantics and `scan --category/--rule` filtering with grouped human output; expanded Chrome and Edge cache rules to cover explicit `Default` targets plus bounded `Profile *` Cache, Code Cache, and GPUCache targets; improved human cleanup plan output with human-readable byte sizes, largest estimated targets, and status-grouped target details.
- In progress: No active implementation task.
- Blocked: None.
- Next action: Consider a real scan-progress callback/progress-bar design if scans become slow, or add another small owned catalog batch such as Discord/Steam/JetBrains caches.

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
