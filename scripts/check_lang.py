#!/usr/bin/env python3
"""Compare every translation with en_us.

A translation may lag: keys it does not have fall back to English at runtime, so
missing keys are only warnings unless --strict is passed. Keys English does not
have are always errors, since nothing will ever show them.

    python3 scripts/check_lang.py
    python3 scripts/check_lang.py --strict
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FALLBACK = "en_us"
DIRS = [
    *sorted(ROOT.glob("crates/*/assets/lang")),
    *sorted(ROOT.glob("mods/*/assets/lang")),
    *sorted(ROOT.glob("examples/*/assets/lang")),
]


def keys(path: Path) -> list[str]:
    """The keys of a .lang file; continuation lines are indented and not keys."""
    out: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line[0].isspace():
            continue
        if line.lstrip().startswith("#") or "=" not in line:
            continue
        out.append(line.split("=", 1)[0].strip())
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict", action="store_true", help="fail on untranslated keys too")
    args = parser.parse_args()

    errors = 0
    warnings = 0
    for directory in DIRS:
        english = directory / f"{FALLBACK}.lang"
        if not english.is_file():
            continue
        base = keys(english)
        base_set = set(base)
        for duplicate in sorted({k for k in base if base.count(k) > 1}):
            print(f"error {english.relative_to(ROOT)}: {duplicate} appears more than once")
            errors += 1
        for path in sorted(directory.glob("*.lang")):
            if path.name == english.name:
                continue
            here = keys(path)
            here_set = set(here)
            for duplicate in sorted({k for k in here if here.count(k) > 1}):
                print(f"error {path.relative_to(ROOT)}: {duplicate} appears more than once")
                errors += 1
            for key in sorted(here_set - base_set):
                print(f"error {path.relative_to(ROOT)}: {key} is not in {FALLBACK}")
                errors += 1
            missing = sorted(base_set - here_set)
            if missing:
                level = "error" if args.strict else "warning"
                if args.strict:
                    errors += len(missing)
                else:
                    warnings += len(missing)
                print(f"{level} {path.relative_to(ROOT)}: {len(missing)} untranslated key(s)")
                for key in missing:
                    print(f"    {key}")
    print(f"{errors} error(s), {warnings} warning(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
