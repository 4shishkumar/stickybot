#!/usr/bin/env python3
"""Copies the stick firmware's flash parts into the Mac agent, with a manifest,
so the dashboard can flash a stick without PlatformIO.

The parts are written separately at their own offsets — exactly like
`pio run -t upload` — so the settings storage (NVS, 0x9000-0xDFFF: Mac
pairings, combo, name, Wi-Fi, Mac password, sounds...) is never touched.
(A single merged image would cover NVS with padding and wipe it.)

    make release        # = pio run -e sticks3 + this script
"""
import datetime
import hashlib
import json
import re
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
BUILD = HERE / ".pio" / "build" / "sticks3"
BOOT_APP0 = Path.home() / ".platformio/packages/framework-arduinoespressif32/tools/partitions/boot_app0.bin"
OUT = HERE.parent / "mac-agent" / "stick_agent" / "firmware"
PARTS = [  # offset, source, name in the bundle
    ("0x0", BUILD / "bootloader.bin", "bootloader.bin"),
    ("0x8000", BUILD / "partitions.bin", "partitions.bin"),
    ("0xe000", BOOT_APP0, "boot_app0.bin"),
    ("0x10000", BUILD / "firmware.bin", "app.bin"),
]


def main():
    for _, src, _ in PARTS:
        if not src.exists():
            raise SystemExit(f"{src} missing — run `pio run -e sticks3` first")
    if (BUILD / "firmware.bin").read_bytes()[:1] != b"\xe9":
        raise SystemExit("firmware.bin doesn't look like an ESP32 app image")
    config_h = (HERE / "src" / "config.h").read_text()
    version = re.search(r'#define\s+FW_VERSION\s+"([^"]+)"', config_h)
    board = re.search(r'#define\s+BOARD_ID\s+"([^"]+)"', config_h)

    if OUT.exists():  # drop files from older bundles (e.g. a merged image)
        for f in OUT.glob("*.bin"):
            f.unlink()
    OUT.mkdir(parents=True, exist_ok=True)
    parts, digest, size = [], hashlib.sha256(), 0
    for offset, src, name in PARTS:
        data = src.read_bytes()
        shutil.copyfile(src, OUT / name)
        parts.append({"offset": offset, "file": name, "size": len(data),
                      "sha256": hashlib.sha256(data).hexdigest()})
        digest.update(data)
        size += len(data)
    manifest = {
        "version": version.group(1) if version else "unknown",
        "board": board.group(1) if board else "m5-sticks3",
        "built": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
        "sha256": digest.hexdigest(),  # of all parts, in order
        "size": size,
        "parts": parts,
        "flash": {"chip": "esp32s3", "mode": "dio", "freq": "80m", "size": "8MB"},
    }
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"release {manifest['version']}: {size / 1024:.0f} KB in {len(parts)} parts -> {OUT}")


if __name__ == "__main__":
    main()
