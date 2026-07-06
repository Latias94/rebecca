# Rebecca Codex Skills

This directory contains optional Codex skills for using Rebecca safely.

Install the Rebecca disk cleaner skill with Python.

On Windows:

```powershell
python .\skills\install.py
```

On macOS or Linux:

```bash
python3 ./skills/install.py
```

The installer copies `skills/rebecca-disk-cleaner` to `$CODEX_HOME/skills` when
`CODEX_HOME` is set, otherwise to `~/.codex/skills`.

Verify the installed skill:

```bash
python3 ./skills/install.py --dry-run
test -f "$HOME/.codex/skills/rebecca-disk-cleaner/SKILL.md"
```

On Windows PowerShell:

```powershell
python .\skills\install.py --dry-run
Test-Path "$env:USERPROFILE\.codex\skills\rebecca-disk-cleaner\SKILL.md"
```

Example prompts after restarting Codex:

- "Use Rebecca to inspect this project and tell me what can be cleaned, but do not delete anything."
- "Scan this drive with Rebecca and show the largest folders with cleanup advice."
- "Preview cleanup of project artifacts in this workspace, then ask before running anything with --yes."
- "Preview Linux browser and developer caches with Rebecca, including any required --allow-moderate or warning gates, but do not use sudo unless I confirm."
- "Use Rebecca to check app leftovers and cache health, preview first."

Validate repository skills before publishing changes:

```bash
python3 ./skills/validate.py
```

PowerShell-only fallback:

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.codex\skills" | Out-Null
Copy-Item -Recurse -Force .\skills\rebecca-disk-cleaner "$env:USERPROFILE\.codex\skills\"
```

Restart Codex after copying the skill.
