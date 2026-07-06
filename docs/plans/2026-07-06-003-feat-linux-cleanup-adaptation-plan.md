---
title: Linux Cleanup Adaptation - Plan
type: feat
date: 2026-07-06
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Linux Cleanup Adaptation - Plan

## Goal Capsule

| Field | Value |
|---|---|
| Objective | Make Linux a first-class Rebecca platform across rule planning, recoverable execution, catalog discovery, doctor diagnostics, user-facing output, docs, and CI. |
| Authority | The user's fearless-refactor direction outranks current unreleased CLI compatibility; Linux correctness, safety, and explainability outrank preserving Windows-shaped shortcuts. |
| Execution profile | Breaking CLI/API/schema changes are allowed when they remove Windows assumptions or make Linux behavior clearer. |
| Stop conditions | Stop if a proposed Linux cleaner cannot be expressed as owned, previewable, bounded, and recoverable-by-default; stop if a system-level Linux target would encourage permanent root-owned deletion without an explicit opt-in and warning gate. |
| Tail ownership | `ce-work` owns implementation, focused tests, full quality gates, commits, and push to `main` unless a genuine safety or platform blocker appears. |

---

## Product Contract

### Summary

Rebecca should feel native on Linux instead of merely compiling there.
The Linux release surface should include safe previews, recoverable trash execution, useful default cleanup rules, platform-filtered catalog discovery, readable progress, machine output that reports `platform: linux`, and diagnostics that explain Linux permissions and trash capability.

### Problem Frame

The cross-platform execution refactor already moved recoverable trash execution into `rebecca-core`, added `Platform::Linux`, and introduced a shared rule manifest shape.
The remaining Linux gap is product depth: only `linux.user-temp` exists today, XDG defaults are not a first-class path-template concept, catalog output cannot filter by platform, `doctor permissions` still reports Windows-era unsupported text, and most tests prove Windows rule richness rather than Linux parity.

Linux cleanup is not a Windows port.
It needs XDG directory semantics, `$HOME` fallback behavior, Flatpak and Snap cache awareness, Linux package-manager caution, `/proc` process diagnostics, root-owned target failure modes, and protection rules that block durable user state such as `.config`, `.local/share`, credentials, container state, and browser private data.

### Requirements

**Linux platform semantics**

- R1. Linux planning must use platform-aware environment expansion for `HOME`, `XDG_CACHE_HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, and `TMPDIR`, with documented fallback behavior when XDG variables are unset.
- R2. Linux target validation must reject broad profile-root and filesystem-root globs, including `%HOME%/*`, `%XDG_CACHE_HOME%/*` when the wildcard is too high, and privileged roots without warning gates.
- R3. Linux safety knowledge must protect durable user state while allowing explicitly bounded cache, temp, package-store, shader-cache, thumbnail-cache, download-cache, log-cache, and package-manager archive targets.

**Linux cleanup coverage**

- R4. Built-in Linux rules must cover mainstream developer caches: Cargo, Rustup, npm, pnpm, Yarn, Bun, Corepack, pip, uv, Poetry, Conda, Go build/module cache, Gradle, Maven, NuGet, Android, ccache, sccache, Hugging Face, and PyTorch.
- R5. Built-in Linux rules must cover mainstream browser and mail caches: Chrome, Chromium, Brave, Edge, Firefox, Waterfox, Zen, Thunderbird, and Linux thumbnail caches, while preserving browser-private-data protections.
- R6. Built-in Linux rules must cover mainstream desktop application caches where owned path knowledge is clear: VS Code, JetBrains, Discord, Slack, Zoom, Postman, Figma, VLC, Steam client cache, and app-package cache locations such as Flatpak and Snap when bounded to known application IDs.
- R7. Built-in Linux system-maintenance rules may cover package-manager download/archive caches such as APT, DNF, Pacman, and Zypper only as moderate, permission-sensitive, preview-first rules with clear failure behavior for standard users.

**CLI and diagnostics**

- R8. `catalog` must expose platform filtering so users can list `linux.*` rules without scanning the entire multi-platform catalog.
- R9. `clean`, `scan`, JSON, NDJSON, and human output must continue to select the current host platform by default, and Linux tests must prove `linux.*` rules produce allowed targets from isolated fixtures.
- R10. `doctor permissions` must stop reporting Linux as a Windows-only unsupported platform and instead report Linux user identity, effective-root status, recoverable-trash expectation, and suggested action.
- R11. `doctor active-processes` must support Linux process discovery through `/proc` when available, with the existing debug override retained for deterministic tests.
- R12. Help text and examples must explain Linux rule IDs, XDG cache defaults, `--allow-moderate`, warning gates, recoverable trash, and standard-user failure modes.

**Verification and release confidence**

- R13. CI and local verification must prove Linux rule catalog validity, Linux CLI behavior, Linux clippy, and Linux-friendly docs without depending on root privileges.
- R14. The Unreleased changelog must call out Linux support breadth, breaking catalog/doctor output changes, and any removed Windows-only assumptions.

### Acceptance Examples

- AE1. Given a Linux test environment with `HOME=/tmp/alice` and no `XDG_CACHE_HOME`, when `rebecca clean --format json --rule linux.pip-cache --allow-moderate` runs against a fixture under `/tmp/alice/.cache/pip`, then the plan reports at least one allowed Linux target and the request platform is `linux`.
- AE2. Given a Linux test environment with a Chrome cache fixture under `$HOME/.cache/google-chrome/Default/Cache`, when `rebecca clean --format ndjson --rule linux.chrome-cache --progress-detail file --no-scan-cache` runs, then target lifecycle events are emitted without human text.
- AE3. Given a Linux host without permission to move `/var/cache/apt/archives` entries to trash, when `rebecca clean --yes --rule linux.apt-cache --allow-moderate --allow-warning permission-sensitive` runs, then Rebecca reports failed targets and does not fall back to permanent deletion.
- AE4. Given any host, when the user runs `rebecca catalog --kind cleanup-rule --platform linux --format json`, then only cleanup rules whose IDs start with `linux.` appear.
- AE5. Given a Linux test process snapshot containing `firefox`, when `rebecca doctor active-processes --format json` runs through the deterministic override or `/proc` reader, then active warning-bearing Linux browser rules are reported.
- AE6. Given a Linux fixture containing `.config/Code/User/settings.json`, when a rule or cleanup advice path shape points at that durable state, then catalog validation or protection assessment blocks it as durable application state.

### Scope Boundaries

- This plan does not add macOS rule parity.
- This plan does not add vendor uninstallers, app removal, package-manager command execution, or root-privilege escalation.
- This plan does not clean browser history, cookies, passwords, profiles, extensions, Local Storage, IndexedDB, Service Worker durable state, or application databases.
- This plan does not introduce permanent deletion for Linux cleanup rules.
- This plan does not require Linux root access in CI; privileged targets are validated through catalog shape tests and failure-mode tests using fixtures or fake backends.

### Sources

- `docs/plans/2026-07-06-002-refactor-cross-platform-cleanup-execution-plan.md` is the dependency that moved recoverable execution and portable project artifacts out of Windows-only semantics.
- `crates/rebecca-core/src/model.rs` already defines `Platform::Linux`, `Platform::current()`, and `DeleteMode::RecoverableDelete`.
- `crates/rebecca-core/src/path_template.rs` currently expands `%VAR%` only from the injected environment and returns no candidate when a variable is missing.
- `crates/rebecca-rules/rules/cleanup/user-temp.toml` is the only built-in manifest with a Linux platform block today.
- `crates/rebecca-core/safety/cleanup.toml` and `crates/rebecca-rules/safety/cleanup.toml` already contain Linux platform safety knowledge but need more allowlist and protected-shape coverage for a broad Linux catalog.
- `crates/rebecca/src/info.rs` still reports Linux permission and active-process capability through Windows-era unsupported diagnostics.

---

## Planning Contract

### Key Technical Decisions

- KTD1. Add platform-aware environment expansion before adding large Linux rule batches.
  Linux rules should not duplicate every XDG fallback path when a deterministic platform default exists; `%XDG_CACHE_HOME%` should resolve to `$HOME/.cache` when unset, while missing `HOME` still yields no candidate.
- KTD2. Keep Linux rule IDs platform-prefixed.
  Runtime IDs such as `linux.chrome-cache` preserve explicit platform identity, make catalog filtering simple, and avoid pretending that Windows and Linux cache paths are the same operational target.
- KTD3. Prefer user-owned caches before privileged system caches.
  Developer, browser, desktop, Flatpak, and Snap user caches are P0 because they are common, recoverable, and standard-user friendly; package-manager caches are P1 moderate rules because they are root-owned on many systems.
- KTD4. Use warning gates rather than hidden platform exclusions for risky Linux shapes.
  `permission-sensitive`, `active-process`, `broad-discovery`, `source-boundary`, and `durable-state-nearby` should explain why a target needs opt-in instead of silently disappearing.
- KTD5. Make Linux diagnostics capability-level, not command-level.
  Linux should be reported as supported for cleanup execution and portable scans, while app-leftover discovery, Windows-native scan, and NTFS/MFT remain separate unavailable capabilities.
- KTD6. Keep plain filesystem Linux adapters in existing crates until a native boundary appears.
  XDG defaults, `/proc` process snapshots, and Steam path discovery can live behind core or CLI interfaces using `std`; create a `rebecca-linux` crate only if future Linux-specific FFI or platform libraries justify it.
- KTD7. Treat external cleaner projects as behavior references only.
  Linux paths may be cross-checked against mature cleaners, docs, and conventions, but built-in TOML data must remain project-owned with no copied GPL/LGPL rule data.

### High-Level Technical Design

```mermaid
flowchart TB
  CLI[rebecca CLI] --> Host[Platform::current]
  Host --> Env[PlatformEnvironment]
  Env --> Planner[core cleanup planner]
  Rules[shared cleanup manifests] --> Catalog[platform-expanded rule catalog]
  Catalog --> Planner
  Safety[platform safety knowledge] --> Planner
  Planner --> Output[human/json/ndjson output]
  Planner --> Trash[core recoverable trash backend]
  CLI --> Doctor[doctor permissions and active-processes]
  Doctor --> Proc[/proc snapshots on Linux]
```

Linux adaptation should keep the existing plan-first pipeline intact.
The new work adds Linux-specific inputs to the pipeline rather than a separate Linux planner.

### Sequencing

1. Build Linux path semantics and safety gates first.
2. Add the developer-cache rule batch because it is high-value and mostly user-owned.
3. Add browser and desktop-app cache batches with stronger active-process and browser-private-data checks.
4. Add moderate system-maintenance rules after the safety gates prove privileged paths remain opt-in.
5. Finish with catalog filtering, doctor output, docs, API examples, and Linux CI/dogfood.

### System-Wide Impact

- Catalog output grows substantially and needs platform filtering to stay usable.
- Linux rules increase the importance of platform-specific safety knowledge and path-template tests.
- `doctor permissions` and `doctor active-processes` machine output may change because Linux is no longer an unsupported placeholder.
- More rules mean progress, scan-cache, and NDJSON tests must avoid relying on Windows-only rule IDs.

### Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Linux cache layouts vary by distro, package format, and application channel. | Start from XDG defaults, common native paths, and bounded Flatpak/Snap app IDs; keep uncertain paths out until owned evidence is clear. |
| `/var/cache` package-manager cleanup can require root and may fail through trash. | Mark rules moderate and permission-sensitive, preview by default, and test graceful failure rather than root deletion. |
| Broad globs under `$HOME` could sweep user data. | Extend shape validation for `%HOME%`, `%XDG_CACHE_HOME%`, Flatpak, and Snap roots before adding those globs. |
| Browser cache rules may accidentally cross into private data. | Keep and extend regenerable-browser-cache validation with Linux-specific accepted cache leaves and rejected private leaves. |
| Large Linux rule batches could make tests brittle across host environments. | Use isolated env fixtures, deterministic debug overrides, and machine-output tests that set `HOME`, XDG variables, `TEMP`, and `TMPDIR`. |

---

## Implementation Units

### U1. Linux environment and template semantics

- **Goal:** Make Linux XDG and HOME expansion a core capability instead of ad hoc duplicated rule paths.
- **Requirements:** R1, R2, R9.
- **Files:** `crates/rebecca-core/src/environment.rs`, `crates/rebecca-core/src/path_template.rs`, `crates/rebecca-core/tests/path_templates.rs`, `crates/rebecca/src/clean.rs`, `crates/rebecca/src/scan.rs`, `docs/rule-authoring.md`.
- **Approach:** Add a platform-aware environment wrapper used by CLI planning that supplies Linux XDG defaults from `HOME` when variables are absent, keeps missing `HOME` as no candidate, and preserves injected test environments.
- **Patterns:** Follow the existing `Environment` trait and `MapEnvironment` test style.
- **Test scenarios:** `%XDG_CACHE_HOME%/pip` expands to `$HOME/.cache/pip` on Linux when XDG is unset; explicit `XDG_CACHE_HOME` wins; missing `HOME` returns no candidate; Windows `%LOCALAPPDATA%` behavior is unchanged; isolated CLI tests can set Linux-style env vars without depending on the real host profile.
- **Verification:** `cargo nextest run -p rebecca-core --locked path_templates`.

### U2. Linux safety catalog and shape validation

- **Goal:** Let a broad Linux catalog grow while protection gates stay stricter than the rule set.
- **Requirements:** R2, R3, R6, R7.
- **Files:** `crates/rebecca-core/safety/cleanup.toml`, `crates/rebecca-rules/safety/cleanup.toml`, `crates/rebecca-core/src/protection.rs`, `crates/rebecca-core/src/protection/patterns.rs`, `crates/rebecca-rules/src/lib.rs`, `crates/rebecca-core/tests/safety_catalog.rs`, `crates/rebecca-core/tests/safety_policy.rs`.
- **Approach:** Add Linux maintenance allowlists for accepted cache/package-store shapes, protected patterns for `.config`, `.local/share`, credentials, container state, startup automation, and browser private data, and root-glob guards for `%HOME%`, `%XDG_CACHE_HOME%`, `%XDG_DATA_HOME%`, Flatpak, and Snap roots.
- **Patterns:** Reuse existing platform safety catalog parsing and built-in catalog validation gates.
- **Test scenarios:** Linux cache leaves pass; `.config/*`, `.local/share/*`, `.ssh`, browser history, Local Storage, IndexedDB, and Service Worker shapes are blocked; broad `%HOME%/*/Cache` globs are rejected; privileged package caches require `permission-sensitive`.
- **Verification:** `cargo nextest run -p rebecca-core --locked safety_catalog safety_policy` and `cargo nextest run -p rebecca-rules --locked`.

### U3. Linux developer cache rule batch

- **Goal:** Add first-class Linux cleanup rules for high-value developer caches.
- **Requirements:** R4, R9, R13.
- **Files:** `crates/rebecca-rules/rules/cleanup/cargo-cache.toml`, `crates/rebecca-rules/rules/cleanup/rustup-cache.toml`, `crates/rebecca-rules/rules/cleanup/npm-cache.toml`, `crates/rebecca-rules/rules/cleanup/pnpm-cache.toml`, `crates/rebecca-rules/rules/cleanup/yarn-cache.toml`, `crates/rebecca-rules/rules/cleanup/bun-cache.toml`, `crates/rebecca-rules/rules/cleanup/corepack-cache.toml`, `crates/rebecca-rules/rules/cleanup/pip-cache.toml`, `crates/rebecca-rules/rules/cleanup/uv-cache.toml`, `crates/rebecca-rules/rules/cleanup/poetry-cache.toml`, `crates/rebecca-rules/rules/cleanup/conda-cache.toml`, `crates/rebecca-rules/rules/cleanup/go-build-cache.toml`, `crates/rebecca-rules/rules/cleanup/go-module-cache.toml`, `crates/rebecca-rules/rules/cleanup/gradle-cache.toml`, `crates/rebecca-rules/rules/cleanup/maven-cache.toml`, `crates/rebecca-rules/rules/cleanup/nuget-cache.toml`, `crates/rebecca-rules/rules/cleanup/android-cache.toml`, `crates/rebecca-rules/rules/cleanup/ccache-cache.toml`, `crates/rebecca-rules/rules/cleanup/sccache-cache.toml`, `crates/rebecca-rules/rules/cleanup/huggingface-cache.toml`, `crates/rebecca-rules/rules/cleanup/pytorch-cache.toml`, `crates/rebecca-rules/src/lib.rs`, `crates/rebecca/tests/cli_clean.rs`.
- **Approach:** Add Linux platform blocks using XDG and documented home layouts, keep destructive install roots and credentials out, attach moderate safety where redownload/rebuild cost is material, and update built-in ID coverage tests.
- **Patterns:** Match the shared manifest shape in `crates/rebecca-rules/rules/cleanup/user-temp.toml` and provenance gates in `crates/rebecca-rules/src/lib.rs`.
- **Test scenarios:** Expected `linux.*` developer rules exist; isolated `HOME` fixtures produce allowed targets for representative safe and moderate rules; `--allow-moderate` gates moderate rules; no developer rule points at toolchain binaries, credentials, virtualenv directories, build-tools, SDK platforms, or package-manager databases.
- **Verification:** `cargo nextest run -p rebecca-rules --locked` and `cargo nextest run -p rebecca --locked cli_clean`.

### U4. Linux browser, mail, and thumbnail cache batch

- **Goal:** Add Linux browser cleanup coverage while keeping private browsing data protected.
- **Requirements:** R5, R9, R11.
- **Files:** `crates/rebecca-rules/rules/cleanup/chrome-cache.toml`, `crates/rebecca-rules/rules/cleanup/chromium-cache.toml`, `crates/rebecca-rules/rules/cleanup/brave-cache.toml`, `crates/rebecca-rules/rules/cleanup/edge-cache.toml`, `crates/rebecca-rules/rules/cleanup/firefox-profile-cache.toml`, `crates/rebecca-rules/rules/cleanup/waterfox-cache.toml`, `crates/rebecca-rules/rules/cleanup/zen-browser-cache.toml`, `crates/rebecca-rules/rules/cleanup/thunderbird-cache.toml`, `crates/rebecca-rules/rules/cleanup/thumbnail-cache.toml`, `crates/rebecca-rules/src/lib.rs`, `crates/rebecca/tests/cli_api.rs`, `crates/rebecca/tests/cli_clean.rs`.
- **Approach:** Add Linux cache leaves under XDG cache directories and known profile cache directories, add active-process warnings where appropriate, and extend browser-shape validators for Linux cache names such as `Cache`, `Code Cache`, `GPUCache`, `ShaderCache`, `cache2`, `startupCache`, and thumbnails.
- **Patterns:** Follow existing browser boundary validation and NDJSON progress tests that already use current-platform rule IDs.
- **Test scenarios:** Linux browser cache fixtures produce target events; history, cookies, preferences, Local Storage, IndexedDB, and Service Worker data are rejected; browser rules include active-process warnings; NDJSON remains machine-only.
- **Verification:** `cargo nextest run -p rebecca-rules --locked` and `cargo nextest run -p rebecca --locked cli_api cli_clean`.

### U5. Linux desktop, Flatpak, Snap, and Steam cache batch

- **Goal:** Cover common Linux desktop app caches without turning app data directories into cleanup targets.
- **Requirements:** R6, R9, R11.
- **Files:** `crates/rebecca-rules/rules/cleanup/vscode-cache.toml`, `crates/rebecca-rules/rules/cleanup/jetbrains-cache.toml`, `crates/rebecca-rules/rules/cleanup/discord-cache.toml`, `crates/rebecca-rules/rules/cleanup/slack-cache.toml`, `crates/rebecca-rules/rules/cleanup/zoom-logs.toml`, `crates/rebecca-rules/rules/cleanup/postman-cache.toml`, `crates/rebecca-rules/rules/cleanup/figma-cache.toml`, `crates/rebecca-rules/rules/cleanup/vlc-cache.toml`, `crates/rebecca-rules/rules/cleanup/steam-cache.toml`, `crates/rebecca-rules/rules/cleanup/steam-install-cache.toml`, `crates/rebecca-rules/rules/cleanup/steam-install-download-cache.toml`, `crates/rebecca-rules/rules/cleanup/steam-library-shader-cache.toml`, `crates/rebecca-core/src/applications.rs`, `crates/rebecca/src/info.rs`, `crates/rebecca/tests/cli_clean.rs`, `crates/rebecca/tests/cli_apps.rs`.
- **Approach:** Add bounded Linux native, Flatpak, and Snap cache paths for known app IDs; add Linux Steam install/library discovery only for filesystem-readable roots such as `$HOME/.steam/steam` and `$HOME/.local/share/Steam`; keep app-leftover installed-application cleanup capability-gated until Linux installed-app discovery is designed separately.
- **Patterns:** Reuse existing Steam install/library target abstractions where possible, but do not require the Windows discovery crate for Linux.
- **Test scenarios:** Native and Flatpak cache fixtures are allowed; `.var/app/<id>/config`, `.var/app/<id>/data`, and Snap data roots are blocked; Linux Steam cache fixtures resolve without Windows registry discovery; `apps scan` remains capability-specific rather than pretending full Linux installed-app cleanup exists.
- **Verification:** `cargo nextest run -p rebecca --locked cli_clean cli_apps`.

### U6. Linux system-maintenance moderate rules

- **Goal:** Add opt-in Linux system cache cleanup for package-manager archives without weakening standard-user safety.
- **Requirements:** R7, R8, R10, R13.
- **Files:** `crates/rebecca-rules/rules/cleanup/apt-cache.toml`, `crates/rebecca-rules/rules/cleanup/dnf-cache.toml`, `crates/rebecca-rules/rules/cleanup/pacman-cache.toml`, `crates/rebecca-rules/rules/cleanup/zypper-cache.toml`, `crates/rebecca-core/safety/cleanup.toml`, `crates/rebecca-rules/safety/cleanup.toml`, `crates/rebecca-rules/src/lib.rs`, `crates/rebecca/tests/cli_clean.rs`.
- **Approach:** Add owned Linux rules for APT, DNF, Pacman, and Zypper archive/download caches as moderate and permission-sensitive; use exact or bounded glob targets; avoid package databases, logs, locks, repositories, and config directories.
- **Patterns:** Follow the warning-derived shape gate used for privileged Windows maintenance paths.
- **Test scenarios:** Rules are hidden without `--allow-moderate` or required warning gates; privileged targets report skipped or failed cleanly when not writable; catalog validation rejects package database paths; no system rule targets `/var/lib`, `/var/log`, `/etc`, or package-manager config.
- **Verification:** `cargo nextest run -p rebecca-rules --locked` and focused `cli_clean` tests with temporary fixture paths or fake backend failure tests.

### U7. Platform-filtered catalog and Linux scan selection

- **Goal:** Make the expanded multi-platform catalog usable and prove Linux selection paths in CLI tests.
- **Requirements:** R8, R9, R12.
- **Files:** `crates/rebecca-core/src/model.rs`, `crates/rebecca-core/src/catalog.rs`, `crates/rebecca/src/catalog.rs`, `crates/rebecca/src/cli.rs`, `crates/rebecca/tests/cli_catalog.rs`, `crates/rebecca/tests/cli_scan.rs`, `docs/api/cli/v1/payloads.schema.json`, `docs/api/cli/v1/examples/success-catalog.json`.
- **Approach:** Add an optional `--platform` catalog filter, include platform as a structured cleanup-rule catalog field if it is not already derivable enough, and keep `clean` and `scan` defaulting to `Platform::current()`.
- **Patterns:** Follow existing `CatalogQuery` filtering and schema example validation tests.
- **Test scenarios:** `catalog --kind cleanup-rule --platform linux` returns only Linux rules; `--platform windows` still returns Windows rules; invalid platform values are rejected by clap; JSON schema examples include platform; `scan` on Linux only reports current-platform rules.
- **Verification:** `cargo nextest run -p rebecca --locked cli_catalog cli_scan cli_api`.

### U8. Linux doctor diagnostics and active process support

- **Goal:** Replace Windows-era unsupported diagnostics with useful Linux capability reports.
- **Requirements:** R10, R11, R12.
- **Files:** `crates/rebecca/src/info.rs`, `crates/rebecca/src/cli.rs`, `crates/rebecca/tests/info.rs`, `crates/rebecca/tests/cli_api.rs`, `docs/api/cli/v1/payloads.schema.json`, `docs/api/cli/v1/examples/success-doctor-permissions.json`, `docs/api/cli/v1/examples/success-doctor-active-processes.json`.
- **Approach:** Report Linux cleanup execution as supported, read effective UID from `/proc/self/status` on Linux instead of adding a native dependency, inspect `/proc/<pid>/comm` and `/proc/<pid>/exe` when readable, and preserve the `REBECCA_ACTIVE_PROCESSES` override for deterministic tests.
- **Patterns:** Keep machine output in `rebecca.cli.v1` envelopes and reuse active-process warning matching.
- **Test scenarios:** Linux permission diagnostic reports `platform_supported: true` and cleanup execution supported; active-process diagnostics find rules from injected snapshots; unreadable `/proc` produces a capability-specific unavailable reason; Windows behavior stays intact.
- **Verification:** `cargo nextest run -p rebecca --locked info cli_api`.

### U9. Linux docs, skill, and release positioning

- **Goal:** Teach users how to install and use Rebecca on Linux without implying unsafe root cleanup.
- **Requirements:** R12, R14.
- **Files:** `README.md`, `CHANGELOG.md`, `docs/rule-authoring.md`, `docs/security-audit.md`, `docs/api/cli/v1/README.md`, `skills/rebecca-disk-cleaner/SKILL.md`, `skills/README.md`.
- **Approach:** Update examples for Linux `clean`, `catalog --platform linux`, `inspect map`, `purge`, `cache purge`, `--allow-moderate`, and warning gates; describe recoverable trash caveats on headless servers; add Unreleased bullets for Linux catalog expansion and diagnostic changes.
- **Patterns:** Follow existing Unreleased changelog and skill validation constraints.
- **Test scenarios:** Skill validation passes; README no longer says Linux has only `linux.user-temp`; docs do not recommend `sudo rebecca clean --yes` as the default path; API docs list platform-filtered catalog behavior.
- **Verification:** `python skills/validate.py` and documentation search audits.

### U10. Linux CI, dogfood, and performance smoke

- **Goal:** Make Linux support durable after the initial implementation.
- **Requirements:** R13, R14.
- **Files:** `.github/workflows/ci.yml`, `scripts/ci/run-linux-target-clippy.ps1`, `scripts/dogfood/README.md`, `scripts/dogfood/run-linux-cleanup-smoke.ps1`, `docs/performance/perf-matrix.md`, `crates/rebecca-core/benches/perf_matrix.rs`.
- **Approach:** Keep Ubuntu CI as a first-class Rust quality gate, add a no-root Linux cleanup smoke that creates isolated XDG fixtures, and add perf-matrix scenarios for Linux developer/browser cache planning if the benchmark harness can stay host-neutral.
- **Patterns:** Follow existing release and dogfood script style.
- **Test scenarios:** Linux CI runs format, clippy, nextest, catalog validation, and skills validation; dogfood smoke proves fixture cleanup preview and recoverable execution or fail-closed behavior; benchmark matrix can compare Windows and Linux portable recursive planning without NTFS backends.
- **Verification:** Full verification contract below.

---

## Verification Contract

| Gate | Command | Proves |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | Rust formatting is stable. |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | Active-host Rust code has no clippy warnings. |
| Linux target lint | `pwsh -File scripts/ci/run-linux-target-clippy.ps1` | Cross-target Linux compilation remains green from the Windows development host. |
| Tests | `cargo nextest run --workspace --locked --no-fail-fast` | Workspace behavior remains green across core, CLI, rules, NTFS, and Windows adapters. |
| Catalog validation | `cargo run -p rebecca --locked -- catalog validate --format json` | Built-in rules and safety catalogs compile and pass metadata/protection gates. |
| Skill validation | `python skills/validate.py` | Rebecca skill docs remain installable and parseable. |
| Linux catalog audit | `cargo run -p rebecca --locked -- catalog --kind cleanup-rule --platform linux --format json` | Linux catalog entries are discoverable through the user-facing CLI. |
| Search audit | `rg "Windows-only|Windows-first|linux currently includes a safe" README.md docs CHANGELOG.md crates` | Stale Linux-underdeveloped messaging is removed from live docs and code. |
| CI | GitHub Actions `Rust quality gate (ubuntu-24.04)` | Linux quality gates are enforced remotely, not only locally. |

---

## Definition of Done

- D1. Linux planning uses platform-aware XDG/HOME environment semantics with focused tests.
- D2. Linux safety catalog and rule validation block durable state, broad globs, and privileged paths unless the target shape and warning gates are explicit.
- D3. Built-in Linux rules cover the developer, browser, desktop-app, thumbnail, Steam, and moderate package-cache groups named in the Product Contract.
- D4. `catalog --platform linux` exists, returns structured platform data, and keeps the expanded catalog usable.
- D5. `clean`, `scan`, JSON, NDJSON, and progress output have Linux fixture coverage that does not depend on the developer machine's real home directory.
- D6. `doctor permissions` and `doctor active-processes` provide useful Linux diagnostics and preserve Windows behavior.
- D7. README, API docs, security docs, skill docs, and CHANGELOG Unreleased explain Linux support and its safety caveats.
- D8. Full Verification Contract is green, or any host-specific exception is documented with the exact command and reason.
- D9. Abandoned Windows-only compatibility code, stale tests, and exploratory Linux adapters are removed before the final commit.
