---
type: "Current State"
title: "Current Engineering State"
description: "Short durable summary of the active engineering state."
tags: ["engineering-memory"]
timestamp: 2026-06-24T15:34:00Z
status: "active"
---

# Current State

- Goal: Release integrity baseline is the active follow-up after the completed Mole-like Windows cleanup safety goal. The first slice adds CI, GitHub Release packaging, SHA-256 checksums, build-provenance attestations, and user verification docs.
- Branch: feat/windows-cleanup-mvp
- Last verified: `pwsh -NoProfile -File scripts/release/build-release.ps1 -Tag v0.1.0 -OutDir target\release-smoke`, `pwsh -NoProfile -File scripts/release/write-checksums.ps1 -DistDir target\release-smoke`, local ZIP checksum comparison against `target\release-smoke\SHA256SUMS`, `cargo fmt --all -- --check`, `cargo nextest run --workspace` (242 passing tests), `cargo clippy --workspace --all-targets -- -D warnings`, and `git diff --check` passed on 2026-06-24 for the release integrity baseline.
- Done: Initialized `docs/knowledge/engineering`; collected Mole, windows-cleaner-cli, CrunchyCleaner, BleachBit, and Bulk Crap Uninstaller references; created platform, architecture, workspace, privilege, registry, scan-engine, deletion/recovery, rule-provenance, and local-state ADRs; initialized the `rebecca` Cargo workspace and crates; created the Windows cleanup MVP implementation plan and later closed it as complete; implemented the MVP cleanup loop across core, rules, Windows adapter, CLI, history, and tests; changed Windows Recycle Bin execution to preserve directory targets and move their direct child entries; added planner and CLI regression tests for overlapping templates; externalized the built-in rules into TOML files under `crates/rebecca-rules/rules/windows/` with schema and provenance validation; added rule authoring guidance and expanded the built-in catalog to Chrome, DirectX shader cache, pip, VS Code, and Windows Error Reporting; added `glob-template` rule targets with bounded wildcard discovery; added Firefox profile cache and Windows thumbnail/icon cache rules; added reusable `RuleSelection` semantics and `scan --category/rule` filtering with grouped human output; expanded Chrome and Edge cache rules to cover explicit `Default` targets plus bounded `Profile *` Cache, Code Cache, and GPUCache targets; improved human cleanup plan output with human-readable byte sizes, largest estimated targets, and status-grouped target details; added target-level planner progress events, `indicatif` CLI progress reporting, and `clean --no-progress`; added JetBrains IDE cache coverage under `%LOCALAPPDATA%\\JetBrains\\<product><version>\\caches`; added Cargo dependency cache coverage for `%CARGO_HOME%` and default `%USERPROFILE%\\.cargo` cache subdirectories; added Discord cache coverage under `%APPDATA%\\discord`, `%APPDATA%\\discordptb`, and `%APPDATA%\\discordcanary`; added file-level scan progress events, scan cancellation tokens, and Ctrl+C cancellation wiring; added structured scan reports, scan failure classification, and a Criterion scan baseline benchmark; added conservative Steam client web cache coverage under `%LOCALAPPDATA%\\Steam\\htmlcache\\Default`; added Steam installation/library discovery, Steam-aware rule target kinds, and a `doctor steam` CLI command; added conservative Steam install download cache, Steam install library cache, and library downloading/temp cache rules with matching planner, CLI, and documentation coverage; added deterministic Steam CLI regression coverage with a debug-only discovery override so `doctor steam` and `clean --dry-run` tests can exercise Steam discovery without relying on the host machine; added a CLI regression that asserts `windows.npm-cache` is skipped without `--allow-moderate` and allowed with `--allow-moderate`; added planner and CLI regressions covering `allow_risky` with temporary risky rules so the safety contract is explicit in both layers; added README examples for `--allow-moderate` and `--allow-risky` to make the opt-in shape discoverable; hardened Steam discovery so unreadable `libraryfolders.vdf` falls back to the install root, merged Steam libraryfolders from both `config` and `steamapps`, added CLI/core regressions for the dual-source and fallback behavior, unified restore-hint rendering across CLI human outputs, locked Steam JSON restore-hint contracts, preserved restore hints through core plan serialization, defined the scan-cache v1 contract, wired `clean --scan-cache` through a planner context for explicit regular-file target estimate reuse, added scan-cache hit/miss/write-skip observability, added bounded directory scan-cache reuse with freshness-bounded invalidation, added human-only scan-cache activity summaries, introduced `ScanCachePolicy` so directory freshness is governed by a planner-carried policy with default behavior unchanged, exposed the policy through config so `config.toml` can override directory freshness without changing the cache record format, improved `cache purge` observability with lifecycle, cache-dir existence/preservation, entry-status counts, and a stable skipped/failed issue matrix in core reports and human output, extended the issue-matrix pattern to cleanup plans with stable target reason codes, summary aggregation, human/JSON output, and legacy plan/history JSON compatibility, extracted dedicated projection types for history, cache purge, and clean human output, and centralized the config schema, path precedence, lifecycle ownership, cache purge boundary, privacy, and migration rules in `docs/configuration.md`.
- Latest completed: Added `docs/plans/2026-06-24-019-feat-release-integrity-and-distribution-plan.md`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, PowerShell release packaging/checksum scripts, `docs/release.md`, and release-integrity links in README, SECURITY, and the safety audit.
- In progress: Distribution hardening beyond the first GitHub Release baseline remains open: installer UX, package-manager manifests, SBOM generation, Windows ARM64 artifacts, and fully pinned GitHub Action SHAs.
- Blocked: None.
- Next action: After committing the release baseline, choose the next distribution slice: installer/checksum verification, action SHA pinning plus dependency update automation, or package-manager publishing.

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
- [Configuration and local state contract](../../../docs/configuration.md)
- [Controlled scan cache plan](../../../docs/plans/2026-06-24-009-feat-controlled-scan-cache-plan.md)
- [Scan cache observability plan](../../../docs/plans/2026-06-24-010-feat-scan-cache-observability-plan.md)
- [Bounded directory scan cache reuse plan](../../../docs/plans/2026-06-24-011-feat-bounded-directory-scan-cache-reuse-plan.md)
- [Scan cache human summary plan](../../../docs/plans/2026-06-24-012-feat-scan-cache-human-summary-plan.md)
- [Scan cache policy plan](../../../docs/plans/2026-06-24-013-refactor-scan-cache-policy-plan.md)
- [Configurable scan cache policy plan](../../../docs/plans/2026-06-24-014-feat-configurable-scan-cache-policy-plan.md)
- [Cache purge observability plan](../../../docs/plans/2026-06-24-015-feat-cache-purge-observability-plan.md)
- [Cache purge issue matrix plan](../../../docs/plans/2026-06-24-016-feat-cache-purge-issue-matrix-plan.md)
- [Cleanup plan issue matrix plan](../../../docs/plans/2026-06-24-017-feat-cleanup-plan-issue-matrix-plan.md)
- [Mole-parity safety governance roadmap](../../../docs/plans/2026-06-24-018-refactor-mole-parity-safety-governance-plan.md)
- [Mole-parity completion review](verification/2026-06-24-mole-parity-completion-review.md)
- [Rebecca Cleanup Safety Audit](../../../docs/security-audit.md)
- [Security policy](../../../SECURITY.md)
- [Release integrity plan](../../../docs/plans/2026-06-24-019-feat-release-integrity-and-distribution-plan.md)
- [Release guide](../../../docs/release.md)
- [Rule authoring guide](../../../docs/rule-authoring.md)
