#!/usr/bin/env python3
"""Install repository-shipped Rebecca skills into a local agent skills directory."""

from __future__ import annotations

import argparse
import os
import shutil
from pathlib import Path


DEFAULT_SKILL = "rebecca-disk-cleaner"


def default_skills_dir() -> Path:
    return Path.home() / ".agents" / "skills"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Install Rebecca agent skills.")
    parser.add_argument(
        "skill",
        nargs="?",
        default=DEFAULT_SKILL,
        help=f"Skill directory name to install. Defaults to {DEFAULT_SKILL}.",
    )
    parser.add_argument(
        "--destination",
        type=Path,
        default=default_skills_dir(),
        help="Destination skills directory. Defaults to ~/.agents/skills.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the copy operation without writing files.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    skills_root = Path(__file__).resolve().parent
    source = skills_root / args.skill
    destination_root = args.destination.expanduser()
    destination = destination_root / args.skill

    if not source.is_dir():
        raise SystemExit(f"Skill not found: {source}")
    if not (source / "SKILL.md").is_file():
        raise SystemExit(f"Skill is missing SKILL.md: {source}")

    print(f"Installing {source} -> {destination}")
    if args.dry_run:
        print("Dry run only; no files copied.")
        return 0

    destination_root.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination, dirs_exist_ok=True)
    print("Done. Restart the agent to load the skill.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
