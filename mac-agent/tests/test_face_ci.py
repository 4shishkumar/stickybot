"""The faces CI tools (tools/faces/pr_check.py, sign_catalog.py; .github/workflows/faces.yml): the
preview comparison, the version check, and signing the catalog the way the agent verifies it."""
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "faces"))
import build_catalog  # noqa: E402
import facepkg  # noqa: E402
import pr_check  # noqa: E402
import sign_catalog  # noqa: E402

from stick_agent import faces  # noqa: E402

CATNAP = ROOT / "faces" / "examples" / "catnap"


def test_previews_compare_by_pixels(tmp_path):
    assert pr_check.preview_problems(CATNAP / "previews", CATNAP / "previews") == []
    # the same pixels in another encoding (RGBA, no compression tricks) still match
    w, h, rgba = pr_check.pixels(CATNAP / "previews" / "portrait.png")
    rgb = b"".join(rgba[i:i + 3] for i in range(0, len(rgba), 4))
    facepkg.write_rgb_png(tmp_path / "portrait.png", w, h, rgb)
    shutil.copy(CATNAP / "previews" / "landscape.png", tmp_path / "landscape.png")
    assert pr_check.preview_problems(CATNAP / "previews", tmp_path) == []
    facepkg.write_rgb_png(tmp_path / "portrait.png", w, h, b"\0\0\0" + rgb[3:])
    (problem,) = pr_check.preview_problems(CATNAP / "previews", tmp_path)
    assert "1 pixels differ" in problem


def test_version_order():
    assert pr_check.facepkg_version("1.0.10") > pr_check.facepkg_version("1.0.9")
    assert pr_check.facepkg_version("1.0.0") <= pr_check.facepkg_version("1.0.0")


def test_the_agent_keys_are_found():
    assert sign_catalog.agent_keys() == faces.CATALOG_KEYS


def test_sign_and_verify_like_the_agent(tmp_path, monkeypatch):
    pub = sign_catalog.keygen(tmp_path / "key")
    assert oct((tmp_path / "key").stat().st_mode & 0o777) == "0o600"
    catalog, errors = build_catalog.build(tmp_path / "site")
    assert not errors
    cat = tmp_path / "site" / "catalog.json"
    sig = sign_catalog.sign(cat, (tmp_path / "key").read_text())
    assert sign_catalog.verify(cat, [pub])
    assert faces.verify_signature(cat.read_bytes(), sig.read_text(), keys=[pub])
    assert not sign_catalog.verify(cat, faces.CATALOG_KEYS)  # another key
    cat.write_bytes(cat.read_bytes() + b" ")
    assert not faces.verify_signature(cat.read_bytes(), sig.read_text(), keys=[pub])
