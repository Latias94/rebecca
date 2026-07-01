# Rule Authoring

Built-in Rebecca rules live under `crates/rebecca-rules/rules/windows/` as TOML
files. Keep each rule small, explicit, and easy to audit.

## Cleaner Manifest v1

- Every built-in rule file is a Cleaner Manifest v1 document and must start
  with `manifest_version = 1`.
- The current built-in catalog still uses the one-file/one-rule compatibility
  shape: top-level metadata plus top-level `[[targets]]`.
- Manifest v1 also supports future cleaner-style `[[options]]` with
  `[[options.actions]]`. A file must choose top-level `targets` or `options`,
  never both.
- Warnings are declared as stable warning-kind strings in `warnings = [...]`.
  They are part of the planner-ready rule definition and should be reserved for
  user-visible gates such as active-process checks or broad-discovery notices.
  Warning kinds must be declared in `crates/rebecca-rules/safety/windows.toml`.
  Cleanup plans block warning-bearing targets with
  `warning-gate-required` until the user selects the specific gate with
  `--allow-warning <WARNING>`.

## Current Built-in Shape

- One file per rule.
- `manifest_version = 1`.
- Stable env-variable templates only.
- Use `glob-template` only for bounded profile or filename discovery.
- Prefer paths that Windows users recognize immediately.

## Required Fields

- `manifest_version`
- `id`
- `platform`
- `category`
- `name`
- `safety_level`
- `restore_hint`
- `targets`
- `provenance`

## Target Guidance

- Use `template` for stable cache directories.
- Use `exact-path` only for fixed paths that do not vary by environment.
- Use `glob-template` for one-segment wildcard discovery, such as Firefox
  profile directories or `thumbcache_*.db` files.
- Chromium-family browser caches may use bounded `Profile *` discovery under
  `User Data`, but should keep `Default` paths explicit.
- Electron app cache rules should keep `Cache`, `Code Cache`, and `GPUCache`
  targets explicit, and should not target `Local Storage`, `IndexedDB`, or the
  application data root. Add each Electron app root to the shared protection
  policy allowlist before adding its built-in rule. Prefer stable
  `%APPDATA%\\<App>\\<cache leaf>` layouts; avoid app roots with path variants
  or mixed account/session state unless the durable-state boundary is proven
  with tests.
- Domestic desktop-app cache rules must be app-specific and conservative. Use
  only observed AppData cache leaves such as WeChat `radium\cache`, Feishu
  `Cache`/`Code Cache`/shader-cache leaves, DingTalk `resource_cache`, WPS
  `filecache`/HTTP cache leaves, Baidu Netdisk `cache`, and Tencent media app
  cache leaves. Do not target the vendor root, account directories, document
  stores, sync state, downloaded media, `Local Storage`, `IndexedDB`,
  `Service Worker`, or session data. Add positive allowlist tests and negative
  near-miss durable-state tests with every new app.
- Communication and utility app diagnostics rules may target narrow log,
  artwork-cache, or crash-dump leaves such as Zoom `logs`, TeamViewer
  `TeamViewer*_Logfile.log`, and VLC `art\artistalbum`/`crashdump`; keep
  recordings, MRU, registry values, configuration, media files, and account
  state out of built-in rules.
- Browser-family cache rules should target only regenerable cache leaves. For
  Chromium-family apps that means profile-local `Cache`, `Code Cache`,
  `GPUCache`, `DawnCache`, and `Media Cache`, plus base-level
  `component_crx_cache`, `extensions_crx_cache`, `GraphiteDawnCache`,
  `GrShaderCache`, and `ShaderCache`; for Gecko-family apps that means local
  profile leaves such as `cache2`, `startupCache`, `jumpListCache`, and
  `OfflineCache`. Keep Network state, Safe Browsing journals, Preferences JSON
  edits, history, cookies, passwords, sessions, site data, preferences, and
  profile databases out of built-in cache rules.
- Steam client cache rules should stay under `%LOCALAPPDATA%\Steam\htmlcache`
  unless the rule also implements explicit Steam install/library discovery.
  Do not target `userdata`, `steamapps`, `appcache` metadata, workshop content,
  download state, `Service Worker`, or `Network` state from a static template rule.
- For Steam install/library discovery rules, resolve the install root from the
  Windows registry using the ordered discovery sources in `rebecca-windows`
  (`SteamPath`, `SteamExe`, `InstallPath`, then `Shell\\Open\\Command`) and
  expand relative paths against each discovered library root. Keep those
  relative targets narrow and safe; do not allow `..` or absolute paths.
- Current Steam discovery-backed rules are intentionally narrow: an install-root
  cache rule may target `appcache\httpcache`, `appcache\download`,
  `appcache\librarycache`, `appcache\shadercache`, `appcache\stats`,
  `appcache\appinfo.vdf`, `appcache\localization.vdf`,
  `appcache\packageinfo.vdf`, `config\avatarcache`, `depotcache`, or `logs`,
  and library-root cache rules may target
  `steamapps\shadercache`, `steamapps\downloading`, or `steamapps\temp`.
  Built-in catalog validation rejects other Steam install/library relative
  target shapes at load time.
- Cargo cache rules should target cache subdirectories under `%CARGO_HOME%`
  and the default `%USERPROFILE%\.cargo`, not Cargo Home as a whole; never
  target `bin`, `config.toml`, `credentials.toml`, `.crates.toml`, or
  `.crates2.json`.
- ccache cache rules should target the cache buckets under `%CCACHE_DIR%`,
  `%USERPROFILE%\.ccache`, `%LOCALAPPDATA%\ccache`, and `%APPDATA%\ccache`,
  plus `tmp`; never target `ccache.conf`, `CACHEDIR.TAG`, or `stats`.
- sccache cache rules should target the Windows local disk cache root under
  `%LOCALAPPDATA%\Mozilla\sccache` and the configurable `%SCCACHE_DIR%` cache
  root; do not target compiler wrapper binaries, server logs, or config state.
- Hugging Face cache rules should target `%HF_HUB_CACHE%`,
  `%HF_DATASETS_CACHE%`, `%HF_ASSETS_CACHE%`, `%HF_XET_CACHE%`,
  `%HUGGINGFACE_HUB_CACHE%`, `%HUGGINGFACE_ASSETS_CACHE%`, and the documented
  `%HF_HOME%\hub`, `%HF_HOME%\datasets`, `%HF_HOME%\assets`, and
  `%HF_HOME%\xet` subdirectories; do not target the surrounding token or
  account-state files.
- PyTorch cache rules should target `%TORCH_HOME%\hub`,
  and the default `%USERPROFILE%\.cache\torch\hub` equivalent; the default
  `checkpoints` subdirectory lives under that hub root, so do not split it into
  a separate top-level target unless you have a strong reason to do so.
- Android cache rules should target only `.android` cache leaves
  (`%ANDROID_USER_HOME%\cache`, `%ANDROID_USER_HOME%\build-cache`,
  `%ANDROID_SDK_HOME%\.android\cache`,
  `%ANDROID_SDK_HOME%\.android\build-cache`,
  `%USERPROFILE%\.android\cache`, and
  `%USERPROFILE%\.android\build-cache`) plus Android Studio's
  `%LOCALAPPDATA%\Google\AndroidStudio*\caches` directories. Do not target AVDs,
  SDK packages, system images, licenses, adb keys, debug keystores, or IDE
  settings/plugins.
- Windows maintenance cache rules may target `%WINDIR%\Temp`,
  `%WINDIR%\Prefetch`, and `%WINDIR%\SoftwareDistribution\Download`; do not
  target broader system roots or `ProgramData` outside a narrowly justified
  cache family.
- Rustup cache rules may target `%RUSTUP_HOME%\downloads`,
  `%RUSTUP_HOME%\tmp`, and the matching default `%USERPROFILE%\.rustup`
  cache leaves; never target `toolchains`, `settings.toml`, `overrides`, or
  installed components.
- Conda cache rules may target `%USERPROFILE%\.conda\pkgs`,
  `%USERPROFILE%\anaconda3\pkgs`, `%USERPROFILE%\miniconda3\pkgs`,
  `%USERPROFILE%\miniforge3\pkgs`, and `%USERPROFILE%\mambaforge\pkgs`;
  never target environments, configuration, or installed tools.
- Python package-manager cache rules may target pip's `%LOCALAPPDATA%\pip\Cache`,
  uv's `%LOCALAPPDATA%\uv\cache`, and Poetry package-cache subdirectories under
  `%LOCALAPPDATA%\pypoetry\Cache\cache` and
  `%LOCALAPPDATA%\pypoetry\Cache\artifacts`; do not target Poetry virtualenvs,
  Python installations, virtual environments, project `.venv`, or project-local
  tool caches such as `.mypy_cache`, `.pytest_cache`, or `.ruff_cache` from the
  built-in system catalog.
- Go cache rules may target the default Windows build cache
  `%LOCALAPPDATA%\go-build` and default GOPATH module cache
  `%USERPROFILE%\go\pkg\mod`; do not target GOPATH `bin`, `src`, or broad
  `%USERPROFILE%\go\pkg` compiled package output.
- JetBrains IDE caches should point at the product `caches` subdirectory under
  `%LOCALAPPDATA%\JetBrains\<product><version>`, not at the Toolbox app tree.
- Keep glob roots narrow. Do not start a glob at `%USERPROFILE%` or a drive
  root.
- Do not target Rebecca's configured `config_dir`, `state_dir`, `history_file`,
  or `cache_dir` from built-in cleanup rules. Rebecca-owned cache cleanup must
  go through `rebecca cache purge`, which preserves configuration, durable
  state, and append-only history.
- Do not model app-leftover cleanup as broad built-in TOML rules. The
  app-leftovers workflow is inventory-derived, read-only on registry data, and
  limited to rebuildable user-scoped cache leaves under `AppData`. Full
  uninstall behavior, vendor uninstallers, registry writes, `Program Files`,
  and broad application data roots are out of scope for rule authoring.

## Safety Guidance

- Review [Rebecca Cleanup Safety Audit](security-audit.md) before adding a
  new rule family.
- Built-in target shapes are checked against the shared protection policy during
  catalog loading. A rule that points at credentials, browser private data,
  protected application data, or non-allowlisted Steam relative targets is
  invalid even before planning expands it on a user's machine.
- Warning kinds, safety categories, and action kinds are cataloged through
  `rebecca catalog`. New rules should keep manifest metadata, safety catalog
  entries, and catalog output explainable enough for GUI wrappers and audit
  tools to display without scraping human text.
- `safe`: disposable caches, shader caches, regenerated browser data.
- `moderate`: developer caches, diagnostic artifacts, package caches.
- `risky` and above: only when the user impact is well understood.

## Verification

- `cargo nextest run -p rebecca-rules`
- `cargo nextest run -p rebecca --test cli_apps`
- `cargo nextest run --workspace`
- `cargo run -p rebecca -- scan`

## Provenance

- Do not copy GPL rule definitions or code into the catalog.
- Built-in rules must use `platform = "windows"`, a `windows.` rule id prefix,
  `source = "owned"`, and `license = "project-owned"`.
- Built-in rules must include a concise non-empty `restore_hint`, because dry-run,
  history, and grouped human output surface it as part of the safety contract.
- Document the source of each rule in `provenance.notes`.
- When a rule family is derived from an external reference, keep the reference
  in `provenance.notes` with the upstream project name, repository or file path,
  license, and any relevant commit or release tag. Treat GPL sources such as
  Mole and BleachBit as behavior references only, not rule-data sources.
- Winapp2 can inform Windows application cache candidates, especially for
  WeChat, Enterprise WeChat, and Kingsoft/WPS. Keep it as a reference input,
  rewrite the rule from scratch, and preserve the cache-vs-state boundary in
  tests.
- `windows-cleaner-cli` can inform Windows maintenance-cache families such as
  temp files, Prefetch, Windows Update downloads, and related system cache
  comparisons.
- `null-e` can inform developer-cache families such as Cargo, npm, pnpm, Yarn,
  pip, uv, Poetry, Conda, Gradle, Maven, Docker, Android, IDE caches, and ML/AI
  caches such as Hugging Face and PyTorch.
- `Bulk Crap Uninstaller` is useful for uninstall and leftovers modeling, not
  for built-in cleanup target paths.
- Prefer permissive or clearly scoped reference sources when possible. If the
  upstream license or reuse terms are unclear, record the source for behavior
  comparison only and rewrite the rule from scratch.
