<div align="center">
  <h1>Rebecca</h1>
  <p><em>Windows-first cleanup CLI for caches, app leftovers, and project artifacts.</em></p>
</div>

<p align="center">
  <a href="docs/security-audit.md">Safety audit</a> ·
  <a href="docs/api/cli/v1/README.md">CLI API v1</a> ·
  <a href="docs/release.md">Release integrity</a> ·
  <a href="docs/rule-authoring.md">Rule authoring</a>
</p>

> Rebecca is built to preview first and execute second. The same planner, protection policy, and history model cover the supported cleanup surfaces.

## Features

- Safe cleanup planning: `scan` and `clean` share the same plan builder, so dry-run output and real execution stay aligned.
- Windows app leftovers: `apps scan` and `apps clean` discover installed apps and target leftover cache data without uninstalling anything.
- Project artifact purge: `purge` targets heavy build output such as `node_modules`, `target`, `build`, `dist`, and `CACHEDIR.TAG` directories.
- Machine-readable output: JSON and NDJSON modes are available for wrappers, scripts, and automation.
- Recycle Bin by default: allowed targets are moved to the Windows Recycle Bin instead of being deleted permanently.
- Release integrity: release assets are checksum-backed and provenance-backed.

## Quick Start

**Install from a GitHub release**

```powershell
.\scripts\install.ps1 -Repository OWNER/REPO
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0
.\scripts\install.ps1 -Repository OWNER/REPO -Tag v0.1.0 -RequireAttestation
```

The default install directory is `%LOCALAPPDATA%\Programs\Rebecca`. Re-run the same command with a newer tag to update the installed `rebecca.exe`. The installer verifies `SHA256SUMS` before extraction; `-RequireAttestation` also requires GitHub CLI build-provenance verification.

**Run from source**

```powershell
cargo run -p rebecca-cli -- scan
cargo run -p rebecca-cli -- clean --dry-run
cargo run -p rebecca-cli -- apps scan
cargo run -p rebecca-cli -- purge --list-artifacts
```

**Preview safely**

```powershell
cargo run -p rebecca-cli -- clean --dry-run
cargo run -p rebecca-cli -- clean --dry-run --format json --scan-cache --rule windows.thumbnail-cache
cargo run -p rebecca-cli -- apps clean --dry-run
cargo run -p rebecca-cli -- purge --dry-run
cargo run -p rebecca-cli -- history --limit 10
cargo run -p rebecca-cli -- history --format json
```

## Security & Safety Design

Rebecca is a local Windows cleanup tool, and the highest-risk behavior is unintended local data loss.

- `clean` previews by default; `clean --dry-run` makes that preview explicit, and `clean --yes` uses the same plan builder before moving allowed targets.
- `apps scan` and `apps clean` share the same planner. `apps clean` previews by default and requires `--yes` before moving leftover cache data to the Recycle Bin.
- `purge` uses a dedicated project-artifacts workflow. It scans configured roots when present, otherwise the current directory, and previews by default before moving project artifacts to the Recycle Bin.
- Default execution uses the Windows Recycle Bin.
- Directory targets keep the target directory and move direct child entries.
- Permanent deletion and administrator auto-elevation are not part of the MVP.
- Junctions, symlinks, and other reparse-point traversal are blocked by default.
- Moderate rules require `--allow-moderate`; risky and dangerous rules require `--allow-risky`.
- Use `--exclude <PATH>` or `[protection].protected_paths` to keep a path out of a run.
- Dry-run human output highlights the largest estimated targets first and then groups the full target list by status.
- `clean --scan-cache` explicitly enables the rebuildable scan cache for eligible targets.
- Human `clean` runs show progress by default and honor `Ctrl+C` for cancellation; use `--no-progress` for quiet logs.
- `--format ndjson` keeps machine output clean for long-running cleanup workflows.

For the current destructive-operation boundary and known safety gaps, see [Rebecca Cleanup Safety Audit](docs/security-audit.md).

Security reporting guidance lives in [SECURITY.md](SECURITY.md).

Reference material under `repo-ref/` is for behavior research only; Rebecca owns its rule catalog and implementation.

## Tips

- `clean`, `apps clean`, `purge`, and `cache purge` all preview first; use `--dry-run` before confirming a real run.
- Use `apps scan` when you want to inspect installed-app leftovers, and `apps clean` when you are ready to move them to the Recycle Bin.
- Use `--format json` or `--format ndjson` when Rebecca is being driven by another tool.
- `history` is the fastest way to review what was planned and what actually happened.

## Usage

```powershell
cargo run -p rebecca-cli -- scan
cargo run -p rebecca-cli -- scan --format json
cargo run -p rebecca-cli -- scan --category browser
cargo run -p rebecca-cli -- scan --rule windows.thumbnail-cache

cargo run -p rebecca-cli -- clean --dry-run
cargo run -p rebecca-cli -- clean --dry-run --format json --category system
cargo run -p rebecca-cli -- clean --dry-run --no-progress --rule windows.edge-cache
cargo run -p rebecca-cli -- clean --dry-run --format json --scan-cache --rule windows.thumbnail-cache
cargo run -p rebecca-cli -- clean --dry-run --format json --allow-moderate --rule windows.npm-cache
cargo run -p rebecca-cli -- clean --dry-run --format json --allow-risky --rule windows.npm-cache
cargo run -p rebecca-cli -- clean --dry-run --exclude "$env:APPDATA\Slack\Cache"
cargo run -p rebecca-cli -- clean --yes --category system

cargo run -p rebecca-cli -- apps scan
cargo run -p rebecca-cli -- apps scan --format json
cargo run -p rebecca-cli -- apps scan --exclude "$env:LOCALAPPDATA\Example App\Cache"
cargo run -p rebecca-cli -- apps clean
cargo run -p rebecca-cli -- apps clean --format json --dry-run
cargo run -p rebecca-cli -- apps clean --yes

cargo run -p rebecca-cli -- purge
cargo run -p rebecca-cli -- purge --list-artifacts
cargo run -p rebecca-cli -- purge --list-artifacts --format json
cargo run -p rebecca-cli -- purge --format json --root . --max-depth 6
cargo run -p rebecca-cli -- purge --root . --min-age-days 0
cargo run -p rebecca-cli -- purge --root . --artifact target
cargo run -p rebecca-cli -- purge --exclude "$PWD\target"
cargo run -p rebecca-cli -- purge --yes --root . --scan-cache

cargo run -p rebecca-cli -- completion powershell
cargo run -p rebecca-cli -- completion bash
cargo run -p rebecca-cli -- completion zsh

cargo run -p rebecca-cli -- history
cargo run -p rebecca-cli -- history --limit 10
cargo run -p rebecca-cli -- history --format json

cargo run -p rebecca-cli -- config paths
cargo run -p rebecca-cli -- cache purge --format json
cargo run -p rebecca-cli -- cache purge --yes
cargo run -p rebecca-cli -- doctor permissions
```

## CLI API

Use `--format json` when a caller needs one final result.
Use `--format ndjson` for long-running cleanup workflows that need progress events.

Every machine-readable success response is wrapped in a `rebecca.cli.v1` envelope with `command`, `payload_kind`, `generated_at_unix_seconds`, and `data`. Fatal failures in JSON mode write a structured error envelope to stderr and exit non-zero.

The contract, schemas, and examples live in [docs/api/cli/v1](docs/api/cli/v1/README.md).

## Built-In Rules

Rebecca ships a conservative Windows catalog under `crates/rebecca-rules/rules/windows/`.

- System and browser caches: temp files, Edge, Chrome, Brave, Firefox profile cache, thumbnail cache, DirectX shader cache, and Windows Error Reporting data.
- App caches: Discord, Slack, Postman, Notion, Figma, WeChat, Enterprise WeChat, QQ, Feishu, DingTalk, WPS, Baidu Netdisk, Tencent Meeting, QQ Music, and Tencent Video.
- Developer caches: pip, uv, Poetry, Conda, Go, Cargo, ccache, rustup, sccache, JetBrains, npm, pnpm, yarn, bun, corepack, Gradle, Android, NuGet, Maven, and VS Code.
- Steam caches: the Steam client cache plus install-root and library-root cache leaves.

Rule metadata includes platform, category, safety level, restore hint, and provenance. Built-in rules use `source = "owned"` with `license = "project-owned"`. Human `scan`, `clean`, and `history` views surface restore hints when available, and `--format json` preserves those fields under the CLI API envelope.

Rule authoring notes live in [docs/rule-authoring.md](docs/rule-authoring.md).

## App Leftovers

`rebecca apps scan` and `rebecca apps clean` provide a bounded app-residue workflow. Rebecca reads installed-app inventory from Windows uninstall registry locations and uses the display name only to derive conservative user-scoped leftover cache targets under `AppData\Local`, `AppData\Roaming`, and `AppData\LocalLow`.

The workflow does not uninstall applications, execute vendor uninstallers, remove uninstall metadata, write registry keys, kill processes, or delete broad `Program Files` or application data roots. It only routes discovered app leftover cache directories such as `Cache`, `Code Cache`, `GPUCache`, and `CachedData` through the same protection policy, dry-run summary, issue matrix, Recycle Bin backend, and JSON/history model used by regular cleanup.

## Project Artifacts

`rebecca purge` provides a Mole-inspired project cleanup workflow for heavy build artifacts and dependency caches.

The current scope is deliberately directory-name based and high confidence: `node_modules`, `target`, `build`, `dist`, Python virtual environments and tool caches, frontend framework caches, coverage output, Gradle caches, Zig/Dart/Expo build caches, CocoaPods `Pods`, Composer `vendor`, .NET `bin`/`obj`, plus directories carrying a valid `CACHEDIR.TAG` cache marker. Ambiguous `vendor` and `bin` directories are included only with strong project context: Composer `vendor` requires `composer.json`, and .NET `bin` requires a sibling `.csproj`, `.fsproj`, or `.vbproj` plus `Debug` or `Release` output.

Rebecca does not auto-scan every common project directory under the user profile. By default it scans configured `[purge].roots` when present and falls back to the current directory when no roots are configured; pass repeated `--root <PATH>` values to override configured roots for one run. Matching artifact directories are pruned from traversal after discovery so nested artifacts are not double-counted. Execution uses the same plan-first model as `clean`: preview is the default, `--yes` is required to move targets to the Windows Recycle Bin, and `--exclude` plus `[protection].protected_paths` can block paths before size scanning or deletion.

To avoid immediately cleaning active build output, `purge` skips artifact directories modified within the last 7 days by default; pass `--min-age-days 0` to include recent artifacts explicitly. Use repeated `--artifact <NAME>` values to include only selected artifact kinds, using either the directory name such as `node_modules` or a rule id suffix such as `target`; run `rebecca purge --list-artifacts` to print the supported selector catalog without scanning. Human output groups artifact targets by project path and labels each artifact type so large purge plans are easier to scan.

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

The full schema, path precedence, migration, and local-state ownership contract is documented in [Configuration And Local State Contract](docs/configuration.md).

`rebecca config paths --format json` also reports lifecycle metadata for these paths inside the CLI API v1 `data` payload:

| Path | Lifecycle | Retention |
|------|-----------|-----------|
| config file / config dir | configuration | preserve |
| state dir | durable-state | preserve |
| history file | append-only-history | preserve |
| cache dir | rebuildable-cache | rebuildable |

`rebecca cache purge` operates only on Rebecca's configured rebuildable cache directory. It previews by default, requires `--yes` to delete direct cache contents, keeps the cache directory itself, reports lifecycle, entry-status, and issue-matrix details in human output and `--format json`, and refuses to run if the cache path overlaps preserved configuration, state, or history paths.

Scan-cache records use a versioned JSON format under the rebuildable cache directory's `scan` subdirectory. The current v1 contract stores the scanned root path, a root metadata fingerprint, the scan report, and the write time. `clean --scan-cache` explicitly enables planner use of eligible regular-file records and freshness-bounded directory records. Directory freshness is governed by a policy seam with a current 5-minute default, so the window can evolve without changing the on-disk record format. Missing, corrupted, stale, expired, or unsupported-version records are treated as cache misses and can be rebuilt. Stale or corrupted cache files are pruned when lookup discovers them, and plan builds also run a best-effort cache prune pass that reports pruned record counts in human output.

The config file can override that directory freshness window:

```toml
[scan_cache]
directory_record_max_age_seconds = 300
```

The config file is human-editable TOML. The current schema version is `1`; if `version` is omitted, Rebecca treats the file as version `1`. Unsupported versions fail clearly instead of being partially interpreted.

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

Every `app_paths` field is optional. Omitted fields keep the default Windows user-directory location. Omitted `scan_cache` fields keep the default directory-record freshness policy. Omitted `protection.protected_paths` means no additional user-protected paths beyond Rebecca's built-in safety policy.

For tests or constrained environments, these paths can also be overridden:

- `REBECCA_CONFIG_DIR`
- `REBECCA_STATE_DIR`
- `REBECCA_CACHE_DIR`
- `REBECCA_HISTORY_FILE`

## Release Integrity

Rebecca's release workflow publishes a Windows x86_64 ZIP artifact, an SPDX SBOM, and `SHA256SUMS`, then generates GitHub build-provenance attestations for release assets. Users should verify both the checksum and the attestation when the GitHub CLI is available.

See [Release Integrity](docs/release.md) for the exact verification commands.

## Development

```powershell
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo bench -p rebecca-core --bench scan_baseline
.\scripts\release\build-release.ps1 -Tag v0.1.0 -OutDir target\release-smoke
.\scripts\release\write-sbom.ps1 -Tag v0.1.0 -DistDir target\release-smoke
.\scripts\release\write-checksums.ps1 -DistDir target\release-smoke
.\scripts\install.ps1 -AssetPath target\release-smoke\rebecca-0.1.0-windows-x86_64-msvc.zip -ChecksumsPath target\release-smoke\SHA256SUMS -InstallDir target\install-smoke -SkipAttestation
```
