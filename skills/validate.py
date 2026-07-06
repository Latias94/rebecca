#!/usr/bin/env python3
"""Validate repository-shipped Codex skills."""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path


SKILLS_ROOT = Path(__file__).resolve().parent
INSTALLER = SKILLS_ROOT / "install.py"
IGNORED_DIRS = {"__pycache__"}
DANGEROUS_DELETE_TERMS = ("rm -rf", "Remove-Item", "rmdir /s", "del /f", "del /q")
PROHIBITION_MARKERS = ("do not", "never", "avoid", "forbid", "禁止", "不要")
REQUIRED_REBECCA_COMMANDS = (
    "rebecca inspect map",
    "rebecca clean --dry-run",
    "rebecca purge",
    "rebecca apps clean",
)


class ValidationError(Exception):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate Rebecca Codex skills.")
    parser.add_argument(
        "--skip-install-smoke",
        action="store_true",
        help="Skip the temporary-directory installer smoke test.",
    )
    return parser.parse_args()


def skill_dirs() -> list[Path]:
    return sorted(
        path
        for path in SKILLS_ROOT.iterdir()
        if path.is_dir() and path.name not in IGNORED_DIRS
    )


def parse_frontmatter(path: Path) -> tuple[dict[str, str], str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "---":
        raise ValidationError(f"{path}: missing opening frontmatter marker")

    try:
        close_index = lines[1:].index("---") + 1
    except ValueError as err:
        raise ValidationError(f"{path}: missing closing frontmatter marker") from err

    data: dict[str, str] = {}
    for line_number, line in enumerate(lines[1:close_index], start=2):
        if not line.strip():
            continue
        if ":" not in line:
            raise ValidationError(f"{path}:{line_number}: invalid frontmatter line")
        key, value = line.split(":", 1)
        key = key.strip()
        value = value.strip()
        if not key or not value:
            raise ValidationError(f"{path}:{line_number}: empty frontmatter key or value")
        data[key] = value

    body = "\n".join(lines[close_index + 1 :]).strip()
    if not body:
        raise ValidationError(f"{path}: skill body is empty")
    return data, body


def validate_frontmatter(skill_dir: Path, metadata: dict[str, str]) -> None:
    name = metadata.get("name")
    if name != skill_dir.name:
        raise ValidationError(f"{skill_dir}: frontmatter name must match directory name")

    disabled = metadata.get("disable-model-invocation", "").lower() == "true"
    description = metadata.get("description", "")
    if not disabled:
        if not description:
            raise ValidationError(f"{skill_dir}: model-invoked skills need a description")
        if not description.startswith("Use when"):
            raise ValidationError(f"{skill_dir}: model-invoked description should start with 'Use when'")


def validate_rebecca_disk_cleaner(skill_path: Path, body: str) -> None:
    required_terms = (
        "preview-first",
        "--dry-run",
        "--yes",
        "Completion criterion:",
        "rebecca catalog",
    )
    for term in required_terms:
        if term not in body:
            raise ValidationError(f"{skill_path}: missing required workflow term: {term}")

    for command in REQUIRED_REBECCA_COMMANDS:
        if command not in body:
            raise ValidationError(f"{skill_path}: missing Rebecca command: {command}")

    for line_number, line in enumerate(body.splitlines(), start=1):
        lower = line.lower()
        for term in DANGEROUS_DELETE_TERMS:
            if term.lower() in lower and not any(marker in lower for marker in PROHIBITION_MARKERS):
                raise ValidationError(
                    f"{skill_path}:{line_number}: dangerous delete command outside a prohibition"
                )


def validate_skill(skill_dir: Path) -> None:
    skill_path = skill_dir / "SKILL.md"
    if not skill_path.is_file():
        raise ValidationError(f"{skill_dir}: missing SKILL.md")

    metadata, body = parse_frontmatter(skill_path)
    validate_frontmatter(skill_dir, metadata)

    if metadata["name"] == "rebecca-disk-cleaner":
        validate_rebecca_disk_cleaner(skill_path, body)


def run_installer_smoke() -> None:
    with tempfile.TemporaryDirectory(prefix="rebecca-skills-") as temp:
        destination = Path(temp) / "skills"
        command = [
            sys.executable,
            str(INSTALLER),
            "--destination",
            str(destination),
        ]
        subprocess.run(command + ["--dry-run"], check=True)
        subprocess.run(command, check=True)

        installed_skill = destination / "rebecca-disk-cleaner" / "SKILL.md"
        if not installed_skill.is_file():
            raise ValidationError(f"installer smoke did not create {installed_skill}")


def main() -> int:
    args = parse_args()
    discovered = skill_dirs()
    if not discovered:
        raise ValidationError("no skill directories found")

    for skill_dir in discovered:
        validate_skill(skill_dir)

    if not args.skip_install_smoke:
        run_installer_smoke()

    print(f"Validated {len(discovered)} skill(s).")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValidationError as err:
        print(f"error: {err}", file=sys.stderr)
        raise SystemExit(1)
