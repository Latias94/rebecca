# Rebecca

Rebecca is a Windows-first cleanup CLI written in Rust. It focuses on a safe,
plan-first cleanup flow for system junk and application caches.

The current MVP supports:

- listing built-in cleanup rules,
- building dry-run cleanup plans,
- scanning target sizes,
- blocking dangerous paths before execution,
- moving allowed files and allowed directory contents to the Windows Recycle Bin,
- recording cleanup history as JSONL.

## Safety Model

Rebecca is designed to preview before deleting.

- `clean --dry-run` and real cleanup use the same plan builder.
- Default execution uses the Windows Recycle Bin.
- Directory targets keep the target directory and move direct child entries.
- Permanent deletion and administrator auto-elevation are not part of the MVP.
- Junctions, symlinks, and other reparse-point traversal are blocked by default.
- Moderate rules require `--allow-moderate`; risky and dangerous rules require `--allow-risky`.
- Dry-run human output highlights the largest estimated targets first and then
  groups the full target list by status.
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
cargo run -p rebecca-cli -- clean --yes --category system

cargo run -p rebecca-cli -- history
cargo run -p rebecca-cli -- history --json

cargo run -p rebecca-cli -- config paths
cargo run -p rebecca-cli -- cache purge --json
cargo run -p rebecca-cli -- cache purge --yes
cargo run -p rebecca-cli -- doctor permissions
cargo run -p rebecca-cli -- doctor steam
```

## Built-In Rules

The starter catalog intentionally stays small and lives in
`crates/rebecca-rules/rules/windows/`:

- `windows.user-temp`
- `windows.edge-cache`
- `windows.chrome-cache`
- `windows.firefox-profile-cache`
- `windows.discord-cache`
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
- `windows.cargo-cache`
- `windows.jetbrains-cache`
- `windows.npm-cache`
- `windows.vscode-cache`
- `windows.wer-reports`

Rule authoring notes live in [`docs/rule-authoring.md`](docs/rule-authoring.md).

Rule metadata includes platform, category, safety level, delete policy, restore
hint, and provenance. Built-in rules use `source = "owned"` with
`license = "project-owned"`. Human `scan`, `clean`, and `history` views surface
restore hints when available, and the JSON forms preserve those fields for
script consumers. The catalog is embedded from TOML files and validated before
it reaches the CLI. Reference projects under `repo-ref/` are research inputs;
their GPL code and cleaner definitions are not copied into Rebecca.
Chromium-family browser cache rules cover `Default` and bounded `Profile *`
directories when they exist.
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

## Local State

By default, Rebecca uses standard Windows user directories:

- config: `%APPDATA%\Rebecca\config.toml`
- state: `%LOCALAPPDATA%\Rebecca\state`
- cache: `%LOCALAPPDATA%\Rebecca\cache`
- history: `%LOCALAPPDATA%\Rebecca\state\history.jsonl`

`rebecca config paths --json` also reports lifecycle metadata for these paths:

| Path | Lifecycle | Retention |
|------|-----------|-----------|
| config file / config dir | configuration | preserve |
| state dir | durable-state | preserve |
| history file | append-only-history | preserve |
| cache dir | rebuildable-cache | rebuildable |

`rebecca cache purge` operates only on Rebecca's configured rebuildable cache
directory. It previews by default, requires `--yes` to delete direct cache
contents, keeps the cache directory itself, reports lifecycle and entry-status
counts in human output, and refuses to run if the cache path overlaps preserved
configuration, state, or history paths.

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
```

Every `app_paths` field is optional. Omitted fields keep the default Windows
user-directory location. Omitted `scan_cache` fields keep the default
directory-record freshness policy.

For tests or constrained environments, these paths can also be overridden:

- `REBECCA_CONFIG_DIR`
- `REBECCA_STATE_DIR`
- `REBECCA_CACHE_DIR`
- `REBECCA_HISTORY_FILE`

## Development

```powershell
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo bench -p rebecca-core --bench scan_baseline
```
