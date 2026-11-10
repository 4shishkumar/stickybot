#!/usr/bin/env python3
"""Builds the faces catalog (docs/faces/CATALOG.md) from the face packages in this repo:

    python3 tools/faces/build_catalog.py --out site/faces [--base https://.../faces/]

writes  <out>/catalog.json                      the index the dashboard reads
        <out>/<id>/<id>-<version>.sfb           each face compiled (what the stick stores)
        <out>/<id>/portrait.png, landscape.png  its previews

Every package must be valid (tools/faces/validate.py, with previews); one bad package
fails the build. Ids must be unique and never reuse a retired one (faces/retired.json).
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import facepkg  # noqa: E402

FACES = facepkg.ROOT / "faces"
CATALOG_VERSION = 1
SOURCE = "https://github.com/4shishkumar/stickybot/tree/main/"


def packages(root=FACES):
    """(package dir, channel) for faces/examples/* and faces/community/*."""
    for channel in ("examples", "community"):
        for p in sorted((root / channel).glob("*/manifest.json")):
            yield p.parent, channel


def last_change(path):
    """When the package last changed in git (ISO date), else today."""
    try:
        out = subprocess.run(["git", "log", "-1", "--format=%cs", "--", str(path)], capture_output=True, text=True,
                             cwd=facepkg.ROOT, timeout=10).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        out = ""
    return out or datetime.now(timezone.utc).date().isoformat()  # noqa: UP017 (runs on Python 3.9 too)


def build(out: Path, base: str = "", root: Path = FACES):
    retired = set()
    if (root / "retired.json").is_file():
        retired = set(json.loads((root / "retired.json").read_text()))
    entries, errors, seen = [], [], {}
    for path, channel in packages(root):
        pkg = facepkg.load(path)
        errs = facepkg.check(pkg)
        m = pkg.manifest
        fid = m.get("id")
        if fid != path.name:
            errs.append(f"the folder must be named after the id ({fid!r})")
        if fid in seen:
            errs.append(f"id {fid!r} is also {seen[fid]}")
        if fid in retired:
            errs.append(f"id {fid!r} was retired and can't be reused")
        seen.setdefault(fid, path)
        if errs:
            errors += [f"{path.relative_to(root.parent)}: {e}" for e in errs]
            continue
        blob = facepkg.compile_face(pkg)
        d = out / fid
        d.mkdir(parents=True, exist_ok=True)
        sfb = f"{fid}-{m['version']}.sfb"
        (d / sfb).write_bytes(blob)
        for side in ("portrait", "landscape"):
            shutil.copyfile(path / "previews" / f"{side}.png", d / f"{side}.png")
        entries.append({
            "id": fid, "name": m["name"], "version": m["version"], "about": m["about"],
            "author": m["author"], "license": m["license"], "tags": m.get("tags", []),
            "channel": channel, "format": m["format"], "minFirmware": m["minFirmware"],
            "motion": m.get("motion", "static"),
            "sfb": f"{base}{fid}/{sfb}", "size": len(blob), "sha256": facepkg.sha256(blob),
            "previews": {"portrait": f"{base}{fid}/portrait.png", "landscape": f"{base}{fid}/landscape.png"},
            "source": SOURCE + path.relative_to(root.parent).as_posix(),
            "updated": last_change(path),
        })
    if errors:
        return None, errors
    now = datetime.now(timezone.utc)  # noqa: UP017 (runs on Python 3.9 too)
    catalog = {"catalog": CATALOG_VERSION, "generated": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
               "formats": [facepkg.FORMAT], "faces": entries, "retired": sorted(retired)}
    out.mkdir(parents=True, exist_ok=True)
    (out / "catalog.json").write_text(json.dumps(catalog, indent=1, ensure_ascii=False) + "\n")
    return catalog, []


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--base", default="", help="URL prefix for the files (default: relative to catalog.json)")
    args = ap.parse_args(argv)
    catalog, errors = build(args.out, args.base)
    for e in errors:
        print(e)
    if errors:
        return 1
    print(f"{args.out / 'catalog.json'}: {len(catalog['faces'])} faces")
    return 0


if __name__ == "__main__":
    sys.exit(main())
