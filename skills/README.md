# Rebecca Skills

This directory contains optional agent skills for using Rebecca safely.

After installing Rebecca, install the disk cleaner skill with the CLI:

```shell
rebecca skills install
```

The default destination is `~/.agents/skills/rebecca-disk-cleaner`. This works
for agents that read shared skills from `~/.agents/skills`.

Codex users can install to the Codex-specific skills directory:

```shell
rebecca skills install --agent codex
```

For another agent, pass its skills root explicitly:

```shell
rebecca skills install --destination <SKILLS_DIR>
```

Use `--dry-run` to preview the target path and `--force` to replace an existing
edited copy. Remove the skill with:

```shell
rebecca skills remove
```

Verify the installed skill:

```bash
rebecca skills path
test -f "$HOME/.agents/skills/rebecca-disk-cleaner/SKILL.md"
```

On Windows PowerShell:

```powershell
rebecca skills path
Test-Path "$env:USERPROFILE\.agents\skills\rebecca-disk-cleaner\SKILL.md"
```

Example prompts after restarting the agent:

- "Use Rebecca to inspect this project and tell me what can be cleaned, but do not delete anything."
- "Scan this drive with Rebecca and show the largest folders with cleanup advice."
- "Preview cleanup of project artifacts in this workspace, then ask before running anything with --yes."
- "Preview Linux browser and developer caches with Rebecca, including any required --allow-moderate or warning gates, but do not use sudo unless I confirm."
- "Preview macOS browser and developer caches with Rebecca, including active-process warning gates, but do not use sudo unless I confirm."
- "Use Rebecca to check app leftovers and cache health, preview first."

Validate repository skills before publishing changes:

```bash
python3 ./skills/validate.py
```

Source checkout fallback when the Rebecca binary is not available yet:

```powershell
python .\skills\install.py --destination "$env:USERPROFILE\.agents\skills"
```

Restart the agent after installing or removing the skill.
