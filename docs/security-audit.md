---
title: "Rebecca Cleanup Safety Audit"
status: "active"
created: "2026-06-24"
last_updated: "2026-06-28"
---

# Rebecca Cleanup Safety Audit

This document describes Rebecca's current cleanup safety model. It is the
human-readable audit surface for destructive-operation boundaries, protected
data categories, rule governance, history/audit behavior, and known limitations.

Rebecca uses Mole as a safety-posture benchmark: prefer bounded cleanup, preview
before deleting, block sensitive data categories, keep the safety model
auditable, and prune stale scan-cache data as rebuildable state. Rebecca does
not copy Mole implementation code or rule definitions. Rebecca keeps a stricter
non-interactive CLI contract than Mole's interactive cleanup flow: destructive
Rebecca cleanup commands preview by default and require `--yes` before moving
or deleting targets.
Security reporting guidance lives in the repository root `SECURITY.md`.

## Executive Summary

Rebecca is a Windows-first cleanup CLI. Its main risk is unintended local data
loss from cleanup targets that are too broad, stale, or misclassified.

The current design is safety-first:

- `clean` previews by default, `clean --dry-run` makes that preview explicit,
  and `clean --yes` shares the same planner before execution.
- `apps scan` and `apps clean` share the planner through an app-leftovers
  workflow that is separate from full uninstall behavior.
- The planner validates paths through `rebecca-core::protection::ProtectionPolicy`,
  which combines the auditable safety catalog with runtime-only overlap and
  filesystem checks.
- The executor revalidates executable targets through the same policy before a
  backend delete runs, then records deterministic outcomes: protected targets
  become `safety-policy-blocked`, disappeared targets become
  `execution-target-missing`, and backend permission or IO errors become
  `execution-failed`.
- Empty paths, traversal, filesystem roots, critical Windows paths, user profile
  roots, protected categories, Rebecca-owned storage, and existing reparse-like
  paths are blocked.
- Built-in rules are Cleaner Manifest v1 TOML, Windows-scoped, project-owned,
  and validated against the shared protection model and safety catalog at load
  time.
- Default execution moves files, or direct child entries of directory targets,
  to the Windows Recycle Bin.
- History stores request metadata, target paths, byte counts, statuses, reason
  codes, issue matrices, target-scoped issue reasons, and restore hints. It
  does not store file contents.

The core destructive-operation boundaries, bounded scan scheduling, execution
revalidation and classification, scan-cache lifecycle with best-effort pruning
after plan builds, catalog target-shape validation, protected-result audit
round-trip, and first guardrailed catalog expansion batch are in place. Future
cleanup families must continue to prove they stay inside those boundaries, but
no remaining cleanup-system safety gap blocks the current Mole-like
Windows-first scope.
Release integrity is tracked separately from cleanup-runtime safety: the
repository now has cargo-dist GitHub Release generation, checksum generation,
crates.io publishing automation, and preflight packaging smoke tests.

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
The policy consumes compiled `SafetyKnowledge` from
`crates/rebecca-rules/safety/windows.toml`; `rebecca-core` also embeds the same
catalog shape so library callers get safe defaults without the rules crate.

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
- Chromium-family cache directories: profile-local `Cache`, `Code Cache`,
  `GPUCache`, `DawnCache`, and `Media Cache`, plus base-level
  `component_crx_cache`, `extensions_crx_cache`, `GraphiteDawnCache`,
  `GrShaderCache`, and `ShaderCache` for Chrome, Edge, Brave, and Chromium;
  these are allowed only as bounded browser-cache leaves and exclude Network
  state, Safe Browsing journals, Preferences edits, history, cookies, sessions,
  and profile databases;
- Gecko-family local profile cache directories such as `cache2`, `startupCache`,
  `jumpListCache`, and `OfflineCache` for Firefox, Waterfox, Zen Browser, and
  Thunderbird;
- Electron/VS Code cache directories such as `Cache`, `Code Cache`,
  `GPUCache`, and `CachedData` for explicitly allowlisted app roots including
  Discord, Slack, Postman, Notion, and Figma;
- domestic desktop-app cache leaves for WeChat, Enterprise WeChat, QQ,
  Feishu, DingTalk, WPS, Baidu Netdisk, Tencent Meeting, QQ Music, and Tencent
  Video. These rules stay on observed AppData cache leaves such as `Cache`,
  `Code Cache`, `Cache_Data`, `filecache`, `resource_cache`, `Image`, and
  vendor dynamic-resource caches; app roots, account state, document
  state, sync state, downloaded media, `Local Storage`, `IndexedDB`, and
  session data remain outside the cleanup surface;
- JetBrains product `caches` directories;
- Android user cache leaves under `.android\cache` and
  `.android\build-cache`, plus Android Studio
  `%LOCALAPPDATA%\Google\AndroidStudio*\caches` directories;
- Cargo cache subdirectories under `registry` and `git`; sccache local disk
  compiler-cache roots under `%LOCALAPPDATA%\Mozilla\sccache` and
  `%SCCACHE_DIR%`;
- Hugging Face cache roots under `%HF_HOME%` and `%HF_*%`, plus the default
  `%USERPROFILE%\.cache\huggingface\hub`, `datasets`, `assets`, and `xet`
  subdirectories; PyTorch Hub cache roots under `%TORCH_HOME%\hub` and
  `%USERPROFILE%\.cache\torch\hub`, including the default `checkpoints`
  subdirectory beneath those roots;
- ccache cache buckets under `%CCACHE_DIR%`, `%USERPROFILE%\.ccache`,
  `%LOCALAPPDATA%\ccache`, and `%APPDATA%\ccache`, plus `tmp`; `ccache.conf`,
  `CACHEDIR.TAG`, and `stats` stay outside the cleanup surface;
- Windows maintenance caches under `%WINDIR%\Temp`, `%WINDIR%\Prefetch`, and
  `%WINDIR%\SoftwareDistribution\Download`;
- pip, uv, Poetry package-cache, Conda package-cache, Go build/module, Cargo,
  rustup, npm, pnpm,
  Yarn, Bun, Corepack, NuGet, Gradle, and Maven cache directories;
- Windows Error Reporting `ReportArchive` and `ReportQueue`;
- Steam client web cache directories.
- app-leftover cache directories derived from discovered installed apps, limited
  to `Cache`, `Code Cache`, `GPUCache`, and `CachedData` under
  `AppData\Local`, `AppData\Roaming`, or `AppData\LocalLow`.

The allowlist exists so protected categories can be conservative without
blocking known rebuildable caches. Pure lists and stable relative target
allowlists live in the safety catalog; dynamic checks such as traversal,
filesystem roots, Rebecca-owned storage overlap, user-protected path overlap,
reparse-point detection, and structured app-specific cache boundaries remain in
Rust.

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

`ProtectionPolicy` currently blocks these categories. Category descriptions and
simple segment or sequence matchers are declared in the safety catalog, while
browser profile boundaries, ccache bucket checks, and domestic desktop-app
cache-vs-state boundaries remain code-level structural checks.

| Category | Examples |
|----------|----------|
| Credentials | Microsoft Credentials/Protect/Crypto/Vault, `.ssh`, `.gnupg`, 1Password, Bitwarden, Cargo `credentials.toml` |
| VPN/proxy state | Clash, Clash Verge, Tailscale, WireGuard, V2Ray, Shadowsocks, sing-box |
| AI/coding durable state | `.codex`, `.claude`, `.cursor`, `.ollama`, Claude, Cursor, Ollama, ChatGPT, VS Code `User` |
| Browser private data | Chromium `History`, `Cookies`, `Login Data`, `Web Data`, `Local Storage`, `IndexedDB`, `Service Worker`, `Network`; Firefox cookies/history/login databases |
| Cloud-synced data | OneDrive, iCloud Drive, iCloud Photos, Dropbox, Google Drive, Box, MEGA |
| Container/VM runtime state | Docker, Docker Desktop, Podman, Rancher Desktop, WSL config, `.docker`, `.podman`, `.kube` |
| Startup automation | Windows Startup folder paths |
| Application durable data | Steam `userdata`, `steamapps\common`, `steamapps\workshop`, `steamapps\compatdata`, Conda environments, Android AVDs, SDK packages, adb/debug keys, domestic desktop-app roots and account/sync/session leaves, browser-like durable storage roots such as `Local Storage`, `IndexedDB`, `Service Worker`, and `Network` |

These categories are intentionally conservative. False negatives where Rebecca
refuses to clean a path are acceptable safety outcomes; false positives that
delete durable user data are not.

## Rule Catalog Governance

Built-in rules live under `crates/rebecca-rules/rules/windows/` and are embedded
from Cleaner Manifest v1 TOML files. The safety catalog lives under
`crates/rebecca-rules/safety/windows.toml`. The loaders and validators enforce:

- `manifest_version = 1` for rule manifests and `catalog_version = 1` for the
  safety catalog;
- valid TOML with unknown fields rejected;
- non-empty rule id, category, name, provenance, and target path;
- rule warning kinds that exist in the safety catalog;
- unique rule ids and target specs;
- Windows platform and `windows.` id prefix for built-ins;
- non-empty restore hints;
- owned source and `project-owned` provenance license;
- target shapes that overlap protected categories or unallowlisted Steam
  install/library relative paths.

When external projects inform a rule family, keep the upstream project name,
repository or file path, license, and revision reference in
`provenance.notes`. GPL references such as Mole and BleachBit remain behavior
references only; do not copy rule definitions or code from them into the
catalog.

Rule authoring guidance lives in `docs/rule-authoring.md`. Reference projects
under `repo-ref/` are research inputs only; their GPL code and rules are not
copied into Rebecca.

## Execution Model

The current Windows backend moves allowed file targets to the Recycle Bin. For
directory targets, it preserves the target directory and moves direct child
entries. This keeps app-created cache directories in place while clearing their
contents.

Current limitations:

- backend permission and IO failures are grouped as `execution-failed` rather
  than more granular filesystem categories;
- non-Windows execution is unavailable and returns a platform error.

## Dry Run, History, And Audit Data

Dry-run is the primary operator contract. Cleanup execution, purge execution,
history, config, cache, doctor, catalog, and inspect machine output use the
versioned `rebecca.cli.v1` API envelope documented in `docs/api/cli/v1/`.
Breaking machine-output changes after release require a new CLI API version or
an explicit pre-release contract migration.

`rebecca catalog` is the canonical audit surface for cleanup rules, project
artifact policies, warning gates, safety categories, and action kinds. Older
single-purpose listings may remain for compatibility, but new wrappers should
consume the unified catalog instead of scraping human text.

`inspect lint` is report-only by design. It may identify duplicate groups,
large files, empty files, and empty directories, but it does not choose
deletion winners, mutate hardlinks, shred files, write history, or enter the
Recycle Bin backend. Reference and protected roots are treated as keep
candidates for conservative duplicate reclaim estimates.

History is append-only JSONL. It records:

- cleanup request metadata;
- summary counts and byte counts;
- target paths;
- statuses;
- stable reason codes;
- issue matrices;
- restore hints.

Human history output replays issue targets with status, stable reason code,
rule id, path, target-scoped reason, and restore hint when present. It includes
execution-time revalidation outcomes such as `execution-target-missing`,
`safety-policy-blocked`, and `execution-failed`, but does not expand arbitrary
child-file listings.

JSON and NDJSON machine modes follow the same privacy boundary: they may expose
target-level paths already present in cleanup plans, status, byte estimates,
reason codes, and scan-cache lifecycle events, but they must not emit file
contents, credentials, tokens, browser databases, or arbitrary child-file
listings. GUI wrappers should consume `--format ndjson` for progress instead
of scraping human spinner text.

History must not store file contents, credentials, tokens, browser databases, or
arbitrary child-file listings. Scan-cache records are rebuildable optimization
data, not an audit log, and stale or corrupted cache files are pruned instead
of being treated as durable state.

## Release Integrity

Rebecca treats release trust as security-sensitive because users run a local
cleanup binary against personal data. The current release hardening path uses:

- a Windows GitHub Actions CI quality gate with read-only repository
  permissions;
- a tag-triggered cargo-dist release workflow for Windows x86_64 MSVC artifacts;
- SHA-256 checksum files generated for downloadable GitHub Release artifacts;
- crates.io publishing automation that releases workspace crates in dependency
  order;
- PowerShell packaging smoke tests that include `rebecca.exe`, README, security
  policy, release guide, install script, and this safety audit;
- user verification guidance in `docs/release.md`.

This distribution layer does not change cleanup behavior. It gives users and
maintainers a way to verify that a downloaded artifact matches the release
checksum and that crates are published in an explicit dependency order.

## Current Verification Coverage

Focused coverage currently includes:

- `crates/rebecca-core/tests/safety_policy.rs` for path validation, protected
  categories, allowlisted maintenance paths, and Rebecca-owned storage
  protection, including domestic app cache leaves and unsafe near-misses;
- `crates/rebecca-core/tests/safety_catalog.rs` for safety catalog versioning,
  category completeness, warning-kind uniqueness, and data-driven matcher
  coverage;
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
- `crates/rebecca/tests/cli_clean.rs`, `cli_scan.rs`, `cli_apps.rs`, and
  `cli_history.rs`
  for user-facing and JSON contract behavior, including protected issue target
  replay.
- `crates/rebecca/tests/cli_purge.rs` for project-artifact purge output,
  selector catalog listing, and recent-artifact messaging.
- `crates/rebecca/tests/cli_scan.rs` for builtin rule listing and scan
  filter coverage.
- `crates/rebecca-core/tests/project_artifacts.rs` for project-artifact
  discovery, context-sensitive cache detection, selector filtering, and recent
  artifact skipping.
- `crates/rebecca-core/tests/lint_report.rs` for duplicate grouping,
  reference/protected keep candidates, large and empty file reporting, empty
  directory ordering, and exclude behavior.
- `crates/rebecca/tests/cli_catalog.rs` and `cli_inspect.rs` for unified
  catalog filters, inspect payloads, read-only history boundaries, and
  canonical inspect command behavior.
- `crates/rebecca/tests/cli_api.rs` for v1 schema parseability and
  representative JSON example validation.

Recent targeted verification for this audit baseline:

- `cargo nextest run -p rebecca-core --test safety_policy`
- `cargo nextest run -p rebecca-core --test planner`
- `cargo nextest run -p rebecca-core --test executor_contract`
- `cargo nextest run -p rebecca-core --test model_contract`
- `cargo nextest run -p rebecca-core --test history`
- `cargo nextest run -p rebecca --test cli_scan`
- `cargo nextest run -p rebecca --test cli_purge`
- `cargo nextest run -p rebecca --test cli_apps`
- `cargo nextest run -p rebecca --test scan`
- `cargo nextest run -p rebecca --test cli_clean`
- `cargo nextest run -p rebecca --test cli_history`
- `cargo nextest run -p rebecca-core --test project_artifacts`
- `cargo nextest run -p rebecca-windows`
- `cargo nextest run -p rebecca-rules`

## Known Limitations And Planned Hardening

- Future cleanup-rule expansion must stay batch-sized and include
  family-specific unsafe-near-miss tests, CLI contract coverage, and audit
  updates before new rules are considered complete.
- `rebecca catalog --kind project-artifact` is the canonical scan-free selector
  catalog surface; `purge --list-artifacts` is retained as a compatibility view
  and must stay in sync.
- Protected category coverage is conservative but not exhaustive for all Windows
  applications.
- The release workflow has not yet been exercised by a public version tag in
  this repository.
- MSI/MSIX installer UX, package-manager publishing, Windows ARM64 artifacts,
  and fully pinned GitHub Action SHAs remain distribution-layer follow-up work
  beyond the current cleanup-safety slice.

The U8 completion review is recorded in
`docs/knowledge/engineering/verification/2026-06-24-mole-parity-completion-review.md`.
