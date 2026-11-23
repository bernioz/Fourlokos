#!/usr/bin/env python3
"""Copies the skills from .agents/skills (the source; Codex reads them there) to .claude/skills
(where Claude Code reads them). Run it after editing a skill; `--check` only reports drift (exit 1).

    python tools/sync_skills.py [--check]
"""
import filecmp
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC, DST = ROOT / ".agents" / "skills", ROOT / ".claude" / "skills"
IGNORE = shutil.ignore_patterns("__pycache__", "*.pyc")


def differs(a: Path, b: Path) -> bool:
    if not b.exists():
        return True
    cmp = filecmp.dircmp(a, b, ignore=["__pycache__"])
    if cmp.left_only or cmp.right_only or cmp.diff_files or cmp.funny_files:
        return True
    return any(differs(a / d, b / d) for d in cmp.common_dirs)


def main() -> int:
    check = "--check" in sys.argv[1:]
    stale = [s.name for s in sorted(SRC.iterdir()) if s.is_dir() and differs(s, DST / s.name)]
    if check:
        print("out of sync: " + ", ".join(stale) if stale else "skills in sync")
        return 1 if stale else 0
    for name in stale:
        shutil.rmtree(DST / name, ignore_errors=True)
        shutil.copytree(SRC / name, DST / name, ignore=IGNORE)
        print(f"synced {name}")
    if not stale:
        print("already in sync")
    return 0


if __name__ == "__main__":
    sys.exit(main())
