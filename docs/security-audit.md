---
title: "Rebecca Cleanup Safety Audit"
status: "active"
created: "2026-06-24"
last_updated: "2026-06-24"
---

# Rebecca Cleanup Safety Audit

This document describes Rebecca's current cleanup safety model. It is the
human-readable audit surface for destructive-operation boundaries, protected
data categories, rule governance, history/audit behavior, and known limitations.

Rebecca uses Mole as a safety-posture benchmark: prefer bounded cleanup, preview
before deleting, block sensitive data categories, and keep the safety model
auditable. Rebecca does not copy Mole implementation code or rule definitions.

## Executive Summary

Rebecca is a Windows-first cleanup CLI. Its main risk is unintended local data
loss from cleanup targets that are too broad, stale, or misclassified.

The current design is safety-first:

- `clean --dry-run` and real cleanup share the same planner.
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
  codes, issue matrices, and restore hints. It does not store file contents.

The largest remaining gap is protected-result preservation across history and
human output. Protected final paths are blocked by planning and execution, but
the audit surface still needs a dedicated round-trip story for blocked and
protected outcomes.

## Threat Surface

Rebecca's highest-risk areas are:

- path template expansion from rule TOML;
- glob and application-discovery target expansion;
- directory size scanning;
- Recycle Bin execution;
- history and scan-cache persistence;
- future rule catalog expansion.

The current product intentionally excludes permanent deletion by default,
administrator auto-elevation, uninstall flows, optimize flows, disk mapping,
and broad orphan-data cleanup.

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
  under `Default` or bounded `Profile *` profiles;
- Firefox `cache2` and `startupCache` directories;
- Electron/VS Code cache directories such as `Cache`, `Code Cache`,
  `GPUCache`, and `CachedData`;
- JetBrains product `caches` directories;
- Cargo cache subdirectories under `registry` and `git`;
- pip and npm cache directories;
- Windows Error Reporting `ReportArchive` and `ReportQueue`;
- Steam client web cache directories.

The allowlist exists so protected categories can be conservative without
blocking known rebuildable caches.

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
| Application durable data | Steam `userdata`, `steamapps\common`, `steamapps\workshop`, `steamapps\compatdata`, browser-like durable storage roots |

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

History must not store file contents, credentials, tokens, browser databases, or
arbitrary child-file listings. Scan-cache records are rebuildable optimization
data, not an audit log.

## Current Verification Coverage

Focused coverage currently includes:

- `crates/rebecca-core/tests/safety_policy.rs` for path validation, protected
  categories, allowlisted maintenance paths, and Rebecca-owned storage
  protection;
- `crates/rebecca-core/tests/planner.rs` for rule selection, target expansion,
  scan-cache behavior, protected storage blocking, protected category blocking,
  and Steam target behavior;
- `crates/rebecca-core/tests/executor_contract.rs` for executor status updates,
  backend failure handling, and execution-time revalidation;
- `crates/rebecca-core/tests/model_contract.rs` for plan serialization and
  backwards compatibility;
- `crates/rebecca-cli/tests/cli_clean.rs`, `cli_scan.rs`, and `cli_history.rs`
  for user-facing and JSON contract behavior.

Recent targeted verification for this audit baseline:

- `cargo nextest run -p rebecca-core --test safety_policy`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-core --test executor_contract`
- `cargo nextest run -p rebecca-rules`

## Known Limitations And Planned Hardening

- Protected-result preservation across history and human output is still being
  deepened.
- Protected category coverage is conservative but not exhaustive for all Windows
  applications.
- Release artifact attestations, installer integrity, and supply-chain signals
  are outside the current cleanup-safety slice.
- Rebecca does not yet have a dedicated public `SECURITY.md` vulnerability
  reporting policy.

These gaps are tracked by
`docs/plans/2026-06-24-018-refactor-mole-parity-safety-governance-plan.md`.
