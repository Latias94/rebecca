# Rebecca

Rebecca is a Windows-first cleanup CLI written in Rust. It focuses on a safe,
plan-first cleanup flow for system junk, application caches, and leftover app
cache data.

The current MVP supports:

- listing built-in cleanup rules,
- previewing leftover app cache data from read-only installed-app discovery,
- previewing project build artifact purges for directories such as `node_modules`
  and `target`,
- building dry-run cleanup plans,
- scanning target sizes,
- blocking dangerous paths before execution,
- moving allowed files and allowed directory contents to the Windows Recycle Bin,
- recording cleanup history as JSONL.

## Safety Model

Rebecca is designed to preview before deleting.

- `clean --dry-run` and real cleanup use the same plan builder.
- `apps scan` and `apps clean` use the shared cleanup planner with a dedicated
  app-leftovers workflow. `apps clean` previews by default and requires `--yes`
  before moving leftover cache data to the Recycle Bin.
- `purge` uses a dedicated project-artifacts workflow. It scans configured
  purge roots when present, otherwise the current directory, and accepts
  repeated `--root <PATH>` values to override configured roots for one run.
  It previews by default and requires `--yes` before moving project artifacts
  to the Recycle Bin.
- Default execution uses the Windows Recycle Bin.
- Directory targets keep the target directory and move direct child entries.
- Permanent deletion and administrator auto-elevation are not part of the MVP.
- Junctions, symlinks, and other reparse-point traversal are blocked by default.
- Moderate rules require `--allow-moderate`; risky and dangerous rules require `--allow-risky`.
- Use `clean --exclude <PATH>`, `apps scan/clean --exclude <PATH>`, or
  `purge --exclude <PATH>` to protect an absolute path for one run. Long-lived
  protected paths can be configured in `config.toml` under
  `[protection].protected_paths`.
- Dry-run human output highlights the largest estimated targets first and then
  groups the full target list by status.
- Cleanup plans persist stable `reason_code` and `issue_matrix` diagnostics
  for skipped, blocked, and failed targets; human `clean` and `history` output
  surface the issue matrix while preserving the detailed human-readable
  `reason` text.
- Human `clean` commands show target-level and file-level scan progress by
  default, and honor `Ctrl+C` to cancel plan building; use `--no-progress` for
  quiet terminal logs. When `--scan-cache` is enabled, human progress also
  reports scan-cache hits, misses, and skipped cache writes, and the final
  human plan output summarizes those counts. JSON output never emits progress
  or scan-cache diagnostics.
- `clean --scan-cache` explicitly enables the rebuildable scan cache for
  eligible regular-file targets and directory targets with fresh records.
  Cache misses, stale or expired records, corrupted records, and cache-write
  failures are treated as soft rebuilds.

The current destructive-operation boundary and known safety gaps are documented
in [Rebecca Cleanup Safety Audit](docs/security-audit.md).

Security reporting guidance lives in [SECURITY.md](SECURITY.md).

Release artifact verification guidance lives in
[Release Integrity](docs/release.md).

## Usage

```powershell
cargo run -p rebecca-cli -- scan
cargo run -p rebecca-cli -- scan --json
cargo run -p rebecca-cli -- scan --category browser
cargo run -p rebecca-cli -- scan --rule windows.thumbnail-cache

cargo run -p rebecca-cli -- clean --dry-run
cargo run -p rebecca-cli -- clean --dry-run --json --category system
cargo run -p rebecca-cli -- clean --dry-run --no-progress --rule windows.edge-cache
cargo run -p rebecca-cli -- clean --dry-run --json --scan-cache --rule windows.thumbnail-cache
cargo run -p rebecca-cli -- clean --dry-run --json --allow-moderate --rule windows.npm-cache
cargo run -p rebecca-cli -- clean --dry-run --json --allow-risky --rule windows.npm-cache
cargo run -p rebecca-cli -- clean --dry-run --exclude "$env:APPDATA\Slack\Cache"
cargo run -p rebecca-cli -- clean --yes --category system

cargo run -p rebecca-cli -- apps scan
cargo run -p rebecca-cli -- apps scan --json
cargo run -p rebecca-cli -- apps scan --exclude "$env:LOCALAPPDATA\Example App\Cache"
cargo run -p rebecca-cli -- apps clean
cargo run -p rebecca-cli -- apps clean --json --dry-run
cargo run -p rebecca-cli -- apps clean --yes

cargo run -p rebecca-cli -- purge
cargo run -p rebecca-cli -- purge --list-artifacts
cargo run -p rebecca-cli -- purge --list-artifacts --json
cargo run -p rebecca-cli -- purge --json --root . --max-depth 6
cargo run -p rebecca-cli -- purge --root . --min-age-days 0
cargo run -p rebecca-cli -- purge --root . --artifact target
cargo run -p rebecca-cli -- purge --exclude "$PWD\target"
cargo run -p rebecca-cli -- purge --yes --root . --scan-cache

cargo run -p rebecca-cli -- history
cargo run -p rebecca-cli -- history --limit 10
cargo run -p rebecca-cli -- history --json

cargo run -p rebecca-cli -- config paths
cargo run -p rebecca-cli -- cache purge --json
cargo run -p rebecca-cli -- cache purge --yes
cargo run -p rebecca-cli -- doctor permissions
```

## Install

Rebecca can be installed from a GitHub Release with the PowerShell installer:

```powershell
.\scripts\install.ps1 -Repository OWNER/REPO
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0 -RequireAttestation
```

The default install directory is `%LOCALAPPDATA%\Programs\Rebecca`. Re-run the
same command with a newer tag to update the installed `rebecca.exe`. The
installer always verifies `SHA256SUMS` before extraction; `-RequireAttestation`
also requires GitHub CLI build-provenance verification.

## Built-In Rules

The starter catalog intentionally stays small and lives in
`crates/rebecca-rules/rules/windows/`:

- `windows.user-temp`
- `windows.edge-cache`
- `windows.chrome-cache`
- `windows.brave-cache`
- `windows.firefox-profile-cache`
- `windows.discord-cache`
- `windows.slack-cache`
- `windows.steam-cache`
- `windows.steam-install-cache`
- `windows.steam-install-depot-cache`
- `windows.steam-install-download-cache`
- `windows.steam-install-library-cache`
- `windows.steam-install-shader-cache`
- `windows.steam-install-logs`
- `windows.steam-install-stats-cache`
- `windows.steam-install-appinfo-cache`
- `windows.steam-install-localization-cache`
- `windows.steam-install-packageinfo-cache`
- `windows.steam-library-shader-cache`
- `windows.steam-library-downloading-cache`
- `windows.steam-library-temp-cache`
- `windows.directx-shader-cache`
- `windows.thumbnail-cache`
- `windows.pip-cache`
- `windows.uv-cache`
- `windows.poetry-cache`
- `windows.conda-cache`
- `windows.go-build-cache`
- `windows.go-module-cache`
- `windows.cargo-cache`
- `windows.rustup-cache`
- `windows.sccache-cache`
- `windows.jetbrains-cache`
- `windows.npm-cache`
- `windows.pnpm-cache`
- `windows.yarn-cache`
- `windows.bun-cache`
- `windows.corepack-cache`
- `windows.gradle-cache`
- `windows.nuget-cache`
- `windows.maven-cache`
- `windows.vscode-cache`
- `windows.wer-reports`

Rule authoring notes live in [`docs/rule-authoring.md`](docs/rule-authoring.md).

Rule metadata includes platform, category, safety level, restore hint, and
provenance. Built-in rules use `source = "owned"` with
`license = "project-owned"`. Human `scan`, `clean`, and `history` views surface
restore hints when available, and the JSON forms preserve those fields for
script consumers. Human `history` output also summarizes the current history
window by result counts and cleanup bytes, and highlights the largest cleanup
runs for quick review. The catalog is embedded from TOML
files and validated before it reaches the CLI. Reference projects under
`repo-ref/` are research inputs; their GPL code and cleaner definitions are not
copied into Rebecca.
Chromium-family browser cache rules for Chrome, Edge, and Brave cover `Default`
and bounded `Profile *` directories when they exist.
Steam support currently discovers the install root from a small ordered set of
Windows registry locations, then library roots from `steamapps\libraryfolders.vdf`.
That lets future Steam rules target install-root-relative or library-root-relative
paths without guessing the machine layout.
The current catalog includes the Steam client web cache, Steam install-root
cache rules for `appcache\\httpcache`, `appcache\\download`,
`appcache\\librarycache`, `appcache\\shadercache`, `appcache\\stats`,
`appcache\\appinfo.vdf`, `appcache\\localization.vdf`,
`appcache\\packageinfo.vdf`, `config\\avatarcache`, `depotcache`, and `logs`,
plus Steam library shader-cache, downloading cache, and temp cache rules. The
Steam browser cache rule intentionally stays on
`Cache`, `Code Cache`, and `GPUCache` under `htmlcache\\Default` and does not
target `Local Storage`, `IndexedDB`, `Service Worker`, or `Network` state. The
Steam install-root cache rules stay limited to disposable subdirectories and do
not touch `userdata`, `steamapps`, or unlisted appcache metadata.

## App Leftovers

`rebecca apps scan` and `rebecca apps clean` provide a bounded app-residue
workflow. Rebecca reads installed-app inventory from Windows uninstall registry
locations and uses the display name only to derive conservative user-scoped
leftover cache targets under `AppData\Local`, `AppData\Roaming`, and
`AppData\LocalLow`.

The workflow deliberately does not uninstall applications, execute vendor
uninstallers, remove uninstall metadata, write registry keys, kill processes,
or delete broad `Program Files` or application data roots. It only routes
discovered app leftover cache directories such as `Cache`, `Code Cache`,
`GPUCache`, and `CachedData` through the same protection policy, dry-run
summary, issue matrix, Recycle Bin backend, and JSON/history model used by
regular cleanup.

## Project Artifacts

`rebecca purge` provides a Mole-inspired project cleanup workflow for heavy
build artifacts and dependency caches. The current scope is deliberately
directory-name based and high confidence: `node_modules`, `target`, `build`,
`dist`, Python virtual environments and tool caches, frontend framework caches,
coverage output, Gradle caches, Zig/Dart/Expo build caches, CocoaPods `Pods`,
Composer `vendor`, .NET `bin`/`obj`, plus directories carrying a valid
`CACHEDIR.TAG` cache marker. Ambiguous `vendor` and `bin` directories are
included only with strong project context: Composer `vendor` requires
`composer.json`, and .NET `bin` requires a sibling `.csproj`, `.fsproj`, or
`.vbproj` plus `Debug` or `Release` output.

Rebecca does not currently auto-scan every common projects directory under the
user profile. By default it scans configured `[purge].roots` when present and
falls back to the current directory when no roots are configured; pass repeated
`--root <PATH>` values to override configured roots for one run. Matching
artifact directories are pruned from traversal after discovery so nested
artifacts are not double-counted. Execution uses the same plan-first model as
`clean`: preview is the default, `--yes` is required to move targets to the
Windows Recycle Bin, and `--exclude` plus `[protection].protected_paths` can
block paths before size scanning or deletion. To avoid immediately cleaning
active build output, `purge` skips artifact directories modified within the last
7 days by default; pass `--min-age-days 0` to include recent artifacts
explicitly. Use repeated `--artifact <NAME>` values to include only selected
artifact kinds, using either the directory name such as `node_modules` or a rule
id suffix such as `target`; run `rebecca purge --list-artifacts` to print the
supported selector catalog without scanning. Human output groups artifact
targets by project path and labels each artifact type so large purge plans are
easier to scan.

Long-lived purge defaults belong in `config.toml`:

```toml
[purge]
roots = ['D:\SourceCodes', 'D:\Work']
max_depth = 6
min_age_days = 7
```

## Local State

By default, Rebecca uses standard Windows user directories:

- config: `%APPDATA%\Rebecca\config.toml`
- state: `%LOCALAPPDATA%\Rebecca\state`
- cache: `%LOCALAPPDATA%\Rebecca\cache`
- history: `%LOCALAPPDATA%\Rebecca\state\history.jsonl`

The full schema, path precedence, migration, and local-state ownership contract
is documented in [Configuration And Local State Contract](docs/configuration.md).

`rebecca config paths --json` also reports lifecycle metadata for these paths:

| Path | Lifecycle | Retention |
|------|-----------|-----------|
| config file / config dir | configuration | preserve |
| state dir | durable-state | preserve |
| history file | append-only-history | preserve |
| cache dir | rebuildable-cache | rebuildable |

`rebecca cache purge` operates only on Rebecca's configured rebuildable cache
directory. It previews by default, requires `--yes` to delete direct cache
contents, keeps the cache directory itself, reports lifecycle, entry-status,
and issue-matrix details in human output and JSON output, and refuses to run if
the cache path overlaps preserved configuration, state, or history paths.

Scan-cache records use a versioned JSON format under the rebuildable cache
directory's `scan` subdirectory. The current v1 contract stores the scanned
root path, a root metadata fingerprint, the scan report, and the write time.
`clean --scan-cache` explicitly enables planner use of eligible regular-file
records and freshness-bounded directory records. Directory freshness is
governed by a policy seam with a current 5-minute default, so the window can
evolve without changing the on-disk record format. Missing,
corrupted, stale, expired, or unsupported-version records are treated as cache
misses and can be rebuilt.

The config file can override that directory freshness window:

```toml
[scan_cache]
directory_record_max_age_seconds = 300
```

The config file is human-editable TOML. The current schema version is `1`; if
`version` is omitted, Rebecca treats the file as version `1`. Unsupported
versions fail clearly instead of being partially interpreted.

```toml
version = 1

[app_paths]
state_dir = 'D:\Rebecca\state'
cache_dir = 'D:\Rebecca\cache'
history_file = 'D:\Rebecca\state\history.jsonl'

[scan_cache]
directory_record_max_age_seconds = 300

[protection]
protected_paths = ['D:\Keep\Cache']
```

Every `app_paths` field is optional. Omitted fields keep the default Windows
user-directory location. Omitted `scan_cache` fields keep the default
directory-record freshness policy. Omitted `protection.protected_paths` means
no additional user-protected paths beyond Rebecca's built-in safety policy.

For tests or constrained environments, these paths can also be overridden:

- `REBECCA_CONFIG_DIR`
- `REBECCA_STATE_DIR`
- `REBECCA_CACHE_DIR`
- `REBECCA_HISTORY_FILE`

## Release Integrity

Rebecca's release workflow publishes a Windows x86_64 ZIP artifact and
`SHA256SUMS`, then generates GitHub build-provenance attestations for release
assets. Users should verify both the checksum and the attestation when the
GitHub CLI is available.

See [Release Integrity](docs/release.md) for the exact verification commands.

## Development

```powershell
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo bench -p rebecca-core --bench scan_baseline
.\scripts\release\build-release.ps1 -Tag v0.1.0 -OutDir target\release-smoke
.\scripts\release\write-checksums.ps1 -DistDir target\release-smoke
.\scripts\install.ps1 -AssetPath target\release-smoke\rebecca-0.1.0-windows-x86_64-msvc.zip -ChecksumsPath target\release-smoke\SHA256SUMS -InstallDir target\install-smoke -SkipAttestation
```
