"""Face packages (docs/faces/FORMAT.md): the validator and compiler in tools/faces/, run against
the example faces and against broken copies of them."""
import copy
import json
import shutil
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "faces"))
import facepkg  # noqa: E402
import validate  # noqa: E402

EXAMPLES = sorted(p for p in (ROOT / "faces" / "examples").iterdir() if (p / "manifest.json").is_file())


def test_there_are_three_examples():
    assert {p.name for p in EXAMPLES} >= {"textclock", "dialclock", "catnap"}


@pytest.mark.parametrize("path", EXAMPLES, ids=lambda p: p.name)
def test_examples_are_valid(path):
    pkg = facepkg.load(path)
    assert facepkg.check(pkg) == []
    assert pkg.manifest["id"] == path.name  # the folder is named after the id


@pytest.mark.parametrize("path", EXAMPLES, ids=lambda p: p.name)
def test_compiled_face_round_trips(path):
    pkg = facepkg.load(path)
    blob = facepkg.compile_face(pkg)
    assert len(blob) <= facepkg.MAX_SFB and len(blob) % 4 == 0
    man, images = facepkg.read_sfb(blob)
    assert man["id"] == pkg.manifest["id"] and "$schema" not in man
    for (name, img), (w, h, fw, pal, clear, px) in zip(pkg.images.items(), images, strict=True):
        assert man["images"][name]["i"] == list(pkg.images).index(name)
        assert (w, h, fw) == (img.w, img.h, img.frame_w)
        assert px == img.pixels
        assert len(pal) == len(img.palette)
        assert clear == next((i for i, c in enumerate(img.palette) if c[3] == 0), 255)


def test_packbits_round_trip():
    for data in [b"", b"a", b"aaaa", b"abc", bytes(range(200)) * 2, b"\0" * 1000 + b"xyz" + b"\1" * 3]:
        assert facepkg.unpackbits(facepkg.packbits(data), len(data)) == data


def test_png_round_trip(tmp_path):
    pal = [(0, 0, 0, 0), (255, 0, 0, 255), (0, 255, 0, 255)]
    px = bytes([0, 1, 2, 1, 2, 0] * 4)
    facepkg.write_png(tmp_path / "a.png", 6, 4, pal, px)
    w, h, pal2, px2 = facepkg.read_png((tmp_path / "a.png").read_bytes())
    assert (w, h, pal2, px2) == (6, 4, pal, px)


def test_rgb_png_is_refused(tmp_path):
    facepkg.write_rgb_png(tmp_path / "a.png", 2, 2, bytes(12))
    with pytest.raises(ValueError, match="indexed"):
        facepkg.read_png((tmp_path / "a.png").read_bytes())


# --- Broken copies of an example: each mistake is reported -------------------------------

@pytest.fixture
def catnap(tmp_path):
    dst = tmp_path / "catnap"
    shutil.copytree(ROOT / "faces" / "examples" / "catnap", dst)
    return dst


def problems(path, change):
    m = json.loads((path / "manifest.json").read_text())
    change(m)
    (path / "manifest.json").write_text(json.dumps(m))
    return facepkg.check(facepkg.load(path))


def _portrait(m):
    return m["layouts"]["portrait"]["elements"]


CASES = [
    ("id", lambda m: m.update(id="Cat-Nap"), "doesn't match"),
    ("reserved id", lambda m: m.update(id="classic"), "reserved"),
    ("long id", lambda m: m.update(id="abcdefghijklm"), "doesn't match"),
    ("license", lambda m: m.update(license="GPL-3.0"), "is not one of"),
    ("no landscape", lambda m: m["layouts"].pop("landscape"), "missing 'landscape'"),
    ("unknown field", lambda m: m.update(script="alert(1)"), "unknown field 'script'"),
    ("unknown element", lambda m: _portrait(m).append({"type": "video"}), "unknown element type"),
    ("unknown binding", lambda m: _portrait(m).append({"type": "text", "text": "{env.HOME}", "color": "ink"}),
     "unknown binding 'env.HOME'"),
    ("letters in a digits font", lambda m: _portrait(m).append(
        {"type": "text", "text": "{time.ampm}", "font": 56, "color": "ink"}), "digits only"),
    ("literal letters in a digits font", lambda m: _portrait(m).append(
        {"type": "text", "text": "h{time.mm}", "font": 96, "color": "ink"}), "font 96 has no"),
    ("unknown colour", lambda m: _portrait(m).append({"type": "rect", "w": 4, "h": 4, "fill": "teal"}),
     "isn't in colors"),
    ("unknown image", lambda m: _portrait(m).append({"type": "image", "image": "dog"}), "no image 'dog'"),
    ("frame past the sheet", lambda m: _portrait(m).append({"type": "image", "image": "cat", "frame": 9}),
     "frame 9"),
    ("flashing", lambda m: _portrait(m).append({"type": "rect", "w": 135, "h": 240, "fill": "red", "anim": [
        {"prop": "mix", "from": 0, "to": 1, "ms": 100, "loop": "pingpong"}]}), "no flashing"),
    ("too many elements", lambda m: _portrait(m).extend(
        [{"type": "rect", "w": 1, "h": 1, "fill": "ink"}] * 40), "at most 48"),
    ("deep condition", lambda m: _portrait(m).append({"type": "rect", "w": 1, "h": 1, "fill": "ink", "when": {
        "not": {"not": {"not": {"not": "sky.night"}}}}}), "nested"),
    ("off-screen card", lambda m: m["layouts"]["portrait"].update(card=[100, 200, 120, 48]), "goes off"),
    ("no time", lambda m: m["layouts"]["landscape"].update(elements=[
        e for e in m["layouts"]["landscape"]["elements"] if "time." not in json.dumps(e.get("text", ""))]),
     "shows no time"),
    ("static but moving", lambda m: m.update(motion="static"), "moves all the time"),
    ("heavy", lambda m: _portrait(m).extend(
        [{"type": "rect", "w": 135, "h": 240, "fill": "ink"}] * 6), "pixels a frame"),
    ("unused part", lambda m: m["parts"].update(spare=[{"type": "rect", "w": 1, "h": 1, "fill": "ink"}]),
     "parts.spare: not used"),
    ("number compared", lambda m: _portrait(m).append(
        {"type": "rect", "w": 1, "h": 1, "fill": "ink", "when": {"var": "work.ci", "gt": 1}}), "isn't a number"),
]


@pytest.mark.parametrize("name,change,expect", CASES, ids=[c[0] for c in CASES])
def test_mistakes_are_reported(catnap, name, change, expect):
    errs = problems(catnap, change)
    assert any(expect in e for e in errs), errs


def test_unused_image_and_missing_file(catnap):
    (catnap / "img" / "fish.png").unlink()
    assert any("fish.png is missing" in e for e in facepkg.check(facepkg.load(catnap)))


def test_big_image_budget(catnap):
    for name in ("hills", "clouds"):
        facepkg.write_png(catnap / "img" / f"{name}.png", 240, 240, [(0, 0, 0, 255), (1, 1, 1, 255)],
                          bytes(240 * 240))
    errs = facepkg.check(facepkg.load(catnap))
    assert any("pixels in all" in e for e in errs), errs


def test_stray_files_and_missing_previews(catnap):
    (catnap / "run.sh").write_text("#!/bin/sh\n")
    (catnap / "previews" / "landscape.png").unlink()
    errs = facepkg.check(facepkg.load(catnap))
    assert any("unexpected file run.sh" in e for e in errs)
    assert any("previews/landscape.png: missing" in e for e in errs)
    assert not any("previews" in e for e in facepkg.check(facepkg.load(catnap), previews=False))


def test_not_json(tmp_path):
    (tmp_path / "manifest.json").write_text("{nope")
    assert "not valid JSON" in facepkg.check(facepkg.load(tmp_path))[0]


def test_read_sfb_refuses_garbage():
    for blob in [b"", b"SFB1" + bytes(12), b"PNG" + bytes(40)]:
        with pytest.raises(ValueError):
            facepkg.read_sfb(blob)


def test_schema_interpreter_matches_the_schema_file():
    s = facepkg.schema()
    assert s["properties"]["format"]["const"] == facepkg.FORMAT
    events = s["$defs"]["event"]["enum"]
    assert events == facepkg.EVENTS
    good = copy.deepcopy(facepkg.load(EXAMPLES[0]).manifest)
    assert facepkg.schema_errors(good) == []


def test_cli(tmp_path, capsys):
    assert validate.main([str(p) for p in EXAMPLES] + ["--compile", str(tmp_path)]) == 0
    assert {p.name for p in tmp_path.iterdir()} == {f"{p.name}.sfb" for p in EXAMPLES}
    assert validate.main([str(tmp_path)]) == 1  # not a package
    assert "manifest.json: missing" in capsys.readouterr().out


def test_catalog_build(tmp_path):
    import build_catalog

    catalog, errors = build_catalog.build(tmp_path / "site", base="https://example.test/faces/")
    assert errors == []
    ids = [f["id"] for f in catalog["faces"]]
    assert set(ids) >= {"textclock", "dialclock", "catnap"}
    face = next(f for f in catalog["faces"] if f["id"] == "catnap")
    blob = (tmp_path / "site" / "catnap" / f"catnap-{face['version']}.sfb").read_bytes()
    assert face["sha256"] == facepkg.sha256(blob) and face["size"] == len(blob)
    assert face["sfb"].startswith("https://example.test/faces/catnap/")
    assert (tmp_path / "site" / "catnap" / "portrait.png").is_file()
    assert json.loads((tmp_path / "site" / "catalog.json").read_text())["faces"] == catalog["faces"]


def test_catalog_refuses_a_bad_package(tmp_path):
    import build_catalog

    root = tmp_path / "faces"
    shutil.copytree(ROOT / "faces" / "examples" / "textclock", root / "community" / "other")
    catalog, errors = build_catalog.build(tmp_path / "site", root=root)
    assert catalog is None and any("named after the id" in e for e in errors)
