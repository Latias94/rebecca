---
name: rebecca-disk-cleaner
description: Use when the user asks Rebecca to install the CLI, inspect disk usage, preview or execute cleanup, remove caches, purge project artifacts, clean app leftovers, or guide a preview-first cleanup.
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

   - If the user asks to install the Rebecca agent skill for future cleanup
     runs, use the packaged CLI installer:

     ```shell
     rebecca skills install
     ```

     The default skill root is `~/.agents/skills`. Use
     `rebecca skills install --agent codex` for Codex-specific installs or
     `--destination <SKILLS_DIR>` for another agent.
   - Completion criterion: `rebecca --version` succeeds, or the user has chosen
     not to install.

3. Inspect before planning deletion.
   - If the user wants an interactive terminal workflow and is at a real TTY,
     prefer the workbench:

     ```powershell
     rebecca tui --root <PATH>
     ```

     Use `rebecca i` as the short alias. Treat the TUI as a human workbench:
     the user can browse map, treemap, type, and extension views, stage cleanup
     rules, preview targets, and execute only after typed confirmation. Mouse
     input selects and navigates; it never authorizes cleanup. Do not script or
     scrape TUI output.
   - For a whole drive, mount point, user profile, or large unknown root, start
     with the guided read-only workflow:

     ```powershell
     rebecca inspect drive <PATH>
     ```

     This enables cleanup advice by default and separates Rebecca preview
     commands from manual-review findings such as Git object stores, SVN
     pristine stores, Unity Library caches, vcpkg build caches, `repo-ref`,
     generated output, and local mirrors. Do not turn review-only findings into
     deletion commands.
   - For a lower-level size map with custom filters, groups, table export, or
     machine wrappers:

     ```powershell
     rebecca inspect map --root <PATH> --top 20 --cleanup-advice
     ```

     Use `--metadata-profile logical-only` for a fast first pass on very large
     trees. Use the default `full-evidence` profile when the user needs
     allocated bytes, hardlink-aware unique bytes, detailed provenance, or
     cleanup-advice confidence.
   - For flat exports, scripts, wrappers, or non-terminal sessions, use
     `--format json`, `--format ndjson`, or `--table csv|tsv`; do not drive the
     TUI as a machine API.
   - On large local Windows NTFS roots, `inspect drive` tries the experimental
     NTFS/MFT inventory by default when the build supports it. If Rebecca
     reports a typed fallback, follow the guidance: feature-disabled means the
     binary lacks NTFS support, permission-denied means the terminal likely
     needs elevation for raw-volume metadata, and unsupported or non-NTFS roots
     should use the portable scanner.
   - On Linux and macOS, the portable scanner is the default. Use
     `rebecca catalog --kind cleanup-rule --platform linux` or
     `rebecca catalog --kind cleanup-rule --platform macos` before selecting
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
     warning gates, and whether it moves data to the system trash or Windows
     Recycle Bin. Say "permanent" only when the command uses `--permanent`.
   - If a command needs `--allow-warning <WARNING>`, explain the named warning
     and keep it out of the execution command unless the user confirms it.
   - Completion criterion: the user can choose by number without reading raw
     command output.

6. Execute only after confirmation.
   - Run the confirmed Rebecca command by replacing `--dry-run` with `--yes`.
   - Keep the command otherwise identical to the preview. Never combine
     `--dry-run` and `--yes`.
   - When the user wants an audit trail, add `--receipt <FILE>` to the confirmed
     command. Receipts record the destination, selected gates, target outcomes,
     pending trash reclaim, and restore hints.
   - By default, `--yes` moves data to the system trash or Windows Recycle Bin.
     If the user wants to bypass trash, add `--permanent` only after they
     explicitly confirm irreversible deletion.
   - If the user wants to free pending space after a normal cleanup, preview
     the trash first, then ask before emptying it. Explain that `--yes` empties
     the platform trash or Windows Recycle Bin scope, not only Rebecca's last
     run:

     ```powershell
     rebecca trash empty
     rebecca trash empty --yes
     ```

     On Windows, use `--drive C` or `--drive E` when the user only wants to
     empty one drive's Recycle Bin.
   - Do not add `sudo` on Linux or macOS unless the confirmed preview is for a
     reviewed target that actually needs elevated execution and the user
     explicitly chooses it.
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

   - Summarize freed bytes, pending trash or Recycle Bin reclaim, skipped
     targets, warnings, and follow-up commands.
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
