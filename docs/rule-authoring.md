# Rule Authoring

Built-in Rebecca cleanup rule families live under
`crates/rebecca-rules/rules/cleanup/` as TOML files. A file is a
platform-neutral cleanup family; the manifest compiler expands each platform
block into runtime rules such as `windows.user-temp` or `linux.user-temp`.
Keep each family small, explicit, and easy to audit.

## Cleaner Manifest v1

- Every built-in rule file is a Cleaner Manifest v1 document and must start
  with `manifest_version = 1`.
- Top-level fields are shared family metadata: `id`, `category`, `name`,
  `safety_level`, `restore_hint`, optional `warnings`, and `provenance`.
  Top-level `id` is a platform-neutral slug without a `windows.` or `linux.`
  prefix.
- Each family must define one or more `[[platforms]]` blocks. A platform block
  has `platform = "windows"`, `platform = "linux"`, or another supported
  platform, plus either `[[platforms.targets]]` or `[[platforms.options]]` with
  `[[platforms.options.actions]]`. A platform block must choose targets or
  options, never both.
- Platform blocks may override `safety_level`, `restore_hint`, and `warnings`
  when the same family has different platform risk or rebuild behavior.
  Warnings merge from shared metadata, platform metadata, and option metadata
  without duplicates.
- Warnings are declared as stable warning-kind strings in `warnings = [...]`.
  They are part of the planner-ready rule definition and should be reserved for
  user-visible gates such as active-process checks or broad-discovery notices.
  Warning kinds must be declared in `crates/rebecca-rules/safety/cleanup.toml`.
  Cleanup plans block warning-bearing targets with
  `warning-gate-required` until the user selects the specific gate with
  `--allow-warning <WARNING>`.
- Targets may declare `search_kind` to make discovery semantics explicit. The
  declared value must match the target kind: `file` for `template` or
  `exact-path`, `glob` for `glob-template`, `steam-install` for
  `steam-install-template`, and `steam-library` for
  `steam-library-template`. Omitted `search_kind` values default from the target
  kind for compatibility.
- Running-process policy is expressed through warning gates today. Use
  `warnings = ["active-process"]` when a cache target belongs to software that
  may be running; the planner blocks it until `--allow-warning active-process`
  is supplied.
- Cache reuse policy is not a per-rule manifest field yet. It is governed by
  the global scan-cache policy and the target search semantics exposed in the
  catalog.

## Current Built-in Shape

- One file per cleanup family.
- `manifest_version = 1`.
- The rule file path, family id, and generated rule ids must agree. For
  example, `rules/cleanup/user-temp.toml` uses `id = "user-temp"` and may
  produce `windows.user-temp` and `linux.user-temp` from separate platform
  blocks.
- Stable env-variable templates only.
- Use `glob-template` only for bounded profile or filename discovery.
- Built-in catalog validation derives a positive target-shape basis from each
  target. A built-in target must prove it is a cache/temp/log/package-store,
  approved Steam maintenance path, approved browser cache boundary, or other
  approved maintenance shape. Do not add broad user folders and rely on prose
  to justify them.
- Shape-derived warnings are mandatory: multi-wildcard glob discovery requires
  `broad-discovery`, `%WINDIR%` maintenance targets require
  `privileged-location`, and Steam install/library discovery targets require
  `source-boundary`.
- Prefer platform-native environment variables and paths that users recognize
  immediately.
- Linux rule templates may use `XDG_CACHE_HOME`, `XDG_CONFIG_HOME`,
  `XDG_DATA_HOME`, and `XDG_STATE_HOME` directly. During planning, Rebecca
  follows XDG defaults from `HOME` when those variables are absent or empty:
  `.cache`, `.config`, `.local/share`, and `.local/state` respectively. If
  `HOME` is missing, the target is skipped as an unexpanded candidate instead
  of guessing a user directory.
- Do not rely on Rebecca to synthesize `TMPDIR` on Linux. A missing `TMPDIR`
  stays missing so built-in user-scoped rules do not accidentally point at a
  shared `/tmp` root.

## Required Fields

- `manifest_version`
- `id`
- `category`
- `name`
- `safety_level`
- `restore_hint`
- `platforms`
- `provenance`

## Target Guidance

- Use `template` for stable cache directories.
- Use `exact-path` only for fixed paths that do not vary by environment.
- Use `glob-template` for one-segment wildcard discovery, such as Firefox
  profile directories or `thumbcache_*.db` files.
- Compatible `glob-template` targets share a per-plan discovery index when they
  enumerate the same directories. Do not rely on that sharing to mix semantics:
  file, Steam install, Steam library, and glob searches remain separate
  `search_kind` values.
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
- Built-in rules must compile to ids with a platform prefix matching each
  `[[platforms]]` block, and must use `source = "owned"` plus
  `license = "project-owned"`.
- Built-in rules must include a concise non-empty `restore_hint`, because dry-run,
  history, and grouped human output surface it as part of the safety contract.
- Document the source of each rule in `provenance.notes`.
- When a rule family is cross-checked against an external reference, keep the
  reference in `provenance.notes` with the upstream project name, repository or
  file path, license, and any relevant commit or release tag. Treat GPL sources
  such as Mole and BleachBit as behavior references only, not rule-data sources,
  and state that no rule data was copied.
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
