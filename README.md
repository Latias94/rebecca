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
- Cleanup intelligence: `catalog` and `inspect` expose rules, warnings, safety categories, space reports, ranked disk maps, project artifact reports, and lint-style opportunities without deleting files.
- Windows app leftovers: `apps scan` and `apps clean` discover installed apps and target leftover cache data without uninstalling anything.
- Project artifact purge: `purge` targets heavy build output such as `node_modules`, `target`, `build`, `dist`, and `CACHEDIR.TAG` directories after verifying project context.
- Machine-readable output: JSON and NDJSON modes are available for wrappers, scripts, and automation, with CSV/TSV table export for disk maps.
- Recycle Bin by default: allowed targets are moved to the Windows Recycle Bin instead of being deleted permanently.
- Release integrity: release assets are checksum-backed and generated through cargo-dist.

## Quick Start

**Install from a GitHub release**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Latias94/rebecca/releases/download/v0.2.0/rebecca-installer.ps1 | iex"
```

The cargo-dist installer downloads the matching Windows artifact and installs `rebecca.exe`. Set `REBECCA_INSTALL_DIR` to override the install directory.

**Install from crates.io**

```powershell
cargo install rebecca --locked
```

**Use as a Rust library**

```toml
[dependencies]
rebecca = "0.1"
```

`rebecca` is the user-facing package. It exposes the curated library surface and the CLI binary while the implementation remains split across `rebecca-core`, `rebecca-rules`, and `rebecca-windows`.

**Run from source**

```powershell
cargo run -p rebecca -- scan
cargo run -p rebecca -- catalog
cargo run -p rebecca -- inspect space --root .
cargo run -p rebecca -- inspect map --root . --top 20 --max-depth 3
cargo run -p rebecca -- inspect map --root . --top 20 --full-path --bar-width 32
cargo run -p rebecca -- inspect map --root . --top 20 --cleanup-advice
cargo run -p rebecca -- inspect map --root . --table csv --table-row entry --top 20 --group-by extension
cargo run -p rebecca -- inspect artifacts --root .
cargo run -p rebecca -- inspect lint --root .
cargo run -p rebecca -- clean --dry-run
cargo run -p rebecca -- doctor active-processes
cargo run -p rebecca -- apps scan
cargo run -p rebecca -- purge --list-artifacts
cargo run -p rebecca -- cache doctor
```

**Preview safely**

```powershell
cargo run -p rebecca -- clean --dry-run
cargo run -p rebecca -- clean --dry-run --no-progress --rule windows.slack-cache --allow-warning active-process
cargo run -p rebecca -- clean --dry-run --format json --scan-cache --rule windows.thumbnail-cache
cargo run -p rebecca -- doctor active-processes
cargo run -p rebecca -- apps clean --dry-run
cargo run -p rebecca -- purge --dry-run
cargo run -p rebecca -- history --limit 10
cargo run -p rebecca -- history --format json
```

## Human Output Examples

Rebecca's human output is optimized for previewing risk, deciding the next command, and spotting the largest space users without opening a separate UI.

**Dry-run decision and active-process guidance**

```powershell
cargo run -p rebecca -- clean --dry-run --no-progress --rule windows.slack-cache --allow-warning active-process
```

```text
Decision: preview only; no files were deleted.
Reclaimable now: 9 (9 B)
Execution: would move allowed targets to the Recycle Bin.
Next command: rebecca clean --yes --rule windows.slack-cache --allow-warning active-process
Required opt-ins in next command: --allow-warning active-process.
Warning gates in plan: active-process.
Doctor hint: rebecca doctor active-processes

Target details:
allowed (3)
  - windows.slack-cache [...\Slack\Cache] 9 bytes (9 B) [warnings: active-process]
```

When a selected rule is not eligible yet, the preview explains what must change before execution:

```text
Execution: no eligible target would be deleted.
Resolve before execution:
- skipped safety-opt-in-required: add --allow-moderate or --allow-risky after reviewing the rule.

Issue matrix:
- skipped safety-opt-in-required: 2 targets, 0 (0 B)
```

**Ranked disk map**

```powershell
cargo run -p rebecca -- inspect map --root . --top 2 --entry-kind file --group-by extension
```

```text
Disk map
Roots: 1
Logical bytes: 10 (10 B)
Files: 2
Directories: 0
Diagnostics: 0

Top map entries:
  #1  8 bytes (8 B)  80.0% [################----] ...\workspace\large.bin [file depth=1] - 1 file, 0 dirs
  #2  2 bytes (2 B)  20.0% [####----------------] ...\workspace\small.txt [file depth=1] - 1 file, 0 dirs

Map groups:
  #1  8 bytes (8 B)  80.0% [################----] .bin [extension] - 1 file
  #2  2 bytes (2 B)  20.0% [####----------------] .txt [extension] - 1 file
```

Use `--full-path` for exact paths, `--no-bars` for plain logs, `--bar-width <COLUMNS>` for denser terminals, and `--screen-reader` for semicolon-separated lines without visual bars.

**Cache doctor**

```powershell
cargo run -p rebecca -- cache doctor
```

```text
Cache health: review recommended
Prunable records: 0

Rebecca cache: ...\rebecca-cache
Namespace: all
Entries: 0, valid: 0, stale: 0, corrupt: 0, missing payloads: 0, prunable: 0
Cache bytes: 0 (0 B)
No cache records found.
Recommendations:
- Info: No Rebecca cache records were found.
```

## Security & Safety Design

Rebecca is a local Windows cleanup tool, and the highest-risk behavior is unintended local data loss.

- `clean` previews by default; `clean --dry-run` makes that preview explicit, and `clean --yes` uses the same plan builder before moving allowed targets.
- `apps scan` and `apps clean` share the same planner. `apps clean` previews by default and requires `--yes` before moving leftover cache data to the Recycle Bin.
- `purge` uses a dedicated project-artifacts workflow. It scans configured roots when present, otherwise the current directory, and previews by default before moving project artifacts to the Recycle Bin.
- `catalog`, `inspect space`, `inspect map`, `inspect artifacts`, and `inspect lint` are read-only surfaces and never write cleanup history.
- Default execution uses the Windows Recycle Bin.
- Windows execution can batch already revalidated, non-overlapping targets into fewer Recycle Bin operations, but status, reason codes, pending bytes, and history remain per target.
- `clean --scan-backend windows-native` opts into the Windows native directory enumeration backend for plan estimates, and `inspect map --scan-backend windows-native` uses the same native entry metadata for ranked disk inventory on supported local paths; `windows-ntfs-mft-experimental` attempts read-only live NTFS/MFT metadata on supported local NTFS volumes, tries a sequential `$MFT` source before the per-record FSCTL source where a full index is explicitly requested, expands valid stream-backed `$INDEX_ALLOCATION:$I30` directory indexes through Rebecca's sequence-aware parser/index model, and falls back to a safe directory scanner with provenance when unsupported, unprivileged, too slow to index within the live build budget, or too ambiguous to trust. Set `REBECCA_NTFS_MFT_INDEX_TIMEOUT_SECONDS` to tune the default 20 second experimental MFT build budget, or `0` to disable that guard for dogfood; set `REBECCA_NTFS_MFT_INDEX_TIMINGS=1` to capture stage timings while profiling the experimental backend. `inspect space` and `inspect map` accept the same backend selector for read-only estimates or inventory. The default remains the portable cleanup walker.
- Directory targets keep the target directory and move direct child entries.
- Permanent deletion and administrator auto-elevation are not part of the MVP.
- Junctions, symlinks, and other reparse-point traversal are blocked by default.
- Moderate rules require `--allow-moderate`; risky and dangerous rules require `--allow-risky`.
- Use `--exclude <PATH>` or `[protection].protected_paths` to keep a path out of a run.
- Dry-run human output highlights the largest estimated targets first, groups the full target list by status, prints a copyable next command, lists required opt-ins already present in that command, explains skipped or blocked pre-execution issues, and points active-process warning runs at `rebecca doctor active-processes`.
- `clean --scan-cache` explicitly enables the rebuildable scan cache for eligible targets.
- Human `clean` runs show target-level progress by default and honor `Ctrl+C` for cancellation; use `--no-progress` for quiet logs. TTY progress stays on stderr with compact `plan | ...`, `scan | ...`, and `cache | ...` messages; `--progress-detail file` adds throttled file and byte throughput for long scans.
- `--format ndjson` keeps machine output clean for long-running cleanup workflows. It emits target-level progress by default; add `--progress-detail file` only when a wrapper needs per-file scan events.
- Warning-bearing cleanup rules are blocked until their named gate is selected with `--allow-warning <WARNING>`; `--allow-moderate` and `--allow-risky` still control safety-level admission.

For the current destructive-operation boundary and known safety gaps, see [Rebecca Cleanup Safety Audit](docs/security-audit.md).

Security reporting guidance lives in [SECURITY.md](SECURITY.md).

Reference material under `repo-ref/` is for behavior research only; Rebecca owns its rule catalog and implementation.

## Tips

- `clean`, `apps clean`, `purge`, and `cache purge` all preview first; `cache purge --yes` moves Rebecca cache entries to the Recycle Bin, and `cache purge --yes --permanent` opts into irreversible deletion.
- Use `catalog` before adding wrappers or scripts; it lists supported cleanup rules, project artifact selectors, warning gates, safety categories, and action kinds from one API.
- Use `inspect space`, `inspect map`, `inspect artifacts`, and `inspect lint` when you need reports rather than cleanup plans.
- Use `cache inspect`, `cache doctor`, and `cache prune` when you need Rebecca cache inventory, recommendations, or targeted stale-record cleanup.
- Use `apps scan` when you want to inspect installed-app leftovers, and `apps clean` when you are ready to move them to the Recycle Bin.
- Use `--format json` or `--format ndjson` when Rebecca is being driven by another tool.
- `history` is the fastest way to review what was planned and what actually happened.

## Usage

```powershell
cargo run -p rebecca -- scan
cargo run -p rebecca -- scan --format json
cargo run -p rebecca -- scan --category browser
cargo run -p rebecca -- scan --rule windows.thumbnail-cache

cargo run -p rebecca -- catalog
cargo run -p rebecca -- catalog --format json --kind warning
cargo run -p rebecca -- catalog --format json --kind project-artifact --artifact node-modules

cargo run -p rebecca -- inspect space --root .
cargo run -p rebecca -- inspect space --root . --format json --top 20
cargo run -p rebecca -- inspect map --root . --format json --top 20 --max-depth 3
cargo run -p rebecca -- inspect map --root . --top 20 --full-path --bar-width 32
cargo run -p rebecca -- inspect map --root . --top 20 --no-bars
cargo run -p rebecca -- inspect map --root . --top 20 --screen-reader
cargo run -p rebecca -- inspect map --root . --format json --top 20 --cleanup-advice
cargo run -p rebecca -- inspect map --root . --format ndjson --top 20 --advice-status cleanable
cargo run -p rebecca -- inspect map --root . --format ndjson --top 20 --group-by extension
cargo run -p rebecca -- inspect map --root . --table tsv --table-row entry --table-row group --top 20 --group-by extension
cargo run -p rebecca -- inspect artifacts --root . --format json
cargo run -p rebecca -- inspect artifacts --root . --artifact target --reclaim-limit-bytes 1073741824
cargo run -p rebecca -- inspect lint --root . --reference "$PWD\archive" --format json

cargo run -p rebecca -- clean --dry-run
cargo run -p rebecca -- clean --dry-run --format json --category system
cargo run -p rebecca -- clean --dry-run --no-progress --rule windows.edge-cache
cargo run -p rebecca -- clean --dry-run --format json --scan-cache --rule windows.thumbnail-cache
cargo run -p rebecca -- clean --dry-run --no-scan-cache --scan-backend windows-native --category system
cargo run -p rebecca -- clean --dry-run --no-scan-cache --scan-backend windows-ntfs-mft-experimental --category system
cargo run -p rebecca -- clean --dry-run --format json --allow-moderate --rule windows.npm-cache
cargo run -p rebecca -- clean --dry-run --format json --allow-risky --rule windows.npm-cache
cargo run -p rebecca -- clean --dry-run --exclude "$env:APPDATA\Slack\Cache"
cargo run -p rebecca -- clean --yes --category system

cargo run -p rebecca -- apps scan
cargo run -p rebecca -- apps scan --format json
cargo run -p rebecca -- apps scan --exclude "$env:LOCALAPPDATA\Example App\Cache"
cargo run -p rebecca -- apps clean
cargo run -p rebecca -- apps clean --format json --dry-run
cargo run -p rebecca -- apps clean --yes

cargo run -p rebecca -- purge
cargo run -p rebecca -- inspect artifacts --root . --format json
cargo run -p rebecca -- purge --list-artifacts
cargo run -p rebecca -- purge --list-artifacts --format json
cargo run -p rebecca -- purge --format json --root . --max-depth 6
cargo run -p rebecca -- purge --root . --min-age-days 0
cargo run -p rebecca -- purge --root . --artifact target
cargo run -p rebecca -- purge --root . --reclaim-limit-bytes 1073741824
cargo run -p rebecca -- purge --exclude "$PWD\target"
cargo run -p rebecca -- purge --yes --root . --scan-cache

cargo run -p rebecca -- completion powershell
cargo run -p rebecca -- completion bash
cargo run -p rebecca -- completion zsh

cargo run -p rebecca -- history
cargo run -p rebecca -- history --limit 10
cargo run -p rebecca -- history --format json

cargo run -p rebecca -- config paths
cargo run -p rebecca -- cache inspect --format json
cargo run -p rebecca -- cache doctor
cargo run -p rebecca -- cache doctor --format json
cargo run -p rebecca -- cache prune --format json --namespace scan-cache --stale-only
cargo run -p rebecca -- cache purge --format json
cargo run -p rebecca -- cache purge --yes
cargo run -p rebecca -- cache purge --yes --permanent
cargo run -p rebecca -- doctor permissions
```

## CLI API

Use `--format json` when a caller needs one final result.
Use `--format ndjson` for long-running cleanup workflows that need progress events. NDJSON defaults to target-level progress; use `--progress-detail file` for verbose per-file scan events. Use `inspect map --table csv|tsv` when a caller needs flat disk-map rows for Excel, PowerShell, DuckDB, or other table-first tools; add repeated `--table-row total|root|entry|group` when the caller only wants selected row kinds. Add `inspect map --cleanup-advice` when ranked entries should include read-only cleanup guidance grounded in Rebecca's rule catalog, project artifact policy, app-leftover discovery, and protection policy; `--advice-status cleanable|maybe-cleanable|contains-cleanable|protected|unknown` narrows ranked entries by that guidance without changing root totals.

Machine-readable success responses use the unified `rebecca.cli.v1` envelope. Every envelope includes `command`, `payload_kind`, `generated_at_unix_seconds`, and `data`. Fatal failures in JSON mode write a structured error envelope to stderr and exit non-zero.

Cleanup, purge, `inspect space`, and `inspect map` targets expose estimate provenance. `estimate_source` remains stable, while `estimate_backend`, optional `estimate_backend_source`, `estimate_confidence`, `estimate_fallback_reason`, and `estimate_caveats` explain backend selection, cache reuse, actual NTFS source selection, parser caveats, and fallback without changing deletion safety. `inspect map` reports path-ranked `logical_bytes`, nullable `allocated_bytes`, and nullable `unique_logical_bytes` / `unique_allocated_bytes` when a backend can deduplicate stable file identities. It can also emit bounded file distribution `groups` when requested with `--group-by extension`, `--group-by depth`, or `--group-by age`; use `--sort logical|allocated|files|unique` and `--group-sort logical|allocated|files|unique` when the most useful ordering is not logical bytes. In NDJSON mode, `inspect map` streams ranked `map-entry` and `map-group` events before the final full `inspect-map` completed event, which makes the same bounded map useful for scripts and future TUI views. With `--cleanup-advice`, ranked map entries include `cleanup_advice` with `cleanable`, `maybe-cleanable`, `contains-cleanable`, `protected`, or `unknown` status plus matched rule, project artifact, app-leftover, or protection facts and a dry-run command hint; actual deletion still goes through `clean`, `apps clean`, or `purge` planning. In table mode, `inspect map --table csv|tsv` writes flat `total`, `root`, `entry`, and `group` rows with one header and no JSON envelope; empty cells mean the column is not applicable to that row type or the metric is unknown, repeated `--table-row` flags can narrow the row set, and `--cleanup-advice` appends cleanup advice columns plus `cleanup_app_*` columns for app-leftover context. Windows native map inventory fills `allocated_bytes` from Windows file allocation metadata when available, deduplicates hardlinked files by native file id for the unique fields, and caveats compressed, sparse, hardlinked, or skipped reparse entries. Portable inventory leaves allocation and unique accounting unknown. Experimental NTFS/MFT map inventory fills logical, allocated, unique logical, and unique allocated values from parser-backed record and stream evidence when available, keeps unknowns nullable, preserves directory-edge provenance from `$FILE_NAME` and `$I30`, and reports parser caveats instead of treating ambiguous metadata as deletion authority.

Human `inspect map` output ranks top entries and requested groups with logical-size share and ASCII usage bars. Use `--full-path` when terminal width is less important than exact paths, `--no-bars` for plain logs, `--bar-width <COLUMNS>` for denser or wider maps, and `--screen-reader` for semicolon-separated lines without visual bars.

The CLI API contract, schemas, and examples live in [docs/api/cli/v1](docs/api/cli/v1/README.md).

## Built-In Rules

Rebecca ships a conservative Windows catalog under `crates/rebecca-rules/rules/windows/`.

- System and browser caches: temp files, Edge, Chrome, Chromium, Brave, Firefox, Waterfox, Zen Browser, thumbnail cache, DirectX shader cache, and Windows Error Reporting data.
- App caches and diagnostics: Discord, Slack, Postman, Notion, Figma, Zoom logs, TeamViewer logs, VLC media cache, Thunderbird cache, Adobe Reader cache, WeChat, Enterprise WeChat, QQ, Feishu, DingTalk, WPS, Baidu Netdisk, Tencent Meeting, QQ Music, and Tencent Video.
- Developer caches: pip, uv, Poetry, Conda, Go, Cargo, ccache, rustup, sccache, JetBrains, npm, pnpm, yarn, bun, corepack, Gradle, Android, NuGet, Maven, and VS Code.
- Steam caches: the Steam client cache plus install-root and library-root cache leaves.

Rule metadata includes platform, category, safety level, restore hint, and provenance. Built-in rules use `source = "owned"` with `license = "project-owned"`. Human `scan`, `clean`, and `history` views surface restore hints when available, and `--format json` preserves those fields under the CLI API envelope.

Rule authoring notes live in [docs/rule-authoring.md](docs/rule-authoring.md).

## App Leftovers

`rebecca apps scan` and `rebecca apps clean` provide a bounded app-residue workflow. Rebecca reads installed-app inventory from Windows uninstall registry locations and uses the display name only to derive conservative user-scoped leftover cache targets under `AppData\Local`, `AppData\Roaming`, and `AppData\LocalLow`.

The workflow does not uninstall applications, execute vendor uninstallers, remove uninstall metadata, write registry keys, kill processes, or delete broad `Program Files` or application data roots. It only routes discovered app leftover cache directories such as `Cache`, `Code Cache`, `GPUCache`, and `CachedData` through the same protection policy, dry-run summary, issue matrix, Recycle Bin backend, and JSON/history model used by regular cleanup.

## Project Artifacts

`rebecca purge` provides a Mole-inspired project cleanup workflow for heavy build artifacts and dependency caches.

The current scope is context-sensitive rather than basename-only: `node_modules`, `target`, `build`, `dist`, Python virtual environments and tool caches, frontend framework caches, coverage output, Gradle caches, Zig/Dart/Expo build caches, CocoaPods `Pods`, Composer `vendor`, .NET `bin`/`obj`, plus directories carrying a valid `CACHEDIR.TAG` cache marker. Each built-in artifact is backed by an explicit project-context rule, such as JavaScript workspace markers for `node_modules`, Rust or Maven markers for `target`, Composer `composer.json` for `vendor`, and sibling `.csproj`, `.fsproj`, or `.vbproj` files for .NET `bin`/`obj`; generic names such as `build`, `dist`, `coverage`, `bin`, and `obj` are ignored without that context.

Rebecca does not auto-scan every common project directory under the user profile. By default it scans configured `[purge].roots` when present and falls back to the current directory when no roots are configured; pass repeated `--root <PATH>` values to override configured roots for one run. Explicit `--root` values are strict and fail if the path is missing, not a directory, or a reparse point. Configured roots are resolved as long-lived workspace intent, so a missing or unreadable configured root is reported as a project-artifact discovery diagnostic instead of aborting the whole run. Known artifact directory names are traversal boundaries even when the directory is not accepted as a cleanup target, which prevents embedded toolchains or installed products from leaking nested `build`, `dist`, `node_modules`, or bytecode caches into the plan. Execution uses the same plan-first model as `clean`: preview is the default, `--yes` is required to move targets to the Windows Recycle Bin, and `--exclude` plus `[protection].protected_paths` can block paths before size scanning or deletion.

Machine-readable purge targets include a `project_artifact` explanation object with the matched context, project root, and anchor path that made the target eligible. For example, a `node_modules` target matched by `package.json` reports `matched_context = "node-project"` and the concrete `project_anchor` path rather than a confidence score.

Project artifact plans may also include `discovery_diagnostics` for partial discovery failures such as missing configured roots, unreadable directories, metadata errors, or skipped reparse points. These diagnostics are plan-level observations; they do not create fake cleanup targets or change target counts.

To avoid immediately cleaning active build output, `purge` skips artifact directories modified within the last 7 days by default; pass `--min-age-days 0` to include recent artifacts explicitly. Use repeated `--artifact <NAME>` values to include only selected artifact kinds, using either the directory name such as `node_modules` or a rule id suffix such as `target`; run `rebecca catalog --kind project-artifact` for the canonical selector catalog, or `rebecca purge --list-artifacts` for the legacy purge-specific listing. Pass `--reclaim-limit-bytes <BYTES>` when you want Rebecca to measure ranked eligible artifacts until a reclaim target is met, leaving later candidates unmeasured. Human output groups artifact targets by project path and labels each artifact type so large purge plans are easier to scan.

Use `rebecca inspect artifacts` when you want a read-only project artifact report rather than a cleanup plan. It uses the same selectors, roots, excludes, depth, age window, scan-cache estimation, warning gates, reclaim limit, and diagnostics as `purge`, but it has no `--yes`, never prompts, and never writes cleanup history. Its machine payload is `inspect-artifacts`, grouped by scan root, project root, and artifact kind with a largest-targets list for dashboards or wrappers. `rebecca purge inspect` is retained as a legacy compatibility alias for this report.

`rebecca inspect space` provides read-only top-level disk usage insight with bounded top entries and no cleanup authorization. Raw diagnostic samples are bounded by `--diagnostic-limit` while `diagnostic_summary` keeps complete counts; use `--diagnostic-limit 0` for summary-only machine output.

`rebecca inspect lint` provides report-only duplicate, large-file, empty-file, and empty-directory findings. It computes conservative reclaim estimates, treats `--reference` roots and protected paths as keep candidates, and intentionally does not perform duplicate remediation or write cleanup history.

`rebecca inspect map` provides a read-only ranked disk map for a requested root. It is designed for "what is using space here?" questions, returns bounded top entries with `--top` and optional `--max-depth`, and never creates cleanup plans or authorizes deletion. The default portable inventory streams directory aggregation into a bounded top-entry heap instead of building a full report tree in memory, and it returns conservative partial diagnostics when child entries disappear, child directories are unreadable, or child reparse points are skipped. Raw diagnostic samples are bounded by `--diagnostic-limit` while `diagnostic_summary` keeps complete counts; use `--diagnostic-limit 0` for summary-only machine output. Use `--min-logical-bytes`, `--entry-kind file|directory|other`, and `--path-contains` to filter ranked entries without changing root totals. Use `--cleanup-advice` to annotate ranked entries with read-only cleanup status from the rule catalog, project artifact policy, app-leftover discovery, and protection policy; use `--advice-status cleanable|maybe-cleanable|contains-cleanable|protected|unknown` to keep only entries with one status. Use `--sort logical|allocated|files|unique` when file count, allocation, or unique logical usage is a better top-entry ordering than logical bytes. Use repeated `--group-by extension|depth|age` plus `--group-limit` to add bounded file distribution groups without running a second scan, and `--group-sort logical|allocated|files|unique` to rank those groups. Human output compacts long top-entry paths by default and can be tuned with `--full-path`, `--no-bars`, `--bar-width <COLUMNS>`, or `--screen-reader`. Use `--table csv|tsv` to export one flat table containing `total`, `root`, `entry`, and `group` rows for spreadsheet or query tooling; use repeated `--table-row total|root|entry|group` to export only selected row kinds. Use `--scan-backend windows-native` when you want Windows native directory enumeration provenance, file allocation bytes, and file-id-deduplicated unique bytes when the host API exposes them. Use `--scan-backend windows-ntfs-mft-experimental` when you want read-only NTFS/MFT provenance and MFT-native grouped disk maps; scoped roots use targeted traversal, while drive roots or explicit full-index diagnostics may use full-volume MFT inventory.

Long-lived purge defaults belong in `config.toml`:

```toml
[purge]
roots = ['D:\SourceCodes', 'D:\Work']
max_depth = 6
min_age_days = 7
```

Reference source trees such as this repository's `repo-ref` directory should be protected by configuration or a one-off exclude instead of product-specific hardcoding:

```powershell
rebecca purge --root . --exclude "$PWD\repo-ref"
```

```toml
[protection]
protected_paths = ['D:\SourceCodes\Rust\rebecca\repo-ref'] # replace with your checkout path
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

`rebecca cache inspect` inventories Rebecca-owned rebuildable cache metadata without deleting anything. Use `--namespace scan-cache`, `--namespace ntfs-volume-index`, or `--namespace all` to narrow the report. `rebecca cache doctor` adds stale/corrupt/missing-payload recommendations, and `rebecca cache prune --stale-only` previews targeted cache metadata cleanup before `--yes` executes it through the same cleanup execution report model as other deletion workflows. JSON inventory entries include `absolute_path` for local authority and `display_path` for safer reports; review absolute local paths before sharing diagnostics.

`rebecca cache purge` operates only on Rebecca's configured rebuildable cache directory. It previews by default, moves direct cache contents to the Recycle Bin with `--yes`, permanently deletes them only with `--yes --permanent`, keeps the cache directory itself, reports lifecycle, entry-status, pending-reclaim, reclaimed-byte, and issue-matrix details in human output and `--format json`, and refuses to run if the cache path overlaps preserved configuration, state, or history paths.

Scan-cache records use a versioned JSON format under the rebuildable cache directory's `scan` subdirectory. The current v1 record stores the scanned root path, root metadata fingerprint, scan report, write time, scan backend, optional backend source, estimate confidence, and optional filesystem identity fields for USN-based invalidation. `clean --scan-cache` explicitly enables planner use of eligible regular-file records and freshness-bounded directory records. Exact records from the portable, Windows native, and experimental NTFS/MFT backends can be reused when the root fingerprint and identity still match. Directory freshness is governed by a policy seam with a current 5-minute default, so the window can evolve without changing user configuration. Missing USN support falls back to the normal fingerprint and identity policy; mismatched journal ids, unavailable journal ranges, or target-subtree changes conservatively invalidate the cache. Missing, corrupted, stale, expired, older-format, or unsupported-version records are treated as cache misses and can be rebuilt. Stale or corrupted cache files are pruned when lookup discovers them, and plan builds also run a best-effort cache prune pass that reports pruned record counts in human output.

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

Rebecca's release workflow publishes a cargo-dist Windows x86_64 ZIP artifact, a PowerShell installer, and SHA-256 checksum files. A separate crates.io workflow publishes the workspace crates in dependency order, including the user-facing `rebecca` package. Users installing from GitHub Releases should verify the checksum when downloading artifacts manually.

See [Release Integrity](docs/release.md) for the exact verification commands.

## License

Rebecca is dual-licensed under MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

## Development

```powershell
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo bench -p rebecca-core --bench scan_baseline
.\scripts\release\build-release.ps1 -Tag v0.2.0 -OutDir target\release-smoke
.\scripts\release\write-sbom.ps1 -Tag v0.2.0 -DistDir target\release-smoke
.\scripts\release\write-checksums.ps1 -DistDir target\release-smoke
.\scripts\install.ps1 -AssetPath target\release-smoke\rebecca-0.2.0-windows-x86_64-msvc.zip -ChecksumsPath target\release-smoke\SHA256SUMS -InstallDir target\install-smoke -SkipAttestation
```
