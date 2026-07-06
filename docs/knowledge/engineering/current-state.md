---
type: "Current State"
title: "Current Engineering State"
description: "Short durable summary of the active engineering state."
tags: ["engineering-memory"]
timestamp: 2026-06-28T00:00:00+08:00
status: "active"
---

# Current State

- Goal: Rebecca is now a cross-platform cleanup CLI with Windows-specific enhancements. Project artifact purge, Rebecca cache purge, inspect workflows, Linux cleanup rules, Linux doctor diagnostics, and the shared recoverable-trash backend are portable, while Windows app-leftover discovery, Windows-native scan, and NTFS/MFT remain explicit Windows capabilities. Rebecca has stronger project-artifact purge coverage, broad Windows and Linux developer-cache coverage, Windows maintenance-cache coverage, Linux package-manager archive coverage, domestic desktop-app cache coverage, and the existing safety model. External rule families need provenance notes that preserve upstream project, repository or file path, license, and revision details, and the canonical reference index now lives at `docs/knowledge/engineering/conventions/rule-sources.md`. `null-e` is now the preferred batch reference for developer-cache families; `windows-cleaner-cli` remains the preferred batch reference for Windows maintenance caches; BleachBit remains a behavior reference for application/system cleaners; and Bulk Crap Uninstaller is the preferred uninstall-leftover reference.
- Branch: main
- Last verified: Cross-platform cleanup execution refactor passed on 2026-07-06 with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace --locked --no-fail-fast` (769 passed), `git diff --check`, `cargo deny check`, and `cargo run -p rebecca --locked -- catalog validate --format json`.
- Done: Initialized `docs/knowledge/engineering`; collected Mole, windows-cleaner-cli, CrunchyCleaner, BleachBit, and Bulk Crap Uninstaller references; created platform, architecture, workspace, privilege, registry, scan-engine, deletion/recovery, rule-provenance, and local-state ADRs; initialized the `rebecca` Cargo workspace and crates; created the Windows cleanup MVP implementation plan and later closed it as complete; implemented the MVP cleanup loop across core, rules, Windows adapter, CLI, history, and tests; changed Windows recoverable trash execution to preserve directory targets and move their direct child entries; added planner and CLI regression tests for overlapping templates; externalized the built-in rules into TOML files under `crates/rebecca-rules/rules/windows/` with schema and provenance validation; added rule authoring guidance and expanded the built-in catalog to Chrome, Edge, Brave, DirectX shader cache, pip, VS Code, and Windows Error Reporting; added `glob-template` rule targets with bounded wildcard discovery; added Firefox profile cache and Windows thumbnail/icon cache rules; added reusable `RuleSelection` semantics and `scan --category/rule` filtering with grouped human output; expanded Chrome, Edge, and Brave cache rules to cover explicit `Default` targets plus bounded `Profile *` Cache, Code Cache, and GPUCache targets; improved human cleanup plan output with human-readable byte sizes, largest estimated targets, and status-grouped target details; added target-level planner progress events, `indicatif` CLI progress reporting, and `clean --no-progress`; added JetBrains IDE cache coverage under `%LOCALAPPDATA%\\JetBrains\\<product><version>\\caches`; added Cargo dependency cache coverage for `%CARGO_HOME%` and default `%USERPROFILE%\\.cargo` cache subdirectories; added Discord cache coverage under `%APPDATA%\\discord`, `%APPDATA%\\discordptb`, and `%APPDATA%\\discordcanary`; added file-level scan progress events, scan cancellation tokens, and Ctrl+C cancellation wiring; added structured scan reports, scan failure classification, and a Criterion scan baseline benchmark; added conservative Steam client web cache coverage under `%LOCALAPPDATA%\\Steam\\htmlcache\\Default`; added Steam installation/library discovery, Steam-aware rule target kinds, and a `doctor steam` CLI command; added conservative Steam install download cache, Steam install library cache, and library downloading/temp cache rules with matching planner, CLI, and documentation coverage; added deterministic Steam CLI regression coverage with a debug-only discovery override so `doctor steam` and `clean --dry-run` tests can exercise Steam discovery without relying on the host machine; added a CLI regression that asserts `windows.npm-cache` is skipped without `--allow-moderate` and allowed with `--allow-moderate`; expanded Node package-manager cache coverage to corrected npm Local AppData plus Roaming fallback, pnpm, Yarn, Bun, and Corepack; added NuGet package-cache coverage for `%USERPROFILE%\\.nuget\\packages` plus `%LOCALAPPDATA%\\NuGet` cache subdirectories; added Gradle cache coverage for `%USERPROFILE%\\.gradle\\caches` and `%USERPROFILE%\\.gradle\\notifications`; added Maven local repository cache coverage for `%USERPROFILE%\\.m2\\repository`; added uv and Poetry package-cache coverage for `%LOCALAPPDATA%\\uv\\cache`, `%LOCALAPPDATA%\\pypoetry\\Cache\\cache`, and `%LOCALAPPDATA%\\pypoetry\\Cache\\artifacts`; added Go build/module cache coverage for `%LOCALAPPDATA%\\go-build` and `%USERPROFILE%\\go\\pkg\\mod`; added Rustup package-cache coverage for `%RUSTUP_HOME%\\downloads`, `%RUSTUP_HOME%\\tmp`, and default `%USERPROFILE%\\.rustup` cache leaves; added Conda package-cache coverage for `%USERPROFILE%\\.conda\\pkgs` and common Windows distribution roots; added planner and CLI regressions covering `allow_risky` with temporary risky rules so the safety contract is explicit in both layers; added README examples for `--allow-moderate` and `--allow-risky` to make the opt-in shape discoverable; hardened Steam discovery so unreadable `libraryfolders.vdf` falls back to the install root, merged Steam libraryfolders from both `config` and `steamapps`, added CLI/core regressions for the dual-source and fallback behavior, unified restore-hint rendering across CLI human outputs, locked Steam JSON restore-hint contracts, preserved restore hints through core plan serialization, defined the scan-cache v1 contract, wired `clean --scan-cache` through a planner context for explicit regular-file target estimate reuse, added scan-cache hit/miss/write-skip observability, added bounded directory scan-cache reuse with freshness-bounded invalidation, added human-only scan-cache activity summaries, introduced `ScanCachePolicy` so directory freshness is governed by a planner-carried policy with default behavior unchanged, exposed the policy through config so `config.toml` can override directory freshness without changing the cache record format, improved `cache purge` observability with lifecycle, cache-dir existence/preservation, entry-status counts, and a stable skipped/failed issue matrix in core reports and human output, extended the issue-matrix pattern to cleanup plans with stable target reason codes, summary aggregation, human/JSON output, and legacy plan/history JSON compatibility, extracted dedicated projection types for history, cache purge, and clean human output, and centralized the config schema, path precedence, lifecycle ownership, cache purge boundary, privacy, and migration rules in `docs/configuration.md`.
- Cleanup-core maturity baseline: Planner scans and cleanup execution now use bounded shared parallelism; scan-cache records are treated as rebuildable optimization data with explicit miss, expiry, corruption, pruning, provenance, and human-summary reporting; execution revalidation classifies protected targets as `safety-policy-blocked`, disappeared targets as `execution-target-missing`, and backend permission/IO errors as `execution-failed`, with history and CLI replay coverage. CLI workflows now share an explicit output contract, a single runtime/cancellation owner, dedicated human renderers, discovery diagnostics for project-artifact partial failures, canonical read-only `inspect artifacts`, portable `portable.project-artifact-*` IDs, and core-owned recoverable trash execution.
- Cleanup performance/API hardening baseline: Command-aware CLI API contracts now drive unified `rebecca.cli.v1` success and error envelopes; `cache purge --yes` defaults to recoverable deletion and requires `--permanent` for direct removal; project-artifact reclaim limits stop measurement once ranked trim-eligible candidates satisfy the target; parallel project-artifact and app-leftover measurement no longer buffers file-level progress for replay; `inspect space` and `inspect lint` use bounded top sections; scan traversal reuses walker entry type metadata where possible; ordinary rule planning now stages candidates and measures eligible targets through bounded scan-pool parallelism; `history --limit` uses a bounded tail reader; and `crates/rebecca-core/benches/perf_matrix.rs` now provides product-level scan/cache/delete/rule-plan scenarios with a JSON report generated by `scripts/perf/run-benchmark-matrix.ps1`.
- Latest completed: Linux cleanup adaptation expanded the built-in Linux catalog across developer, browser, desktop-app, Steam, thumbnail, and package-manager cache rules; `catalog --platform linux` exposes those rules; `scan` defaults to current-host rules; and `doctor permissions` plus `doctor active-processes` now report Linux capability through `/proc` diagnostics.
- In progress: None.
- Blocked: None.
- Next action: Finish Linux-facing docs, skill guidance, and CI/dogfood smoke so the expanded Linux support stays durable.

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
- [Cleanup-core hardening plan](../../../docs/plans/2026-06-28-001-refactor-cleanup-core-hardening-plan.md)
- [Mole-parity completion review](verification/2026-06-24-mole-parity-completion-review.md)
- [Rebecca Cleanup Safety Audit](../../../docs/security-audit.md)
- [CLI API v1 contract](../../../docs/api/cli/v1/README.md)
- [CLI API contract plan](../../../docs/plans/2026-06-28-003-refactor-cli-api-contract-plan.md)
- [NTFS live volume index plan](../../../docs/plans/2026-07-02-002-feat-ntfs-live-volume-index-plan.md)
- [NTFS sequential MFT reader plan](../../../docs/plans/2026-07-02-003-feat-ntfs-sequential-mft-reader-plan.md)
- [NTFS parser core dependency gate plan](../../../docs/plans/2026-07-02-004-refactor-ntfs-parser-core-dependency-gate-plan.md)
- [NTFS index allocation stream reader plan](../../../docs/plans/2026-07-02-005-refactor-ntfs-index-allocation-stream-plan.md)
- [NTFS targeted traversal plan](../../../docs/plans/2026-07-02-007-refactor-ntfs-targeted-traversal-plan.md)
- [Cleanup workflow architecture refactor plan](../../../docs/plans/2026-06-30-001-refactor-cleanup-workflow-architecture-plan.md)
- [Cleanup performance/API hardening plan](../../../docs/plans/2026-06-30-003-refactor-performance-api-hardening-plan.md)
- [Cleanup evidence and advice refactor plan](../../../docs/plans/2026-07-04-001-refactor-cleanup-evidence-and-advice-plan.md)
- [NTFS physical usage provenance refactor plan](../../../docs/plans/2026-07-04-002-refactor-ntfs-physical-usage-provenance-plan.md)
- [NTFS live dogfood fixtures plan](../../../docs/plans/2026-07-04-003-feat-ntfs-live-dogfood-fixtures-plan.md)
- [Security policy](../../../SECURITY.md)
- [Release integrity plan](../../../docs/plans/2026-06-24-019-feat-release-integrity-and-distribution-plan.md)
- [Windows install verification plan](../../../docs/plans/2026-06-24-020-feat-windows-install-verification-plan.md)
- [Release guide](../../../docs/release.md)
- [Rule authoring guide](../../../docs/rule-authoring.md)
