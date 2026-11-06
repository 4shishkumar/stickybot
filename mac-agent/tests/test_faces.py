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
    for f in sorted(FACES.glob("*.cpp")):
        m = re.search(r'const char\* id\(\) const override \{ return "([a-z0-9]+)"; \}', f.read_text())
        if m:
            ids.append(m.group(1))
    return ids


def test_every_face_has_its_pictures():
    ids = firmware_faces()
    assert {"classic", "stickyworld", "aquarium", "bigtime", "words", "analog", "dashboard", "space",
            "lava"} <= set(ids)
    for face in ids:
        assert (SHOTS / f"{face}.png").is_file(), f"no portrait picture for {face}"
        assert (SHOTS / f"{face}-wide.png").is_file(), f"no landscape picture for {face}"


def test_every_face_is_registered():
    registry = (FACES / "faces.cpp").read_text()
    for f in FACES.glob("*.cpp"):
        src = f.read_text()
        if "const char* id() const override" not in src:
            continue
        accessor = re.search(r"^Face& (\w+)\(\) \{", src, re.M)
        assert accessor, f"{f.name} defines no Face& accessor"
        assert f"X({accessor.group(1)})" in registry, f"{accessor.group(1)} is not in FACES (faces.cpp)"


def test_face_ids_are_unique_and_short():
    ids = firmware_faces()
    assert len(ids) == len(set(ids))
    assert all(len(i) <= 12 for i in ids)  # saved in NVS, typed in serial commands
