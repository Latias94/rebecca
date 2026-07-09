# Rebecca

Rebecca is a cleanup CLI for caches, app leftovers, project artifacts, and disk usage inspection.

It is built around one simple habit: look first, delete second. `clean`, `purge`, `apps clean`, and the TUI all use the same planner, warning gates, protected path checks, system Trash or Recycle Bin behavior, and history log.

<p align="center">
  <a href="docs/security-audit.md">Safety audit</a> |
  <a href="docs/api/cli/v1/README.md">CLI API</a> |
  <a href="docs/release.md">Release integrity</a> |
  <a href="docs/rule-authoring.md">Rule authoring</a>
</p>

## Install

Windows users can install the latest GitHub release with the PowerShell installer:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Latias94/rebecca/releases/latest/download/rebecca-installer.ps1 | iex"
```

The installer downloads the matching release archive and installs `rebecca.exe`. Set `REBECCA_INSTALL_DIR` if you want a different install directory.

You can also install from crates.io:

```shell
cargo install rebecca --locked
```

Linux and macOS currently use `cargo install` or a source build until release archives are published for those platforms.

Use Rebecca as a Rust library:

```toml
[dependencies]
rebecca = "0.3"
```

The public `rebecca` crate exposes the supported API. The workspace crates under `crates/` are implementation packages and may move faster.

Install the Rebecca agent skill when you want an AI coding agent to use Rebecca's preview-first cleanup workflow:

```shell
rebecca skills install
```

The default target is `~/.agents/skills`. Use `--agent codex` for `$CODEX_HOME/skills` or `~/.codex/skills`, and use `--destination <SKILLS_DIR>` for another agent. Remove it with `rebecca skills remove`.

## Use it

Find where space went:

```powershell
rebecca inspect map --root . --top 20
rebecca inspect map --root . --top 20 --cleanup-advice
rebecca tui --root .
```

Preview cleanup. This does not delete anything:

```powershell
rebecca clean --dry-run
rebecca clean --dry-run --category browser
rebecca purge --dry-run --root .
rebecca purge --dry-run --root . --artifact target
```

Save a reviewed preview when you want to execute it later:

```powershell
rebecca clean --dry-run --category browser --save-plan cleanup-plan.json
rebecca plan inspect cleanup-plan.json
rebecca plan run cleanup-plan.json
rebecca plan run cleanup-plan.json --yes
```

`plan run` rechecks the current platform and target metadata first. If a target changed since the preview, Rebecca skips it and asks you to save a fresh plan.

Execute only after the preview looks right. Normal cleanup moves allowed targets to the system Trash or Windows Recycle Bin, so the space is pending until the trash is emptied:

```powershell
rebecca clean --yes --category browser
rebecca purge --yes --root . --artifact target
```

Write a cleanup receipt when you want an audit file for what happened:

```powershell
rebecca clean --yes --category browser --receipt cleanup-receipt.json
rebecca plan run cleanup-plan.json --yes --receipt cleanup-receipt.json
```

Bypass the trash only when you want irreversible deletion:

```powershell
rebecca clean --yes --permanent --category browser
rebecca purge --yes --permanent --root . --artifact target
```

Preview trash from Rebecca, then empty it only after review:

```powershell
rebecca trash empty
rebecca trash empty --yes
rebecca trash empty --drive E --yes
```

List the rules Rebecca knows about:

```powershell
rebecca catalog --kind cleanup-rule
rebecca catalog --kind project-artifact
```

The TUI shows a ranked disk map, treemap, type distribution, extension distribution, cleanup advice, staged rules, preview, execution, and history. Mouse input selects and navigates; cleanup still requires an explicit confirmation.

## What Rebecca cleans

Rebecca ships conservative built-in rules for:

- User cache and temp directories on Windows, Linux, and macOS.
- Browser caches for Chromium-family browsers, Firefox-family browsers, and Edge.
- Developer caches for Rust, Node.js, Python, Java, .NET, Android, JetBrains IDEs, VS Code, and common ML tooling.
- Desktop app caches and logs for chat, meeting, design, media, office, and download apps.
- Steam client cache and library cache leaves.
- Project artifacts such as `node_modules`, `target`, `build`, `dist`, Python virtual environments, coverage output, `.next`, Gradle output, Composer `vendor`, CocoaPods `Pods`, and .NET `bin` / `obj`.
- Windows app leftovers discovered from installed-app inventory.

Rules are discoverable:

```powershell
rebecca catalog --kind cleanup-rule --platform windows
rebecca catalog --kind cleanup-rule --platform linux
rebecca catalog --kind cleanup-rule --platform macos
rebecca catalog --kind project-artifact
```

Rule authoring and external cleaner manifests are documented in [docs/rule-authoring.md](docs/rule-authoring.md).

## Disk usage

`inspect map` answers "what is using space here?" without creating a cleanup plan.

```powershell
rebecca inspect map --root . --top 20
rebecca inspect map --root . --top 20 --metadata-profile logical-only
rebecca inspect map --root . --top 20 --group-by extension
rebecca inspect map --root . --top 20 --cleanup-advice
rebecca inspect map --root . --table csv --table-row entry --group-by extension
```

Human output is compact by default. Use `--full-path` for exact paths, `--no-bars` for plain logs, `--bar-width <COLUMNS>` for narrow or wide terminals, and `--screen-reader` for semicolon-separated lines without visual bars.

The default metadata profile is `full-evidence`, which asks the backend for the richest accounting it can provide. Use `--metadata-profile logical-only` for the quickest "what is big?" pass, `allocated` when physical disk usage matters, `unique` when hardlink deduplication matters, and `age-and-grouping` when you need grouping and cleanup-advice context without every backend evidence field.

On Windows, `--scan-backend windows-native` can use native directory enumeration and allocation metadata. Builds compiled with the `ntfs` Cargo feature also expose the experimental `windows-ntfs-mft-experimental` backend for read-only NTFS/MFT inventory. Unsupported or ambiguous cases fall back to the portable scanner with provenance.

## Project cleanup

`purge` is for build output and dependency directories. It checks project context before accepting broad names like `build`, `dist`, `bin`, or `obj`, so a random directory name is not enough.

```powershell
rebecca purge --dry-run --root .
rebecca purge --dry-run --root . --artifact node_modules
rebecca purge --dry-run --root . --min-age-days 0
rebecca purge --yes --root . --artifact target
```

By default, recent artifacts are skipped for 7 days. Use `--min-age-days 0` when you intentionally want to include fresh build output.

## Safety defaults

Rebecca is a local deletion tool, so the defaults are intentionally boring:

- Cleanup-capable commands preview first. Use `--dry-run` to make that explicit and `--yes` to execute.
- Allowed targets move to the platform trash by default.
- Moderate rules need `--allow-moderate`; risky and dangerous rules need `--allow-risky`.
- Warning-bearing rules stay blocked until you pass the named `--allow-warning <WARNING>` gate.
- Junctions, symlinks, and other reparse points are blocked by default.
- `--exclude <PATH>` and `[protection].protected_paths` keep paths out of a run.
- Permanent deletion is explicit with `--permanent`; administrator auto-elevation is not part of normal cleanup.

Do not start by running `sudo rebecca clean --yes`. Preview as the current user, review permission-sensitive targets, then elevate only for the specific system cache you intend to clean.

The longer destructive-operation boundary lives in [docs/security-audit.md](docs/security-audit.md). Report security issues through [SECURITY.md](SECURITY.md).

## Output for scripts

Use JSON when a caller needs one final result:

```powershell
rebecca clean --dry-run --format json
rebecca inspect map --root . --format json --top 20
```

Use NDJSON for long-running work that needs progress events:

```powershell
rebecca clean --dry-run --format ndjson
rebecca clean --yes --format ndjson --category browser
rebecca inspect map --root . --format ndjson --top 20
```

Dry-run NDJSON reports planning progress. Confirmed cleanup NDJSON also reports execution events for each target, including whether bytes were freed immediately or are pending in trash.

Use table export when the next tool is Excel, PowerShell, DuckDB, or a shell pipeline:

```powershell
rebecca inspect map --root . --table csv --table-row entry --top 100
rebecca inspect map --root . --table tsv --table-row group --group-by extension
```

The CLI contract, schemas, and examples live in [docs/api/cli/v1](docs/api/cli/v1/README.md).

## Shell completions

Rebecca generates completions from the live clap parser:

```powershell
rebecca completion powershell > rebecca.ps1
rebecca completion bash > rebecca.bash
rebecca completion zsh > _rebecca
rebecca completion fish > rebecca.fish
rebecca completion elvish > rebecca.elv
```

GitHub Releases also publish standalone completion assets with checksums. The Windows installer copies packaged completions to `<install-dir>\completions` when the archive contains them; it does not edit shell profiles.

## Local state

Rebecca stores config, history, and rebuildable cache data in the platform user directories:

- Windows config: `%APPDATA%\Rebecca\config.toml`
- Windows state/cache: `%LOCALAPPDATA%\Rebecca\state`, `%LOCALAPPDATA%\Rebecca\cache`
- Linux config: `$XDG_CONFIG_HOME/Rebecca/config.toml` or `$HOME/.config/Rebecca/config.toml`
- Linux state/cache: `$XDG_DATA_HOME/Rebecca/state`, `$XDG_CACHE_HOME/Rebecca/cache`, or their standard home-directory fallbacks
- macOS config/state/cache: platform user directories under `Library`

Inspect the resolved paths:

```powershell
rebecca config paths
rebecca cache doctor
rebecca history --limit 10
```

The full local state contract is in [docs/configuration.md](docs/configuration.md).

## Release integrity

GitHub releases publish the Windows x86_64 archive, installer, completions, and SHA-256 checksums. Manual verification commands and maintainer release steps are in [docs/release.md](docs/release.md).

## Development

```powershell
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
.\scripts\ci\run-linux-target-clippy.ps1
cargo nextest run --workspace
cargo bench -p rebecca-core --bench scan_baseline
```

## License

Rebecca is dual-licensed under MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
