"""Release packaging cannot label an S3 image as a Plus2 build."""
import importlib.util
import json
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "m5-stick-s3/tools/make_release.py"
spec = importlib.util.spec_from_file_location("make_release_boards", SCRIPT)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def sources(tmp_path, chip):
    build = tmp_path / "build"
    build.mkdir()
    image = bytearray(64)
    image[0] = 0xe9
    image[12:14] = chip.to_bytes(2, "little")
    (build / "firmware.bin").write_bytes(image)
    (build / "bootloader.bin").write_bytes(image)
    (build / "partitions.bin").write_bytes(b"partitions")
    boot = tmp_path / "boot.bin"
    boot.write_bytes(b"ota")
    config = tmp_path / "config.h"
    config.write_text('#define FW_VERSION "1.0.0"\n')
    return {"build": build, "boot": boot, "config_h": config}


@pytest.mark.parametrize(("board", "chip", "offset", "silicon"), [
    ("m5-sticks3", 9, "0x0", "esp32s3"),
    ("m5-stickc-plus2", 0, "0x1000", "esp32"),
])
def test_packages_exact_board_layout(tmp_path, monkeypatch, board, chip, offset, silicon):
    monkeypatch.setattr(release, "git_commit", lambda: "test")
    out = tmp_path / "bundle"
    manifest = release.make(out, board=board, **sources(tmp_path, chip))
    assert manifest["board"] == board
    assert manifest["flash"]["chip"] == silicon
    assert [p["offset"] for p in manifest["parts"]] == [offset, "0x8000", "0xe000", "0x10000"]
    assert json.loads((out / "manifest.json").read_text()) == manifest


def test_wrong_silicon_never_overwrites_bundle(tmp_path):
    out = tmp_path / "bundle"
    out.mkdir()
    (out / "app.bin").write_bytes(b"keep")
    with pytest.raises(SystemExit, match="not an image for"):
        release.make(out, board="m5-stickc-plus2", **sources(tmp_path, 9))
    assert (out / "app.bin").read_bytes() == b"keep"


def test_wrong_bootloader_never_gets_packaged(tmp_path):
    inputs = sources(tmp_path, 0)
    boot = bytearray((inputs["build"] / "bootloader.bin").read_bytes())
    boot[12] = 9
    (inputs["build"] / "bootloader.bin").write_bytes(boot)
    with pytest.raises(SystemExit, match="bootloader.bin is not an image"):
        release.make(tmp_path / "bundle", board="m5-stickc-plus2", **inputs)
    assert not (tmp_path / "bundle").exists()
