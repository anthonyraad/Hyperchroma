#!/usr/bin/env python3
"""Copy conversation uploads into samples/ when that folder is mounted."""

import shutil
from pathlib import Path

NAMES = [
    "bug",
    "dark",
    "dragon",
    "electric",
    "fairy",
    "fighting",
    "fire",
    "flying",
    "ghost",
    "grass",
    "ground",
    "ice",
    "normal",
    "poison",
    "psychic",
    "rock",
    "steel",
    "water",
]

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "samples"
SOURCES = [Path("/workspace/uploads"), Path("/uploads")]


def canonical(stem: str) -> str | None:
    lower = stem.lower()
    for name in NAMES:
        if lower == name or lower.startswith(name + "_") or lower.startswith(name + "-"):
            return name
    return None


def main() -> None:
    DEST.mkdir(exist_ok=True)
    copied = 0
    for source in SOURCES:
        if not source.is_dir():
            continue
        for path in sorted(source.iterdir()):
            if not path.is_file():
                continue
            if path.suffix.lower() not in {".mp3", ".wav"}:
                continue
            name = canonical(path.stem)
            if name is None:
                continue
            target = DEST / f"{name}{path.suffix.lower()}"
            if target.exists() and target.stat().st_size == path.stat().st_size:
                continue
            shutil.copy2(path, target)
            copied += 1
            print(f"copied {path.name} -> {target.name}")
    if copied == 0:
        print("no new uploads found")


if __name__ == "__main__":
    main()
