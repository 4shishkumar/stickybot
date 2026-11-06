"""The dashboard's Face picker: the stick sends its list of faces (@state "faces"), and the
dashboard shows each one's pictures from static/faces/ (made by m5-stick-s3/tools/face_shots,
`make -C m5-stick-s3/tools/face_shots previews`). Every face in the firmware has both."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FACES = ROOT / "m5-stick-s3" / "src" / "faces"
SHOTS = ROOT / "mac-agent" / "stick_agent" / "static" / "faces"


def firmware_faces():
    ids = []
    for f in sorted(FACES.glob("*.cpp")):  # a file can hold a face and its variants (the dark ones)
        ids += re.findall(r'const char\* id\(\) const override \{ return "([a-z0-9]+)"; \}', f.read_text())
    return ids


def test_every_face_has_its_pictures():
    ids = firmware_faces()
    assert {"classic", "stickyworld", "aquarium", "bigtime", "words", "analog", "dashboard", "space",
            "lava", "stickydark", "aquadark"} <= set(ids)
    for face in ids:
        assert (SHOTS / f"{face}.png").is_file(), f"no portrait picture for {face}"
        assert (SHOTS / f"{face}-wide.png").is_file(), f"no landscape picture for {face}"


def test_every_face_is_registered():
    registry = (FACES / "faces.cpp").read_text()
    for f in FACES.glob("*.cpp"):
        src = f.read_text()
        if "const char* id() const override" not in src:
            continue
        accessors = re.findall(r"^Face& (\w+)\(\) \{", src, re.M)
        assert accessors, f"{f.name} defines no Face& accessor"
        assert len(accessors) == src.count("const char* id() const override"), f"{f.name}: a face with no accessor"
        for a in accessors:
            assert f"X({a})" in registry, f"{a} is not in FACES (faces.cpp)"


def test_face_ids_are_unique_and_short():
    ids = firmware_faces()
    assert len(ids) == len(set(ids))
    assert all(len(i) <= 12 for i in ids)  # saved in NVS, typed in serial commands
