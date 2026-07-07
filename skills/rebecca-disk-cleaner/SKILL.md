---
name: rebecca-disk-cleaner
description: Use when the user wants to install Rebecca, clean disk space with Rebecca, inspect large folders, remove safe caches, purge project artifacts, clean app leftovers, or run a preview-first cleanup workflow.
---

# Rebecca Disk Cleaner

Use Rebecca as the cleanup authority. The workflow is **preview-first**: inspect,
preview, ask for a numbered confirmation, then execute only the confirmed
Rebecca command.

## Steps

1. Establish scope.
   - Identify the target root, drive, workspace, or category.
   - If the user did not name a root, start with the current directory for
     project cleanup and ask before scanning a whole user profile or drive.
   - Completion criterion: the cleanup scope and intent are explicit.

2. Ensure Rebecca is available.
   - Check `rebecca --version`.
   - If missing and the user asked to install, prefer Cargo when Rust is
     available because it works across Windows, macOS, and Linux:

     ```shell
     cargo install rebecca --locked
     ```

   - On Windows, the GitHub release installer is also available. On Linux and
     macOS, keep using Cargo until Unix release archives are published:

     ```powershell
     powershell -ExecutionPolicy Bypass -c "irm https://github.com/Latias94/rebecca/releases/latest/download/rebecca-installer.ps1 | iex"
     ```

   - Completion criterion: `rebecca --version` succeeds, or the user has chosen
     not to install.

3. Inspect before planning deletion.
   - If the user wants an interactive terminal workflow and is at a real TTY,
     prefer the workbench:

     ```powershell
     rebecca tui --root <PATH>
     ```

     Use `rebecca i` as the short alias. The TUI can navigate the disk map,
     switch to Treemap with `4`/`w`, switch to type distribution with `2`/`t`,
     switch to extension distribution with `3`/`x`, cycle views with `Tab`,
     press `Enter` on a type or extension row to filter the map and Treemap,
     press `Backspace` to clear the group filter, refresh the selected directory
     with `r`, refresh the scan root with `R`, restore the previous scan with
     `b`, stage cleanup rules behind advised entries, preview all matching rule
     targets, show live task progress, and execute only after typed confirmation
     through recoverable trash. In mouse-capable terminals, clicks select tabs,
     rows, and Treemap tiles, and the wheel moves selection; mouse input never
     executes cleanup. Press Esc on the working screen to request cooperative
     cancellation. Use `--screen-reader` or `--no-color` when the terminal needs
     plain text cues.
   - For a size map:

     ```powershell
     rebecca inspect map --root <PATH> --top 20 --cleanup-advice
     ```

   - For flat exports, scripts, wrappers, or non-terminal sessions, use
     `--format json`, `--format ndjson`, or `--table csv|tsv`; do not drive the
     TUI as a machine API.
   - On large local Windows NTFS roots, consider `--scan-backend windows-native`;
     use `windows-ntfs-mft-experimental` only for read-only inspection when the
     user wants maximum NTFS provenance and accepts that it is experimental.
   - On Linux and macOS, the portable scanner is the default. Use
     `catalog --platform linux` or `catalog --platform macos` before selecting
     platform cleanup rule IDs.
   - Completion criterion: the user has a ranked, read-only report or a clear
     reason inspection cannot run.

4. Build preview plans with Rebecca.
   - General safe cleanup:

     ```powershell
     rebecca clean --dry-run
     ```

   - Linux cleanup catalog:

     ```shell
     rebecca catalog --kind cleanup-rule --platform linux
     ```

   - macOS cleanup catalog:

     ```shell
     rebecca catalog --kind cleanup-rule --platform macos
     ```

   - Linux user-scoped cache examples:

     ```shell
     rebecca clean --dry-run --rule linux.chrome-cache --allow-warning active-process
     rebecca clean --dry-run --rule linux.pip-cache --allow-moderate
     ```

   - macOS user-scoped cache examples:

     ```shell
     rebecca clean --dry-run --rule macos.chrome-cache --allow-warning active-process
     rebecca clean --dry-run --rule macos.pip-cache --allow-moderate
     rebecca clean --dry-run --rule macos.xcode-cache --allow-moderate --allow-warning active-process --allow-warning permission-sensitive
     ```

   - External Cleaner Manifest v1 rules:

     ```shell
     rebecca rules validate --format json --file <manifest.toml>
     rebecca rules import --format json --file <manifest.toml>
     rebecca rules list --format json
     ```

     Imported rules are disabled by default. Enable them only after the preview
     is reviewed:

     ```shell
     rebecca rules enable <IMPORT_ID>
     ```

   - Linux package-manager archive caches are moderate and permission-sensitive;
     preview them explicitly before considering elevated execution:

     ```shell
     rebecca clean --dry-run --rule linux.apt-cache --allow-moderate --allow-warning permission-sensitive
     ```

   - Project artifacts:

     ```powershell
     rebecca purge --root <WORKSPACE> --dry-run
     ```

   - Installed-app leftovers:

     ```powershell
     rebecca apps clean --dry-run
     ```

   - Rebecca cache health:

     ```powershell
     rebecca cache doctor
     ```

   - Do not invent rule IDs, artifact selectors, warning IDs, or categories.
     Read them from `rebecca catalog`, `inspect map --cleanup-advice`, or the
     preview output.
   - Completion criterion: every proposed cleanup action has a Rebecca dry-run
     result and an estimated reclaim amount or a reason it is unknown.

5. Present a numbered decision list.
   - For each option, include the command, estimated reclaim, safety level,
     warning gates, and whether it moves data to recoverable trash or is
     permanent.
   - If a command needs `--allow-warning <WARNING>`, explain the named warning
     and keep it out of the execution command unless the user confirms it.
   - Completion criterion: the user can choose by number without reading raw
     command output.

6. Execute only after confirmation.
   - Run the confirmed Rebecca command by replacing `--dry-run` with `--yes`.
   - Keep the command otherwise identical to the preview. Never combine
     `--dry-run` and `--yes`.
   - Do not add `sudo` on Linux or macOS unless the confirmed preview is for a
     reviewed target that actually needs elevated execution and the user
     explicitly chooses it.
   - Avoid `--permanent` unless the user explicitly confirms irreversible
     deletion.
   - Do not use `Remove-Item`, `rm -rf`, shell wildcards, or ad hoc deletion as
     the primary cleanup path.
   - Completion criterion: execution finishes and Rebecca reports reclaimed,
     pending, skipped, and failed targets.

7. Verify and report.
   - Re-run the relevant read-only command when useful:

     ```powershell
     rebecca inspect map --root <PATH> --top 20
     rebecca cache doctor
     rebecca history --limit 5
     ```

   - Summarize reclaimed bytes, pending recoverable-trash reclaim, skipped targets,
     warnings, and follow-up commands.
   - Completion criterion: the user has a concise before/after or execution
     summary and any residual risk is named.

## Guardrails

- Never treat `inspect` output as authorization to delete; execution must go
  through `clean`, `purge`, `apps clean`, or `cache purge`.
- Prefer default safe rules before `--allow-moderate`, `--allow-risky`, or
  warning gates.
- On Linux, prefer user-owned XDG cache rules before package-manager cache rules,
  and never use `sudo rebecca clean --yes` as the first command.
- On macOS, prefer user-owned `Library/Caches`, browser, and developer-cache
  rules; do not clean broad `Library/Application Support`, `Containers`, or
  `Group Containers` roots. Homebrew, CocoaPods, and Xcode rules are scoped to
  cache leaves and DerivedData; do not target Homebrew taps, Xcode Archives,
  device support payloads, provisioning profiles, or preferences. If privacy
  probes report a likely Full Disk Access block, grant access to the terminal
  for reviewed user-owned cache paths instead of adding `sudo`.
- Use `--exclude <PATH>` for user-protected paths instead of editing plans by
  hand.
- Treat `--dry-run` and `--yes` as mutually exclusive: preview first, then
  execute by replacing the preview flag with `--yes`.
- Do not clean broad roots such as an entire profile or drive unless the user
  explicitly asked for that scope.
- Keep stdout clean for JSON, NDJSON, CSV, and TSV consumers; progress belongs
  on stderr or in NDJSON progress events.
- Treat `rebecca tui` as a human-only terminal surface. Its map, treemap, type
  distribution, extension distribution, refresh, mouse-selection, and cleanup
  workbench views are not stable machine contracts. For automation, use the
  typed CLI API instead of replaying TUI output.
