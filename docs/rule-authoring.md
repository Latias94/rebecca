# Rule Authoring

Built-in Rebecca rules live under `crates/rebecca-rules/rules/windows/` as TOML
files. Keep each rule small, explicit, and easy to audit.

## Current Shape

- One file per rule.
- Stable env-variable templates only.
- Use `glob-template` only for bounded profile or filename discovery.
- Prefer paths that Windows users recognize immediately.

## Required Fields

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
  policy allowlist before adding its built-in rule.
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
- `safe`: disposable caches, shader caches, regenerated browser data.
- `moderate`: developer caches, diagnostic artifacts, package caches.
- `risky` and above: only when the user impact is well understood.

## Verification

- `cargo nextest run -p rebecca-rules`
- `cargo nextest run -p rebecca-cli --test cli_apps`
- `cargo nextest run --workspace`
- `cargo run -p rebecca-cli -- scan`

## Provenance

- Do not copy GPL rule definitions or code into the catalog.
- Built-in rules must use `platform = "windows"`, a `windows.` rule id prefix,
  `source = "owned"`, and `license = "project-owned"`.
- Built-in rules must include a concise non-empty `restore_hint`, because dry-run,
  history, and grouped human output surface it as part of the safety contract.
- Document the source of each rule in `provenance.notes`.
