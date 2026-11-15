#!/usr/bin/env python3
"""Collects the stick firmware's flash parts with a manifest, so the Mac agent can
flash a stick without PlatformIO.

The parts are written separately at their own offsets — exactly like
`pio run -t upload` — so the settings storage (NVS, 0x9000-0xDFFF: Mac
pairings, combo, name, Wi-Fi, Mac password, sounds...) is never touched.
(A single merged image would cover NVS with padding and wipe it.)

    make release            # = pio run -e sticks3 + this script: a local build for the agent
                            #   in ~/.stick-agent/firmware/local/ (or $STICK_AGENT_HOME/...)
    make_release.py --out dist --release v1.8.0 --agent-version 1.8.0
                            # the release workflow: the GitHub release's assets

manifest.json: the firmware version (FW_VERSION) and board, the release tag and agent
version it ships with (if given), the git commit, every part's offset / size / sha256,
the sha256 of all parts in order, and the flash parameters.
"""
import argparse
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
BUILD = HERE / ".pio" / "build" / "sticks3"
PIO_PACKAGES = Path.home() / ".platformio" / "packages"
BOOT_APP0 = PIO_PACKAGES / "framework-arduinoespressif32/tools/partitions/boot_app0.bin"
LOCAL = Path(os.environ.get("STICK_AGENT_HOME", Path.home() / ".stick-agent")) / "firmware" / "local"
PARTS = [  # offset, source (in the build folder, or boot_app0), name in the bundle
    ("0x0", "bootloader.bin", "bootloader.bin"),
    ("0x8000", "partitions.bin", "partitions.bin"),
    ("0xe000", None, "boot_app0.bin"),
    ("0x10000", "firmware.bin", "app.bin"),
]
BOARDS = {
    "m5-sticks3": {"env": "sticks3", "chip": "esp32s3", "chip_id": 9, "boot_offset": "0x0"},
    "m5-stickc-plus2": {"env": "stickc_plus2", "chip": "esp32", "chip_id": 0, "boot_offset": "0x1000"},
}
FLASH = {"chip": "esp32s3", "mode": "dio", "freq": "80m", "size": "8MB"}


def boot_app0():
    if BOOT_APP0.exists():
        return BOOT_APP0
    found = sorted(PIO_PACKAGES.glob("*/tools/partitions/boot_app0.bin"))  # another framework package name
    return found[0] if found else BOOT_APP0


def git_commit():
    try:
        r = subprocess.run(["git", "-C", str(HERE), "rev-parse", "HEAD"], capture_output=True, text=True, timeout=10)
        return r.stdout.strip() or None
    except (OSError, subprocess.TimeoutExpired):
        return None


def make(out, build=None, boot=None, release=None, agent_version=None, config_h=HERE / "src" / "config.h",
         board="m5-sticks3"):
    """Writes the parts + manifest.json into `out`; returns the manifest."""
    target = BOARDS.get(board)
    if target is None:
        raise SystemExit(f"unsupported board: {board}")
    build = Path(build) if build else HERE / ".pio" / "build" / target["env"]
    boot = boot or boot_app0()
    parts = [(target["boot_offset"], *PARTS[0][1:]), *PARTS[1:]]
    sources = [(offset, (build / src) if src else boot, name) for offset, src, name in parts]
    for _, src, _ in sources:
        if not src.exists():
            raise SystemExit(f"{src} missing — run `pio run -e {target['env']}` first")
    for image_name in ("firmware.bin", "bootloader.bin"):
        image = (build / image_name).read_bytes()
        if len(image) < 24 or image[0] != 0xe9 or int.from_bytes(image[12:14], "little") != target["chip_id"]:
            raise SystemExit(f"{image_name} is not an image for {board} ({target['chip']}); refusing to label it")
    text = config_h.read_text()
    version = re.search(r'#define\s+FW_VERSION\s+"([^"]+)"', text)

    out = Path(out)
    if out.exists():  # drop files from older bundles (e.g. a merged image)
        for f in out.glob("*.bin"):
            f.unlink()
    out.mkdir(parents=True, exist_ok=True)
    parts, digest, size = [], hashlib.sha256(), 0
    for offset, src, name in sources:
        data = src.read_bytes()
        shutil.copyfile(src, out / name)
        parts.append({"offset": offset, "file": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        digest.update(data)
        size += len(data)
    manifest = {
        "version": version.group(1) if version else "unknown",  # the firmware's (what the stick reports)
        "board": board,
        "release": release,              # e.g. "v1.8.0"; None for a local build
        "agent_version": agent_version,  # the agent this firmware was released with
        "commit": git_commit(),
        "built": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
        "sha256": digest.hexdigest(),  # of all parts, in order
        "size": size,
        "parts": parts,
        "flash": {**FLASH, "chip": target["chip"]},
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, default=LOCAL, help=f"where to write the bundle (default {LOCAL})")
    ap.add_argument("--board", choices=BOARDS, default="m5-sticks3", help="board whose build to package")
    ap.add_argument("--release", help="the release tag, e.g. v1.8.0")
    ap.add_argument("--agent-version", help="the agent version released with it")
    args = ap.parse_args(argv)
    m = make(args.out, release=args.release, agent_version=args.agent_version, board=args.board)
    print(f"firmware {m['version']}{' for ' + m['release'] if m['release'] else ''}: "
          f"{m['size'] / 1024:.0f} KB in {len(m['parts'])} parts -> {args.out}")


if __name__ == "__main__":
    main()
