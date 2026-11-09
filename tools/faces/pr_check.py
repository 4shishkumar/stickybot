#!/usr/bin/env python3
"""Checks for a pull request that touches faces/ (.github/workflows/faces.yml):

    python3 tools/faces/pr_check.py changed  --base <sha>            # the changed packages, one per line
    python3 tools/faces/pr_check.py versions --base <sha> <pkg>...   # each changed package raised its version
    python3 tools/faces/pr_check.py previews <committed dir> <rendered dir>
                                       # the re-rendered previews show the same pixels as the committed ones

Pictures are compared by their pixels, not their bytes, so a different zlib can't fail it.
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import facepkg  # noqa: E402

ROOT = facepkg.ROOT


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, cwd=ROOT, check=False)


def changed(base):
    """faces/<channel>/<id> folders with changes since base that still exist."""
    out = git("diff", "--name-only", f"{base}...HEAD", "--", "faces/").stdout.split()
    dirs = []
    for f in out:
        parts = Path(f).parts
        if len(parts) >= 3 and parts[1] in ("examples", "community"):
            d = Path(*parts[:3])
            if (ROOT / d / "manifest.json").is_file() and d not in dirs:
                dirs.append(d)
    return dirs


def version_problems(base, pkgs):
    problems = []
    for pkg in pkgs:
        rel = Path(pkg).resolve().relative_to(ROOT)
        new = json.loads((ROOT / rel / "manifest.json").read_text())
        old = git("show", f"{base}:{rel.as_posix()}/manifest.json")
        if old.returncode != 0:
            continue  # a new face
        try:
            before = json.loads(old.stdout).get("version", "0")
        except ValueError:
            continue
        v_new, v_old = facepkg_version(new.get("version")), facepkg_version(before)
        if v_new <= v_old:
            problems.append(f"{rel}: changed but its version is still {new.get('version')} "
                            f"(was {before}); raise it (GUIDELINES.md, Updates)")
    return problems


def facepkg_version(v):
    try:
        return tuple(int(x) for x in str(v).split("-")[0].split(".")[:3])
    except ValueError:
        return (0,)


def pixels(path):
    """A PNG's (w, h, RGBA bytes); 8-bit grey / RGB / grey+alpha / RGBA, or indexed at any depth."""
    data = Path(path).read_bytes()
    if not data.startswith(facepkg.PNG_SIG):
        raise ValueError(f"{path}: not a PNG")
    pos, ihdr, idat = 8, None, []
    while pos + 8 <= len(data):
        n = int.from_bytes(data[pos:pos + 4], "big")
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if kind == b"IHDR":
            ihdr = body
        elif kind == b"IDAT":
            idat.append(body)
    w, h = int.from_bytes(ihdr[0:4], "big"), int.from_bytes(ihdr[4:8], "big")
    depth, ctype, interlace = ihdr[8], ihdr[9], ihdr[12]
    if ctype == 3:
        w, h, pal, idx = facepkg.read_png(data)
        return w, h, b"".join(bytes(pal[i]) for i in idx)
    channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(ctype)
    if depth != 8 or channels is None or interlace:
        raise ValueError(f"{path}: unsupported PNG (colour type {ctype}, depth {depth})")
    raw = zlib.decompress(b"".join(idat))
    stride, bpp = w * channels, channels
    out, prev = bytearray(), bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        prev = line
        for x in range(w):
            px = line[x * channels:(x + 1) * channels]
            out += {1: lambda p: bytes([p[0]] * 3 + [255]), 2: lambda p: bytes([p[0]] * 3 + [p[1]]),
                    3: lambda p: bytes(p) + b"\xff", 4: bytes}[channels](px)
    return w, h, bytes(out)


def preview_problems(committed, rendered):
    problems = []
    for side in ("portrait", "landscape"):
        a, b = Path(committed) / f"{side}.png", Path(rendered) / f"{side}.png"
        if not a.is_file():
            problems.append(f"{a}: missing (render it: make -C m5-stick-s3/tools/face_shots pkg PKG=... PREVIEWS=1)")
            continue
        pa, pb = pixels(a), pixels(b)
        if pa[:2] != pb[:2]:
            problems.append(f"{a}: {pa[0]}x{pa[1]}, the renderer draws {pb[0]}x{pb[1]}")
        elif pa[2] != pb[2]:
            diff = sum(1 for i in range(0, len(pa[2]), 4) if pa[2][i:i + 4] != pb[2][i:i + 4])
            problems.append(f"{a}: {diff} pixels differ from what the renderer draws; commit the renderer's "
                            "previews (make -C m5-stick-s3/tools/face_shots pkg PKG=... PREVIEWS=1), unedited")
    return problems


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("changed")
    c.add_argument("--base", required=True)
    v = sub.add_parser("versions")
    v.add_argument("--base", required=True)
    v.add_argument("packages", nargs="*", type=Path)
    p = sub.add_parser("previews")
    p.add_argument("committed", type=Path)
    p.add_argument("rendered", type=Path)
    args = ap.parse_args(argv)
    if args.cmd == "changed":
        for d in changed(args.base):
            print(d.as_posix())
        return 0
    problems = version_problems(args.base, args.packages) if args.cmd == "versions" \
        else preview_problems(args.committed, args.rendered)
    for line in problems:
        print(line)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
