#!/usr/bin/env python3
"""Check face packages (docs/faces/FORMAT.md) against the schema and the budgets.

    python3 tools/faces/validate.py faces/examples/textclock [more packages...]
    python3 tools/faces/validate.py faces/examples/*  --compile build/   # also write <id>.sfb
    python3 tools/faces/validate.py faces/examples/catnap --no-previews  # while drawing it

Exit status 0 when every package is valid. Standard library only.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import facepkg  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("packages", nargs="+", type=Path)
    ap.add_argument("--compile", metavar="DIR", type=Path, help="write each valid face's <id>.sfb here")
    ap.add_argument("--no-previews", action="store_true", help="don't require previews/*.png yet")
    ap.add_argument("-q", "--quiet", action="store_true")
    args = ap.parse_args(argv)

    bad = 0
    ids = {}
    for path in args.packages:
        pkg = facepkg.load(path)
        errs = facepkg.check(pkg, previews=not args.no_previews)
        fid = pkg.manifest.get("id") if isinstance(pkg.manifest, dict) else None
        if fid in ids:
            errs.append(f"id: {fid!r} is also used by {ids[fid]}")
        ids.setdefault(fid, path)
        if errs:
            bad += 1
            print(f"{path}: {len(errs)} problem{'s' if len(errs) > 1 else ''}")
            for e in errs:
                print(f"  - {e}")
            continue
        blob = facepkg.compile_face(pkg)
        if not args.quiet:
            print(f"{path}: ok  ({fid} {pkg.manifest['version']}, {len(blob)} bytes compiled, "
                  f"~{len(blob) / 15000:.1f} s over Bluetooth)")
        if args.compile:
            args.compile.mkdir(parents=True, exist_ok=True)
            (args.compile / f"{fid}.sfb").write_bytes(blob)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
