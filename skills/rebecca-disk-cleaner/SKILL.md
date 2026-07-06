---
name: rebecca-disk-cleaner
description: Use when the user wants to install Rebecca, clean disk space with Rebecca, inspect large folders, remove safe caches, purge project artifacts, clean app leftovers, or run a preview-first Windows cleanup workflow.
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

   - On Windows, the GitHub release installer is also available:

     ```powershell
     powershell -ExecutionPolicy Bypass -c "irm https://github.com/Latias94/rebecca/releases/latest/download/rebecca-installer.ps1 | iex"
     ```

   - Completion criterion: `rebecca --version` succeeds, or the user has chosen
     not to install.

3. Inspect before planning deletion.
   - For a size map:

     ```powershell
     rebecca inspect map --root <PATH> --top 20 --cleanup-advice
     ```

   - For flat exports or scripts, use `--format json`, `--format ndjson`, or
     `--table csv|tsv`.
   - On large local Windows NTFS roots, consider `--scan-backend windows-native`;
     use `windows-ntfs-mft-experimental` only for read-only inspection when the
     user wants maximum NTFS provenance and accepts that it is experimental.
   - Completion criterion: the user has a ranked, read-only report or a clear
     reason inspection cannot run.

4. Build preview plans with Rebecca.
   - General safe cleanup:

     ```powershell
     rebecca clean --dry-run
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
     warning gates, and whether it moves data to the Recycle Bin or is
     permanent.
   - If a command needs `--allow-warning <WARNING>`, explain the named warning
     and keep it out of the execution command unless the user confirms it.
   - Completion criterion: the user can choose by number without reading raw
     command output.

6. Execute only after confirmation.
   - Run the confirmed Rebecca command with `--yes`.
   - Keep the command otherwise identical to the preview.
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

   - Summarize reclaimed bytes, pending Recycle Bin reclaim, skipped targets,
     warnings, and follow-up commands.
   - Completion criterion: the user has a concise before/after or execution
     summary and any residual risk is named.

## Guardrails

- Never treat `inspect` output as authorization to delete; execution must go
  through `clean`, `purge`, `apps clean`, or `cache purge`.
- Prefer default safe rules before `--allow-moderate`, `--allow-risky`, or
  warning gates.
- Use `--exclude <PATH>` for user-protected paths instead of editing plans by
  hand.
- Do not clean broad roots such as an entire profile or drive unless the user
  explicitly asked for that scope.
- Keep stdout clean for JSON, NDJSON, CSV, and TSV consumers; progress belongs
  on stderr or in NDJSON progress events.
