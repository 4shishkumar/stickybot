"""Face packages (docs/faces/FORMAT.md): read, check and compile them. Standard library only.

    pkg = load("faces/examples/textclock")   # manifest + images
    problems = check(pkg)                    # [] when it's a valid face
    blob = compile_face(pkg)                 # the .sfb the stick stores

The checks are the schema (docs/faces/face.schema.json, by a small interpreter for the
keywords it uses: no jsonschema dependency) plus what a schema can't say: references,
bindings, fonts, the budgets and the preview pictures.
"""
from __future__ import annotations

import hashlib
import json
import math
import re
import struct
import zlib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_PATH = ROOT / "docs" / "faces" / "face.schema.json"

FORMAT = 1            # the face format this checks and compiles
SFB_VERSION = 1       # the compiled container

# --- Screens and budgets (FORMAT.md, "Budgets") -----------------------------------------
SCREENS = {"portrait": (135, 240), "landscape": (240, 135)}
MAX_SFB = 64 * 1024            # compiled bytes: ~4-5 s over Bluetooth at ~15 KB/s
MAX_MANIFEST = 16 * 1024       # minified manifest bytes
MAX_PACKAGE = 256 * 1024       # the package folder in the repo (PNGs, previews, README)
MAX_IMAGES = 16
MAX_IMAGE_SIDE = 240
MAX_FRAME_SIDE = 128
MAX_FRAMES = 32
MAX_PIXELS = 96 * 1024         # all images' pixels together (decoded 1 byte each in PSRAM)
MAX_ELEMENTS = 48              # per layout, counting group / part children
MAX_INSTANCES = 96             # per layout, counting repeats (draw calls)
MAX_COST = 6                   # screens' worth of pixels touched per frame
MAX_COND_DEPTH = 3
MIN_COLOR_TWEEN_MS = 500       # colour / mix changes: no flashing (GUIDELINES.md)
MIN_STATIC_TEXT_PX = 10

RESERVED_IDS = {
    # built-in faces (m5-stick-s3/src/faces/) and words the protocol / serial commands use
    "classic", "stickyworld", "aquarium", "bigtime", "words", "analog", "dashboard", "space", "lava",
    "stickydark", "aquadark", "all", "none", "demo", "default", "off", "face", "faces", "stickybot",
}

# Status icons the firmware draws over every face (top right) and the default urgent card.
STATUS_ZONE = {"portrait": (135 - 70, 0, 70, 18), "landscape": (240 - 70, 0, 70, 18)}
DEFAULT_CARD = {"portrait": [6, 184, 123, 48], "landscape": [10, 84, 220, 44]}
MIN_CARD = {"portrait": (110, 38), "landscape": (96, 34)}

# --- Fonts: the stick's built-in Inter sizes. 44 / 56 / 96 have digits only ------------
TEXT_CHARS = set(chr(c) for c in range(0x20, 0x7F)) | set("°’‘“”–—…•·×")
FONT_CHARS = {10: TEXT_CHARS, 12: TEXT_CHARS, 14: TEXT_CHARS, 18: TEXT_CHARS, 24: TEXT_CHARS, 32: TEXT_CHARS,
              44: set("0123456789:-–° "), 56: set("0123456789:-–° "), 96: set("0123456789: ")}
DIGIT_FONTS = {44, 56, 96}

# --- Bindings: name -> (kind, chars). kind: str / num / bool. chars: what the value can
# contain, for the digits-only fonts: "d" digits, "t" digits and ':', "n" digits and '-',
# "s" anything.
BINDINGS = {
    # time (the agent's clock; unknown until it has sent it: time.known false, values "")
    "time.known": ("bool", "s"), "time.hm": ("str", "t"), "time.hm24": ("str", "t"),
    "time.h12": ("str", "d"), "time.h24": ("str", "d"), "time.hh12": ("str", "d"), "time.hh24": ("str", "d"),
    "time.mm": ("str", "d"), "time.ss": ("str", "d"), "time.ampm": ("str", "s"),
    "time.hour": ("num", "d"), "time.minute": ("num", "d"), "time.second": ("num", "d"),
    "time.minuteOfDay": ("num", "d"),
    "time.words": ("str", "s"), "time.wordsLead": ("str", "s"), "time.wordsHour": ("str", "s"),
    # date
    "date.dow": ("str", "s"), "date.weekday": ("str", "s"), "date.day": ("str", "d"), "date.dd": ("str", "d"),
    "date.mon": ("str", "s"), "date.month": ("str", "s"), "date.mm": ("str", "d"),
    "date.short": ("str", "s"), "date.long": ("str", "s"),
    # weather
    "wx.valid": ("bool", "s"), "wx.temp": ("num", "n"), "wx.hi": ("num", "n"), "wx.lo": ("num", "n"),
    "wx.icon": ("str", "s"), "wx.cond": ("str", "s"), "wx.day": ("bool", "s"),
    # the stick
    "dev.battery": ("num", "d"), "dev.charging": ("bool", "s"), "dev.bt": ("str", "s"), "dev.name": ("str", "s"),
    "agent.linked": ("bool", "s"),
    # the agent's card (the urgent one is drawn by the firmware anyway)
    "card.kind": ("str", "s"), "card.a": ("str", "s"), "card.b": ("str", "s"), "card.urgent": ("bool", "s"),
    "urgent.on": ("bool", "s"),
    # your work (the agent's world message)
    "work.known": ("bool", "s"), "work.asks": ("num", "d"), "work.reviews": ("num", "d"),
    "work.sessions": ("num", "d"), "work.ci": ("str", "s"), "work.merged": ("num", "d"),
    "work.mergedToday": ("num", "d"),
    # the Claude desktop app (Hardware Buddy)
    "buddy.total": ("num", "d"), "buddy.running": ("num", "d"), "buddy.waiting": ("num", "d"),
    "buddy.tokens": ("num", "d"),
    # StickyBot's progress
    "pet.level": ("num", "d"), "pet.xp": ("num", "d"), "pet.into": ("num", "d"), "pet.need": ("num", "d"),
    "pet.streak": ("num", "d"), "pet.greens": ("num", "d"),
    "pet.hat": ("bool", "s"), "pet.shades": ("bool", "s"), "pet.crown": ("bool", "s"),
    # sky and motion
    "sky.night": ("bool", "s"), "sky.day": ("num", "d"),
    "motion.tiltX": ("num", "n"), "motion.tiltY": ("num", "n"),
}
EVENTS = ["merged", "ci_red", "ci_green", "claude_start", "claude_done", "release", "level_up", "review",
          "shake", "flick", "upside_down", "lift", "minute", "hour", "dusk", "dawn", "urgent"]
for _e in EVENTS:
    BINDINGS[f"react.{_e}"] = ("bool", "s")
TIME_BINDINGS = {k for k in BINDINGS if k.startswith("time.") and k not in ("time.known", "time.ampm")}

TEMPLATE = re.compile(r"\{([a-z]+\.[a-zA-Z0-9_]+)(?::(0[2-4]|upper|lower))?\}")


@dataclass
class Image:
    name: str
    w: int
    h: int
    palette: list          # [(r, g, b, a)]
    pixels: bytes          # one index per pixel, row by row
    frame_w: int = 0

    @property
    def frames(self):
        return max(1, self.w // self.frame_w) if self.frame_w else 1


@dataclass
class Package:
    path: Path
    manifest: dict
    images: dict = field(default_factory=dict)
    problems: list = field(default_factory=list)   # found while loading


# --- PNG (indexed colour only: what the stick's palette images are made from) ------------

PNG_SIG = b"\x89PNG\r\n\x1a\n"


def read_png(data: bytes):
    """An indexed-colour PNG -> (w, h, palette [(r,g,b,a)], pixels one index each).
    Raises ValueError with a helpful message for anything else."""
    if not data.startswith(PNG_SIG):
        raise ValueError("not a PNG")
    pos, ihdr, plte, trns, idat = 8, None, None, b"", []
    while pos + 8 <= len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if kind == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif kind == b"PLTE":
            plte = body
        elif kind == b"tRNS":
            trns = body
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
    if not ihdr:
        raise ValueError("no IHDR")
    w, h, depth, ctype, _comp, _filt, interlace = ihdr
    if ctype != 3:
        raise ValueError("not an indexed-colour PNG (save it with a palette: Aseprite 'Indexed', "
                         "Pillow img.convert('P'), pngquant)")
    if interlace:
        raise ValueError("interlaced PNG (save it without interlacing)")
    if depth not in (1, 2, 4, 8) or plte is None:
        raise ValueError("bad indexed PNG")
    ncol = len(plte) // 3
    pal = [(plte[i * 3], plte[i * 3 + 1], plte[i * 3 + 2], trns[i] if i < len(trns) else 255) for i in range(ncol)]
    raw = zlib.decompress(b"".join(idat))
    stride = (w * depth + 7) // 8
    out, prev = bytearray(), bytearray(stride)
    bpp = 1  # filter unit (bytes per complete pixel, at least 1)
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
        per = 8 // depth
        for x in range(w):
            byte = line[x // per]
            shift = 8 - depth * (x % per + 1)
            out.append((byte >> shift) & ((1 << depth) - 1))
    for v in out:
        if v >= ncol:
            raise ValueError(f"pixel index {v} past the palette ({ncol} colours)")
    return w, h, pal, bytes(out)


def write_png(path, w, h, palette, pixels):
    """An 8-bit indexed PNG (palette [(r,g,b,a)]); used to make the example faces' art."""
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    raw = b"".join(b"\0" + bytes(pixels[y * w:(y + 1) * w]) for y in range(h))
    out = PNG_SIG + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 3, 0, 0, 0))
    out += chunk(b"PLTE", b"".join(bytes(c[:3]) for c in palette))
    if any(c[3] < 255 for c in palette):
        out += chunk(b"tRNS", bytes(c[3] for c in palette))
    out += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    Path(path).write_bytes(out)


def write_rgb_png(path, w, h, rgb: bytes):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    raw = b"".join(b"\0" + rgb[y * w * 3:(y + 1) * w * 3] for y in range(h))
    out = PNG_SIG + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    out += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    Path(path).write_bytes(out)


def png_size(data: bytes):
    if not data.startswith(PNG_SIG) or data[12:16] != b"IHDR":
        return None
    return struct.unpack(">II", data[16:24])


# --- The schema interpreter (the keywords face.schema.json uses) ---------------------------

_schema_cache = None


def schema():
    global _schema_cache
    if _schema_cache is None:
        _schema_cache = json.loads(SCHEMA_PATH.read_text())
    return _schema_cache


def _type_ok(v, t):
    if isinstance(t, list):
        return any(_type_ok(v, x) for x in t)
    return {
        "object": isinstance(v, dict), "array": isinstance(v, list), "string": isinstance(v, str),
        "boolean": isinstance(v, bool), "null": v is None,
        "integer": isinstance(v, int) and not isinstance(v, bool),
        "number": isinstance(v, (int, float)) and not isinstance(v, bool),
    }[t]


def _resolve(s, root):
    while "$ref" in s:
        ref = s["$ref"]
        assert ref.startswith("#/"), ref
        node = root
        for part in ref[2:].split("/"):
            node = node[part]
        s = {**node, **{k: v for k, v in s.items() if k != "$ref"}}
    return s


def schema_errors(value, s=None, root=None, path="$"):
    """Errors of value against the schema: ["$.layouts.portrait: ...", ...]."""
    root = root or schema()
    s = _resolve(s if s is not None else root, root)
    errs = []
    if "const" in s and value != s["const"]:
        return [f"{path}: must be {s['const']!r}"]
    if "enum" in s and value not in s["enum"]:
        return [f"{path}: {value!r} is not one of {s['enum']}"]
    if "type" in s and not _type_ok(value, s["type"]):
        return [f"{path}: should be {s['type']}"]
    if "anyOf" in s:
        branches = [schema_errors(value, b, root, path) for b in s["anyOf"]]
        if all(branches):
            best = min(branches, key=len)
            return best if len(branches) == 1 else [f"{path}: matches none of the allowed forms ({best[0]})"]
    if "oneOf" in s:  # elements: pick the branch by "type" for readable errors
        if isinstance(value, dict) and isinstance(value.get("type"), str):
            pick = [b for b in s["oneOf"] if _resolve(b, root).get("properties", {}).get("type", {}).get("const")
                    == value["type"]]
            if not pick:
                kinds = [_resolve(b, root)["properties"]["type"]["const"] for b in s["oneOf"]]
                return [f"{path}: unknown element type {value['type']!r} (one of {', '.join(kinds)})"]
            errs += schema_errors(value, pick[0], root, path)
        elif sum(not schema_errors(value, b, root, path) for b in s["oneOf"]) != 1:
            errs.append(f"{path}: must match exactly one form")
    if isinstance(value, str):
        if "pattern" in s and not re.search(s["pattern"], value):
            errs.append(f"{path}: {value!r} doesn't match {s['pattern']}")
        if len(value) < s.get("minLength", 0):
            errs.append(f"{path}: too short (at least {s['minLength']})")
        if "maxLength" in s and len(value) > s["maxLength"]:
            errs.append(f"{path}: too long (at most {s['maxLength']})")
    if _type_ok(value, "number"):
        if "minimum" in s and value < s["minimum"]:
            errs.append(f"{path}: {value} is below {s['minimum']}")
        if "maximum" in s and value > s["maximum"]:
            errs.append(f"{path}: {value} is above {s['maximum']}")
    if isinstance(value, list):
        if len(value) < s.get("minItems", 0):
            errs.append(f"{path}: needs at least {s['minItems']} items")
        if "maxItems" in s and len(value) > s["maxItems"]:
            errs.append(f"{path}: at most {s['maxItems']} items (has {len(value)})")
        if "items" in s:
            for i, v in enumerate(value):
                errs += schema_errors(v, s["items"], root, f"{path}[{i}]")
    if isinstance(value, dict):
        props = dict(s.get("properties", {}))
        for sub in s.get("allOf", []):
            sub = _resolve(sub, root)
            for k, v in sub.get("properties", {}).items():
                props.setdefault(k, v)
        for k in s.get("required", []):
            if k not in value:
                errs.append(f"{path}: missing {k!r}")
        if len(value) < s.get("minProperties", 0):
            errs.append(f"{path}: needs at least {s['minProperties']} properties")
        if "maxProperties" in s and len(value) > s["maxProperties"]:
            errs.append(f"{path}: at most {s['maxProperties']} entries")
        closed = s.get("additionalProperties") is False or s.get("unevaluatedProperties") is False
        extra = s.get("additionalProperties") if isinstance(s.get("additionalProperties"), dict) else None
        for k, v in value.items():
            if "propertyNames" in s:
                errs += [e.replace(f"{path}.{k}", f"{path}: name {k!r}") for e in
                         schema_errors(k, s["propertyNames"], root, f"{path}.{k}")]
            if k in props:
                errs += schema_errors(v, props[k], root, f"{path}.{k}")
            elif extra is not None:
                errs += schema_errors(v, extra, root, f"{path}.{k}")
            elif closed:
                errs.append(f"{path}: unknown field {k!r}")
    return errs


# --- Loading --------------------------------------------------------------------------

def load(path) -> Package:
    path = Path(path)
    mf = path / "manifest.json"
    pkg = Package(path, {})
    try:
        pkg.manifest = json.loads(mf.read_text(encoding="utf-8"))
    except FileNotFoundError:
        pkg.problems.append("manifest.json: missing")
        return pkg
    except (ValueError, UnicodeDecodeError) as e:
        pkg.problems.append(f"manifest.json: not valid JSON ({e})")
        return pkg
    if not isinstance(pkg.manifest, dict):
        pkg.problems.append("manifest.json: must be an object")
        pkg.manifest = {}
        return pkg
    images = pkg.manifest.get("images")
    for name, spec in (images.items() if isinstance(images, dict) else []):
        src = spec.get("src") if isinstance(spec, dict) else None
        if not isinstance(src, str) or ".." in src or src.startswith("/"):
            continue  # the schema reports it
        f = path / src
        try:
            w, h, pal, px = read_png(f.read_bytes())
        except FileNotFoundError:
            pkg.problems.append(f"images.{name}: {src} is missing")
            continue
        except (ValueError, zlib.error, struct.error) as e:
            pkg.problems.append(f"images.{name}: {src}: {e}")
            continue
        fw = spec.get("frameW") or 0
        pkg.images[name] = Image(name, w, h, pal, px, fw if isinstance(fw, int) else 0)
    return pkg


# --- Checks ------------------------------------------------------------------------------

def _walk(elements, depth=0):
    """(element, depth) for every element, group children included."""
    for e in elements or []:
        if not isinstance(e, dict):
            continue
        yield e, depth
        if e.get("type") == "group":
            yield from _walk(e.get("children"), depth + 1)


def _template_vars(text):
    return [(m.group(1), m.group(2)) for m in TEMPLATE.finditer(text)]


def _literal(text):
    return TEMPLATE.sub("", text)


def _cond_vars(c, depth=1):
    """(vars, max depth) of a condition."""
    if isinstance(c, str):
        return [c.lstrip("!")], depth
    if isinstance(c, dict):
        if "var" in c:
            return [c["var"]], depth
        for key in ("all", "any"):
            if key in c:
                out, d = [], depth
                for x in c[key]:
                    v, dd = _cond_vars(x, depth + 1)
                    out += v
                    d = max(d, dd)
                return out, d
        if "not" in c:
            return _cond_vars(c["not"], depth + 1)
    return [], depth


def _maybe(c, fixed):
    """A condition with some bindings fixed (fixed: {name: bool}): True / False / None (depends
    on the others). For the cost budget: day and night layers aren't drawn together."""
    if c is None:
        return True
    if isinstance(c, str):
        neg, name = c.startswith("!"), c.lstrip("!")
        return None if name not in fixed else fixed[name] != neg
    if isinstance(c, dict):
        if "all" in c:
            vals = [_maybe(x, fixed) for x in c["all"]]
            return False if False in vals else None if None in vals else True
        if "any" in c:
            vals = [_maybe(x, fixed) for x in c["any"]]
            return True if True in vals else None if None in vals else False
        if "not" in c:
            v = _maybe(c["not"], fixed)
            return None if v is None else not v
    return None


COST_SCENARIOS = [{"sky.night": False, "time.known": True}, {"sky.night": True, "time.known": True}]


def _cost(elements, pkg, layout, parts, fixed):
    total = 0.0
    for e in elements:
        if not isinstance(e, dict) or _maybe(e.get("when"), fixed) is False:
            continue
        rep = e.get("repeat")
        n = rep.get("max", rep["count"] if isinstance(rep.get("count"), int) else 1) if isinstance(rep, dict) else 1
        if e.get("type") == "group":
            kids = parts.get(e["part"], []) if "part" in e else e.get("children", [])
            total += _cost(kids, pkg, layout, parts, fixed) * n
        else:
            total += _area(e, pkg, layout)
    return total


def _bindings_in(e):
    """Every binding an element names: (where, name)."""
    out = []
    for key in ("x", "y", "frame", "value"):
        v = e.get(key)
        if isinstance(v, dict) and "var" in v:
            out.append((key, v["var"]))
    if isinstance(e.get("text"), str):
        out += [("text", v) for v, _ in _template_vars(e["text"])]
    if "when" in e:
        out += [("when", v) for v in _cond_vars(e["when"])[0]]
    r = e.get("repeat")
    if isinstance(r, dict) and isinstance(r.get("count"), str):
        out.append(("repeat", r["count"]))
    return out


def _area(e, pkg, layout):
    """A rough count of the pixels an element touches per frame (for the cost budget)."""
    t = e.get("type")
    W, H = SCREENS[layout]
    if t in ("rect", "bar"):
        a = e.get("w", 0) * e.get("h", 0)
    elif t == "circle":
        a = math.pi * e.get("r", 0) ** 2
    elif t == "arc":
        r, th = e.get("r", 0), e.get("t", 0)
        a = math.pi * (r * r - max(0, r - th) ** 2)
    elif t == "line":
        a = math.hypot(e.get("x2", 0) - (e.get("x") if isinstance(e.get("x"), int) else 0),
                       e.get("y2", 0) - (e.get("y") if isinstance(e.get("y"), int) else 0)) * e.get("width", 1) * 2
    elif t == "hand":
        a = (e.get("length", 0) + e.get("tail", 0)) * max(e.get("w0", 2), e.get("w1", 1)) * 2
        a *= 2 if "shadow" in e else 1
    elif t == "ticks":
        a = e.get("count", 0) * e.get("majorLen", e.get("len", 3)) * e.get("majorWidth", e.get("width", 1)) * 2
    elif t == "text":
        fonts = e.get("fit") or [e.get("font", 12)]
        px = max(f for f in fonts if isinstance(f, int)) if fonts else 12
        chars = len(TEMPLATE.sub("0000", e.get("text", "")))  # a binding ~ 4 characters
        a = min(e.get("w") or W, chars * px * 0.6) * px * e.get("lines", 1)
        a *= 9 if "outline" in e else 2 if "shadow" in e else 1
    elif t == "image":
        img = pkg.images.get(e.get("image"))
        if not img:
            return 0
        fw = img.frame_w or img.w
        s = e.get("scale", 1)
        a = fw * img.h * s * s
        if e.get("tile") == "x":
            a = W * img.h * s * s + a
    elif t == "weather":
        a = e.get("size", 20) ** 2
    elif t == "scatter":
        a = e.get("n", 0) * e.get("size", 1) ** 2
    else:
        a = 0
    rep = e.get("repeat")
    if isinstance(rep, dict):
        a *= rep.get("max", rep["count"] if isinstance(rep.get("count"), int) else 1)
    return a


def check(pkg: Package, *, previews=True, catalog_ids=None):
    """Every problem with the package, as readable lines ([] = valid)."""
    errs = list(pkg.problems)
    m = pkg.manifest
    if not m:
        return errs
    errs += schema_errors(m)
    if errs:
        return errs  # the rest assumes the shape is right

    fid = m["id"]
    if fid in RESERVED_IDS:
        errs.append(f"id: {fid!r} is reserved (a built-in face or a protocol word)")
    if catalog_ids and fid in catalog_ids:
        errs.append(f"id: {fid!r} is taken in the catalog")
    if m["format"] != FORMAT:
        errs.append(f"format: this tool knows format {FORMAT}")

    colors = m.get("colors", {})
    parts = m.get("parts", {})

    def color_ok(where, c):
        if isinstance(c, str) and not c.startswith("#") and c not in colors:
            errs.append(f"{where}: colour {c!r} isn't in colors")

    # images
    total_px = 0
    for name, spec in m.get("images", {}).items():
        img = pkg.images.get(name)
        if not img:
            continue
        total_px += img.w * img.h
        if img.w > MAX_IMAGE_SIDE or img.h > MAX_IMAGE_SIDE:
            errs.append(f"images.{name}: {img.w}x{img.h} is bigger than {MAX_IMAGE_SIDE}x{MAX_IMAGE_SIDE}")
        if len(img.palette) > 256:
            errs.append(f"images.{name}: more than 256 colours")
        alphas = {c[3] for c in img.palette}
        if alphas - {0, 255}:
            errs.append(f"images.{name}: partial transparency (each colour must be fully opaque or fully clear)")
        if "frameW" in spec:
            fw = spec["frameW"]
            if img.w % fw:
                errs.append(f"images.{name}: width {img.w} isn't a whole number of {fw}-px frames")
            if img.w // fw > MAX_FRAMES:
                errs.append(f"images.{name}: {img.w // fw} frames (at most {MAX_FRAMES})")
            if fw > MAX_FRAME_SIDE or img.h > MAX_FRAME_SIDE:
                errs.append(f"images.{name}: frames are at most {MAX_FRAME_SIDE}x{MAX_FRAME_SIDE}")
    if total_px > MAX_PIXELS:
        errs.append(f"images: {total_px} pixels in all (at most {MAX_PIXELS})")
    used_images = set()
    moving = []  # what animates all the time (not just on events): the frame-rate tier
    used_parts = set()

    def check_element(e, where, layout, depth):
        t = e["type"]
        for key in ("fill", "stroke", "color", "track", "outline", "shadow", "majorColor", "firstColor"):
            if key in e:
                color_ok(f"{where}.{key}", e[key])
        for key, name in _bindings_in(e):
            if name not in BINDINGS:
                errs.append(f"{where}.{key}: unknown binding {name!r}")
        if "when" in e:
            _, d = _cond_vars(e["when"])
            if d > MAX_COND_DEPTH:
                errs.append(f"{where}.when: nested {d} deep (at most {MAX_COND_DEPTH})")
            c = e["when"]
            if isinstance(c, dict) and "var" in c:
                kind = BINDINGS.get(c["var"], ("str",))[0]
                op = next(k for k in c if k != "var")
                if op in ("gt", "ge", "lt", "le") and kind != "num":
                    errs.append(f"{where}.when: {c['var']} isn't a number")
        if t == "group":
            if depth >= 1:
                errs.append(f"{where}: groups can't hold groups")
            if ("part" in e) == ("children" in e):
                errs.append(f"{where}: a group has either part or children")
            if "part" in e:
                if e["part"] not in parts:
                    errs.append(f"{where}.part: no part {e['part']!r}")
                used_parts.add(e.get("part"))
        if t == "image":
            img = pkg.images.get(e["image"])
            used_images.add(e["image"])
            if e["image"] not in m.get("images", {}):
                errs.append(f"{where}.image: no image {e['image']!r}")
            elif img:
                frames = [e["frame"]] if isinstance(e.get("frame"), int) else []
                frames += e.get("play", {}).get("seq", [])
                for f in frames:
                    if f >= img.frames:
                        errs.append(f"{where}: frame {f} but {e['image']} has {img.frames}")
                if "play" in e and img.frames < 2:
                    errs.append(f"{where}.play: {e['image']} has one frame (set frameW)")
        if t == "text":
            text = e["text"]
            fonts = e.get("fit") or [e.get("font", 12)]
            if "fit" in e and "w" not in e:
                errs.append(f"{where}: fit needs w (the width to fit in)")
            if "lines" in e and "w" not in e:
                errs.append(f"{where}: lines needs w (the width to wrap at)")
            if e.get("fit") and e["fit"] != sorted(e["fit"], reverse=True):
                errs.append(f"{where}.fit: list the fonts biggest first")
            for f in fonts:
                missing = set(_literal(text)) - FONT_CHARS[f]
                if missing:
                    errs.append(f"{where}: font {f} has no {''.join(sorted(missing))!r}"
                                + (" (44 / 56 / 96 are digits only)" if f in DIGIT_FONTS else ""))
                if f in DIGIT_FONTS:
                    for var, fmt in _template_vars(text):
                        chars = BINDINGS.get(var, ("str", "s"))[1]
                        ok = chars in ("d", "t") or (chars == "n" and f != 96)
                        if not ok or fmt in ("upper", "lower"):
                            errs.append(f"{where}: {var} can show letters, font {f} has digits only")
            if "{" in _literal(text) or "}" in _literal(text):
                errs.append(f"{where}.text: a {{ }} that isn't a binding (bindings look like {{time.hm}})")
        for i, tw in enumerate(e.get("anim", [])):
            tw_where = f"{where}.anim[{i}]"
            if tw["prop"] == "color":
                for k in ("from", "to"):
                    if k in tw:
                        if not isinstance(tw[k], str):
                            errs.append(f"{tw_where}.{k}: a colour")
                        else:
                            color_ok(f"{tw_where}.{k}", tw[k])
            elif any(isinstance(tw.get(k), str) for k in ("from", "to")):
                errs.append(f"{tw_where}: {tw['prop']} takes numbers")
            if tw["prop"] == "mix" and not all(0 <= tw.get(k, 0) <= 1 for k in ("from", "to")):
                errs.append(f"{tw_where}: mix goes from 0 to 1")
            if tw["prop"] in ("mix", "color") and tw["ms"] < MIN_COLOR_TWEEN_MS:
                errs.append(f"{tw_where}: colour changes take at least {MIN_COLOR_TWEEN_MS} ms (no flashing)")
            if tw["prop"] == "frame" and t != "image":
                errs.append(f"{tw_where}: frame is for images")
            if "mixTo" in tw:
                color_ok(f"{tw_where}.mixTo", tw["mixTo"])
            if "times" in tw and "on" not in tw:
                errs.append(f"{tw_where}: times is for anims started by an event (on)")
            if "on" not in tw:
                moving.append(tw_where)
        if "play" in e or (t == "scatter" and any(e.get(k) for k in ("twinkle", "fall", "drift"))):
            moving.append(where)
        if any(isinstance(e.get(k), dict) and str(e[k].get("var", "")).startswith("motion.") for k in ("x", "y")):
            moving.append(where)
        rep = e.get("repeat")
        if isinstance(rep, dict) and isinstance(rep.get("count"), str) and "max" not in rep:
            errs.append(f"{where}.repeat: a count from a binding needs max")

    for pname, elems in parts.items():
        for j, (e, depth) in enumerate(_walk(elems)):
            check_element(e, f"parts.{pname}[{j}]", "portrait", depth + 1)

    for layout, (W, H) in SCREENS.items():
        lay = m["layouts"][layout]
        where = f"layouts.{layout}"
        bg = lay.get("bg")
        if isinstance(bg, str):
            color_ok(f"{where}.bg", bg)
        elif isinstance(bg, dict):
            for c in bg["grad"]:
                color_ok(f"{where}.bg", c)
        card = lay.get("card", DEFAULT_CARD[layout])
        x, y, w, h = card
        if x + w > W or y + h > H:
            errs.append(f"{where}.card: {card} goes off the {W}x{H} screen")
        if w < MIN_CARD[layout][0] or h < MIN_CARD[layout][1]:
            errs.append(f"{where}.card: at least {MIN_CARD[layout][0]}x{MIN_CARD[layout][1]} (urgent text must fit)")
        count = instances = 0
        shows_time = False
        flat = list(_walk(lay["elements"]))
        for j, (e, depth) in enumerate(flat):
            check_element(e, f"{where}.elements[{j}]", layout, depth)
            count += 1
            rep = e.get("repeat")
            n = 1
            if isinstance(rep, dict):
                n = rep.get("max", rep["count"] if isinstance(rep["count"], int) else 1)
            if e["type"] == "group" and "part" in e:
                sub = list(_walk(parts.get(e["part"], [])))
                count += len(sub)
                instances += len(sub) * n
                shows_time |= any(_shows_time(s) for s, _ in sub)
            instances += n
            shows_time |= _shows_time(e)
        cost = max(_cost(lay["elements"], pkg, layout, parts, fixed) for fixed in COST_SCENARIOS)
        if count > MAX_ELEMENTS:
            errs.append(f"{where}: {count} elements (at most {MAX_ELEMENTS}, group and part children included)")
        if instances > MAX_INSTANCES:
            errs.append(f"{where}: {instances} things to draw with the repeats (at most {MAX_INSTANCES})")
        if cost > MAX_COST * W * H:
            errs.append(f"{where}: draws about {int(cost)} pixels a frame (at most {MAX_COST * W * H}: "
                        f"{MAX_COST} screens); use smaller images or fewer layers")
        if not shows_time:
            errs.append(f"{where}: shows no time (a text with a time binding, or a hand)")
    for name in m.get("images", {}):
        if name not in used_images:
            errs.append(f"images.{name}: not used (unused images still cost Bluetooth time)")
    for name in parts:
        if name not in used_parts:
            errs.append(f"parts.{name}: not used")
    motion = m.get("motion", "static")
    if motion == "static" and moving:
        errs.append(f"motion: static, but {moving[0]} moves all the time (use calm or lively)")
    if motion != "static" and not moving:
        errs.append(f"motion: {motion}, but nothing moves all the time: use static (anims on events still play)")

    # compiled size
    if not errs:
        blob = compile_face(pkg)
        man = len(_compiled_manifest(m))
        if man > MAX_MANIFEST:
            errs.append(f"manifest: {man} bytes minified (at most {MAX_MANIFEST})")
        if len(blob) > MAX_SFB:
            errs.append(f"compiled: {len(blob)} bytes (at most {MAX_SFB}): smaller or fewer images")
    # the package folder
    if pkg.path.is_dir():
        total = sum(f.stat().st_size for f in pkg.path.rglob("*") if f.is_file())
        if total > MAX_PACKAGE:
            errs.append(f"package: {total} bytes on disk (at most {MAX_PACKAGE})")
        allowed = {"manifest.json", "README.md", "LICENSE", "LICENSE.md", "LICENSE.txt"}
        for f in pkg.path.rglob("*"):
            if f.is_file() and f.name != ".DS_Store":
                rel = f.relative_to(pkg.path).as_posix()
                if rel not in allowed and not re.fullmatch(r"img/[a-z0-9][a-z0-9_-]{0,31}\.png", rel) \
                        and rel not in ("previews/portrait.png", "previews/landscape.png"):
                    errs.append(f"package: unexpected file {rel}")
        if previews:
            for layout, size in SCREENS.items():
                p = pkg.path / "previews" / f"{layout}.png"
                if not p.is_file():
                    errs.append(f"previews/{layout}.png: missing (make -C m5-stick-s3/tools/face_shots pkg "
                                f"PKG={pkg.path})")
                elif png_size(p.read_bytes()) != size:
                    errs.append(f"previews/{layout}.png: should be {size[0]}x{size[1]}")
    return errs


def _shows_time(e):
    if e.get("type") == "hand":
        return True
    if e.get("type") == "text":
        return any(v in TIME_BINDINGS for v, _ in _template_vars(e.get("text", "")))
    return False


# --- Compiling (the .sfb the stick stores; FORMAT.md, "Compiled form") --------------------

def rgb565(r, g, b):
    return ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3)


def packbits(data: bytes) -> bytes:
    out, i, n = bytearray(), 0, len(data)
    while i < n:
        j = i
        while j + 1 < n and data[j + 1] == data[i] and j - i < 127:
            j += 1
        if j > i:  # a run of j - i + 1 equal bytes
            out += bytes([257 - (j - i + 1), data[i]])
            i = j + 1
            continue
        j = i + 1  # literals until a run of 3 starts
        while j < n and j - i < 128 and not (j + 2 < n and data[j] == data[j + 1] == data[j + 2]):
            j += 1
        out += bytes([j - i - 1]) + data[i:j]
        i = j
    return bytes(out)


def unpackbits(data: bytes, size: int) -> bytes:
    out, i = bytearray(), 0
    while len(out) < size and i < len(data):
        c = data[i]
        i += 1
        if c < 128:
            out += data[i:i + c + 1]
            i += c + 1
        elif c > 128:
            out += bytes([data[i]]) * (257 - c)
            i += 1
    return bytes(out[:size])


def _bits_for(ncolors):
    return 1 if ncolors <= 2 else 2 if ncolors <= 4 else 4 if ncolors <= 16 else 8


def _pack_indices(px: bytes, w: int, h: int, bits: int) -> bytes:
    per = 8 // bits
    out = bytearray()
    for y in range(h):
        row = px[y * w:(y + 1) * w]
        for x in range(0, w, per):
            b = 0
            for k in range(per):
                v = row[x + k] if x + k < w else 0
                b |= v << (8 - bits * (k + 1))
            out.append(b)
    return bytes(out)


def _compiled_manifest(m):
    out = dict(m)
    out.pop("$schema", None)
    out["images"] = {name: {"i": i, **({"frameW": s["frameW"]} if "frameW" in s else {})}
                     for i, (name, s) in enumerate(m.get("images", {}).items())}
    return json.dumps(out, separators=(",", ":"), ensure_ascii=False, sort_keys=False).encode()


def compile_face(pkg: Package) -> bytes:
    """The package as one .sfb (little-endian):

    header  "SFB1" | u16 container version | u16 face format | u32 manifest bytes | u16 images | u16 0
    manifest (minified JSON; images[name] = {"i": index, "frameW"}), zero-padded to 4
    per image: u16 w | u16 h | u16 frameW (0 = one frame) | u8 bits (1/2/4/8) | u8 colours - 1 |
               u8 clear index (255: none) | u8 encoding (0 raw, 1 PackBits) | u16 0 | u32 data bytes |
               u16 RGB565 x colours (zero-padded to 4) | data (indices, rows padded to a byte), zero-padded to 4
    """
    m = pkg.manifest
    man = _compiled_manifest(m)
    names = list(m.get("images", {}))
    out = bytearray(b"SFB1" + struct.pack("<HHIHH", SFB_VERSION, m.get("format", FORMAT), len(man), len(names), 0))
    out += man + b"\0" * (-len(man) % 4)
    for name in names:
        img = pkg.images[name]
        pal = img.palette
        clear = next((i for i, c in enumerate(pal) if c[3] == 0), 255)
        bits = _bits_for(len(pal))
        raw = _pack_indices(img.pixels, img.w, img.h, bits)
        pb = packbits(raw)
        enc, data = (1, pb) if len(pb) < len(raw) else (0, raw)
        out += struct.pack("<HHHBBBBHI", img.w, img.h, img.frame_w, bits, len(pal) - 1, clear, enc, 0,
                           len(data))
        out += b"".join(struct.pack("<H", rgb565(*c[:3])) for c in pal) + b"\0" * (len(pal) % 2 * 2)
        out += data + b"\0" * (-len(data) % 4)
    return bytes(out)


def read_sfb(blob: bytes):
    """The compiled face back: (manifest, [(w, h, frameW, palette565, pixels)]). Raises ValueError."""
    if len(blob) < 16 or blob[:4] != b"SFB1":
        raise ValueError("not a compiled face")
    ver, fmt, mlen, nimg, _ = struct.unpack("<HHIHH", blob[4:16])
    if ver != SFB_VERSION or fmt != FORMAT:
        raise ValueError(f"container {ver} / format {fmt} not supported")
    if len(blob) > MAX_SFB or 16 + mlen > len(blob) or nimg > MAX_IMAGES:
        raise ValueError("bad sizes")
    man = json.loads(blob[16:16 + mlen].decode())
    pos = 16 + mlen + (-mlen % 4)
    images = []
    for _ in range(nimg):
        if pos + 16 > len(blob):
            raise ValueError("truncated image header")
        w, h, fw, bits, nc1, clear, enc, _, dlen = struct.unpack("<HHHBBBBHI", blob[pos:pos + 16])
        pos += 16
        nc = nc1 + 1
        if bits not in (1, 2, 4, 8) or not (0 < w <= MAX_IMAGE_SIDE and 0 < h <= MAX_IMAGE_SIDE):
            raise ValueError("bad image header")
        pal = list(struct.unpack(f"<{nc}H", blob[pos:pos + 2 * nc]))
        pos += 2 * nc + nc % 2 * 2
        data = blob[pos:pos + dlen]
        pos += dlen + (-dlen % 4)
        stride = (w * bits + 7) // 8
        packed = unpackbits(data, stride * h) if enc == 1 else data
        if len(packed) != stride * h:
            raise ValueError("image data too short")
        per = 8 // bits
        px = bytearray()
        for y in range(h):
            for x in range(w):
                b = packed[y * stride + x // per]
                v = (b >> (8 - bits * (x % per + 1))) & ((1 << bits) - 1)
                if v >= nc:
                    raise ValueError("pixel past the palette")
                px.append(v)
        images.append((w, h, fw, pal, clear, bytes(px)))
    return man, images


def sha256(blob: bytes) -> str:
    return hashlib.sha256(blob).hexdigest()
