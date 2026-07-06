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

PowerShell-only fallback:

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.codex\skills" | Out-Null
Copy-Item -Recurse -Force .\skills\rebecca-disk-cleaner "$env:USERPROFILE\.codex\skills\"
```

Restart Codex after copying the skill.
