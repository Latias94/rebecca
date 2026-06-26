---
title: "Rebecca Cleanup Safety Audit"
status: "active"
created: "2026-06-24"
last_updated: "2026-06-26"
---

# Rebecca Cleanup Safety Audit

This document describes Rebecca's current cleanup safety model. It is the
human-readable audit surface for destructive-operation boundaries, protected
data categories, rule governance, history/audit behavior, and known limitations.

Rebecca uses Mole as a safety-posture benchmark: prefer bounded cleanup, preview
before deleting, block sensitive data categories, and keep the safety model
auditable. Rebecca does not copy Mole implementation code or rule definitions.
Security reporting guidance lives in the repository root `SECURITY.md`.

## Executive Summary

Rebecca is a Windows-first cleanup CLI. Its main risk is unintended local data
loss from cleanup targets that are too broad, stale, or misclassified.

The current design is safety-first:

- `clean --dry-run` and real cleanup share the same planner.
- `apps scan` and `apps clean` share the planner through an app-leftovers
  workflow that is separate from full uninstall behavior.
- The planner validates paths through `rebecca-core::protection::ProtectionPolicy`.
- The executor revalidates executable targets through the same policy before a
  backend delete runs.
- Empty paths, traversal, filesystem roots, critical Windows paths, user profile
  roots, protected categories, Rebecca-owned storage, and existing reparse-like
  paths are blocked.
- Built-in rules are typed TOML, Windows-scoped, project-owned, and validated
  against the shared protection model at load time.
- Default execution moves files, or direct child entries of directory targets,
  to the Windows Recycle Bin.
- History stores request metadata, target paths, byte counts, statuses, reason
  codes, issue matrices, target-scoped issue reasons, and restore hints. It
  does not store file contents.

The core destructive-operation boundaries, execution revalidation, catalog
target-shape validation, protected-result audit round-trip, and first
guardrailed catalog expansion batch are in place. Future cleanup families must
continue to prove they stay inside those boundaries, but no remaining
cleanup-system safety gap blocks the current Mole-like Windows-first scope.
Release integrity is tracked separately from cleanup-runtime safety: the
repository now has a GitHub Release workflow, checksum generation, and
build-provenance attestation path for official artifacts.

## Threat Surface

Rebecca's highest-risk areas are:

- path template expansion from rule TOML;
- glob and application-discovery target expansion;
- read-only installed-app inventory from Windows uninstall registry locations;
- directory size scanning;
- Recycle Bin execution;
- history and scan-cache persistence;
- future rule catalog expansion.

The current product intentionally excludes permanent deletion by default,
administrator auto-elevation, vendor uninstaller execution, registry removal
flows, optimize flows, disk mapping, and broad orphan-data cleanup.

## Destructive Operation Boundaries

Cleanup planning routes target paths through `ProtectionPolicy`, with
`crates/rebecca-core/src/safety.rs` preserving the older compatibility wrapper.

The policy blocks:

- empty paths;
- path traversal segments such as `..`;
- filesystem roots including drive roots and UNC share roots;
- critical Windows paths such as `C:\Windows`, `C:\Program Files`,
  `C:\ProgramData`, `$Recycle.Bin`, `C:\Recovery`, and
  `C:\System Volume Information`;
- Windows user profile roots such as `C:\Users\Alice`;
- Rebecca-owned config, state, history, and cache paths from
  `AppPaths::storage_entries()`;
- existing symlinks, junctions, and other reparse-like paths through the safety
  wrapper;
- protected data categories listed below.

Protected targets are blocked before filesystem metadata is required. This means
a synthetic rule that points at browser history or Rebecca-owned storage is
blocked even if that target does not exist on the current machine.

## Maintenance Allowlist

Rebecca allows bounded maintenance targets that correspond to current built-in
rules. These are narrow subpaths, not broad app roots:

- user temp directories;
- Chromium-family cache directories: `Cache`, `Code Cache`, and `GPUCache`
  under `Default` or bounded `Profile *` profiles for Chrome, Edge, and Brave;
- Firefox `cache2` and `startupCache` directories;
- Electron/VS Code cache directories such as `Cache`, `Code Cache`,
  `GPUCache`, and `CachedData` for explicitly allowlisted app roots including
  Discord and Slack;
- JetBrains product `caches` directories;
- Cargo cache subdirectories under `registry` and `git`;
- pip, uv, Poetry package-cache, Conda package-cache, Go build/module, Cargo,
  rustup, npm, pnpm,
  Yarn, Bun, Corepack, NuGet, Gradle, and Maven cache directories;
- Windows Error Reporting `ReportArchive` and `ReportQueue`;
- Steam client web cache directories.
- app-leftover cache directories derived from discovered installed apps, limited
  to `Cache`, `Code Cache`, `GPUCache`, and `CachedData` under
  `AppData\Local`, `AppData\Roaming`, or `AppData\LocalLow`.

The allowlist exists so protected categories can be conservative without
blocking known rebuildable caches.

## App Leftovers Boundary

The app-leftovers workflow is discovery-assisted cleanup, not an uninstaller.
On Windows, Rebecca reads uninstall inventory and install hints from registry
locations in read-only mode. Missing keys, unreadable entries, empty display
names, and system-component entries are skipped.

Inventory records are used only to derive user-scoped leftover cache paths from
the app display name. The workflow then applies the same protection policy,
directory scan, issue matrix, execution revalidation, Recycle Bin backend, and
history model as ordinary cleanup. It does not write registry data, remove
uninstall metadata, execute vendor uninstallers, kill app processes, or delete
system-owned install roots.

## Protected Categories

`ProtectionPolicy` currently blocks these categories:

| Category | Examples |
|----------|----------|
| Credentials | Microsoft Credentials/Protect/Crypto/Vault, `.ssh`, `.gnupg`, 1Password, Bitwarden, Cargo `credentials.toml` |
| VPN/proxy state | Clash, Clash Verge, Tailscale, WireGuard, V2Ray, Shadowsocks, sing-box |
| AI/coding durable state | `.codex`, `.claude`, `.cursor`, `.ollama`, Claude, Cursor, Ollama, ChatGPT, VS Code `User` |
| Browser private data | Chromium `History`, `Cookies`, `Login Data`, `Web Data`, `Local Storage`, `IndexedDB`, `Service Worker`, `Network`; Firefox cookies/history/login databases |
| Cloud-synced data | OneDrive, iCloud Drive, iCloud Photos, Dropbox, Google Drive, Box, MEGA |
| Container/VM runtime state | Docker, Docker Desktop, Podman, Rancher Desktop, WSL config, `.docker`, `.podman`, `.kube` |
| Startup automation | Windows Startup folder paths |
| Application durable data | Steam `userdata`, `steamapps\common`, `steamapps\workshop`, `steamapps\compatdata`, Conda environments, browser-like durable storage roots |

These categories are intentionally conservative. False negatives where Rebecca
refuses to clean a path are acceptable safety outcomes; false positives that
delete durable user data are not.

## Rule Catalog Governance

Built-in rules live under `crates/rebecca-rules/rules/windows/` and are embedded
from TOML files. The catalog loader and validators enforce:

- one typed `RuleDefinition` per file;
- valid TOML with unknown fields rejected;
- non-empty rule id, category, name, provenance, and target path;
- unique rule ids and target specs;
- Windows platform and `windows.` id prefix for built-ins;
- non-empty restore hints;
- owned source and `project-owned` provenance license.
- target shapes that overlap protected categories or unallowlisted Steam
  install/library relative paths.

Rule authoring guidance lives in `docs/rule-authoring.md`. Reference projects
under `repo-ref/` are research inputs only; their GPL code and rules are not
copied into Rebecca.

## Execution Model

The current Windows backend moves allowed file targets to the Recycle Bin. For
directory targets, it preserves the target directory and moves direct child
entries. This keeps app-created cache directories in place while clearing their
contents.

Current limitations:

- the backend does not yet classify all filesystem failures into rich safety
  categories;
- non-Windows execution is unavailable and returns a platform error.

## Dry Run, History, And Audit Data

Dry-run is the primary operator contract. JSON output is stable and additive for
current fields unless a future contract version exists.

History is append-only JSONL. It records:

- cleanup request metadata;
- summary counts and byte counts;
- target paths;
- statuses;
- stable reason codes;
- issue matrices;
- restore hints.

Human history output replays issue targets with status, stable reason code,
rule id, path, target-scoped reason, and restore hint when present. It does not
expand arbitrary child-file listings.

History must not store file contents, credentials, tokens, browser databases, or
arbitrary child-file listings. Scan-cache records are rebuildable optimization
data, not an audit log.

## Release Integrity

Rebecca treats release trust as security-sensitive because users run a local
cleanup binary against personal data. The current release hardening path uses:

- a Windows GitHub Actions CI quality gate with read-only repository
  permissions;
- a tag-triggered release workflow for Windows x86_64 MSVC artifacts;
- PowerShell packaging that includes `rebecca.exe`, README, security policy,
  release guide, install script, and this safety audit;
- `SHA256SUMS` generated from final downloadable artifacts;
- GitHub build-provenance attestations for release assets;
- a PowerShell installer that verifies `SHA256SUMS` before extraction and can
  require GitHub CLI attestation verification;
- user verification guidance in `docs/release.md`.

This distribution layer does not change cleanup behavior. It gives users and
maintainers a way to verify that a downloaded artifact matches the release
checksum and, when GitHub CLI attestation verification is available, that the
artifact came from the expected GitHub Actions build path.

## Current Verification Coverage

Focused coverage currently includes:

- `crates/rebecca-core/tests/safety_policy.rs` for path validation, protected
  categories, allowlisted maintenance paths, and Rebecca-owned storage
  protection;
- `crates/rebecca-core/tests/planner.rs` for rule selection, target expansion,
  scan-cache behavior, protected storage blocking, protected category blocking,
  Steam target behavior, and app-leftover planning;
- `crates/rebecca-core/tests/executor_contract.rs` for executor status updates,
  backend failure handling, and execution-time revalidation;
- `crates/rebecca-core/tests/model_contract.rs` for plan serialization,
  protected issue contracts, and backwards compatibility;
- `crates/rebecca-core/tests/history.rs` for append/load history JSONL
  round-trips, including protected issue reason preservation;
- `crates/rebecca-windows/tests/apps_inventory.rs` for best-effort Windows app
  inventory discovery;
- `crates/rebecca-cli/tests/cli_clean.rs`, `cli_scan.rs`, `cli_apps.rs`, and
  `cli_history.rs`
  for user-facing and JSON contract behavior, including protected issue target
  replay.
- `crates/rebecca-cli/tests/cli_purge.rs` for project-artifact purge output,
  selector catalog listing, and recent-artifact messaging.
- `crates/rebecca-cli/tests/cli_scan.rs` for builtin rule listing and scan
  filter coverage.
- `crates/rebecca-core/tests/project_artifacts.rs` for project-artifact
  discovery, context-sensitive cache detection, selector filtering, and recent
  artifact skipping.

Recent targeted verification for this audit baseline:

- `cargo nextest run -p rebecca-core --test safety_policy`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-core --test executor_contract`
- `cargo nextest run -p rebecca-core --test model_contract`
- `cargo nextest run -p rebecca-core --test history`
- `cargo nextest run -p rebecca-cli --test cli_scan`
- `cargo nextest run -p rebecca-cli --test cli_purge`
- `cargo nextest run -p rebecca-cli --test cli_apps`
- `cargo nextest run -p rebecca-cli --test scan`
- `cargo nextest run -p rebecca-cli --test cli_clean`
- `cargo nextest run -p rebecca-cli --test cli_history`
- `cargo nextest run -p rebecca-core --test project_artifacts`
- `cargo nextest run -p rebecca-windows`
- `cargo nextest run -p rebecca-rules`

## Known Limitations And Planned Hardening

- Future cleanup-rule expansion must stay batch-sized and include
  family-specific unsafe-near-miss tests, CLI contract coverage, and audit
  updates before new rules are considered complete.
- `rebecca purge --list-artifacts` is the supported scan-free selector catalog
  surface; new project-artifact selectors should keep it and the JSON catalog in
  sync.
- Protected category coverage is conservative but not exhaustive for all Windows
  applications.
- The release workflow has not yet been exercised by a public version tag in
  this repository.
- MSI/MSIX installer UX, package-manager publishing, SBOM generation, Windows
  ARM64 artifacts, and fully pinned GitHub Action SHAs remain distribution-layer
  follow-up work beyond the current cleanup-safety slice.

The U8 completion review is recorded in
`docs/knowledge/engineering/verification/2026-06-24-mole-parity-completion-review.md`.
