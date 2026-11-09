"""Face packages on the stick (firmware 2.4.0+): the catalog, installing over Bluetooth,
listing, removing and choosing them. See docs/faces/FORMAT.md (§17 storage, §18 installing,
§19 `@faces`) and docs/faces/CATALOG.md.

- **Catalog**: catalog.json from GitHub Pages (config `faces_catalog_url`), fetched with
  If-None-Match / If-Modified-Since at most every 6 hours (or when asked) and kept in
  ~/.stick-agent/faces/, so browsing works offline. The default catalog must carry a valid
  ed25519 signature (catalog.json.sig, made in CI) by one of CATALOG_KEYS; another URL (a fork,
  a test server) may be unsigned.
- **Verify**: every .sfb is checked before it goes anywhere: the catalog's size and sha256, the
  container parsed (check_sfb: header, budgets, every image), its id / version equal to the
  entry's, a format the stick reads and a minFirmware the stick has.
- **Install**: `{"faces":{"put":{id, version, size, sha256}}}`, then the bytes over the OTA
  characteristic with the same flow control as firmware (ota.Transfer), `end`, and "done".
- **List**: `@faces` (and `@faces page=N` while `more`); **remove**: `{"faces":{"rm":id}}`;
  **use**: `@face id=<id>`.
"""
import asyncio
import base64
import binascii
import hashlib
import json
import logging
import re
import struct
import time
import urllib.error
import urllib.request
from email.utils import formatdate
from pathlib import Path
from urllib.parse import urljoin, urlparse

from . import config, flasher, ota

log = logging.getLogger("faces")

DEFAULT_CATALOG_URL = "https://ashish-kumar-noon.github.io/stickybot/faces/catalog.json"
MIN_FIRMWARE = "2.4.0"   # the first firmware with the face engine (pkgFaces in @state)
CATALOG_VERSION = 1      # catalog.json "catalog" this agent understands
FORMATS = (1,)           # face formats this agent can check
REFRESH_S = 6 * 3600     # fetch the catalog at most this often unless asked
HTTP_TIMEOUT_S = 15
MAX_CATALOG = 2 * 1024 * 1024
MAX_PREVIEW = 256 * 1024
REPLY_S = 6.0            # rm / @faces answers
MAX_PAGES = 4            # @faces pages (6 faces each; the stick keeps 12)

# The compiled form's limits (FORMAT.md §14, §15; tools/faces/facepkg.py)
MAX_SFB = 64 * 1024
MAX_IMAGES = 16
MAX_IMAGE_SIDE = 240
MAX_PIXELS = 96 * 1024
MAX_MANIFEST = 16 * 1024
SFB_VERSION = 1
ID_RE = re.compile(r"^[a-z][a-z0-9]{2,11}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")

# ed25519 public keys (base64, raw 32 bytes) whose signature on catalog.json is accepted. A list,
# so a key can be rotated: add the new one, ship an agent, switch CI's secret, then drop the old.
# The private half lives only in the repo's Actions secret FACES_SIGNING_KEY.
CATALOG_KEYS = [
    "h5TygDfRbTwqYniIFDTQ0gApT0g7Znx4e1EDzgOfSYA=",
]

REASONS = {  # the stick's refusal / error codes (FORMAT.md §18) -> words for the dashboard
    "busy": "the stick is busy ({why}); try again when it's idle",
    "ota": "a firmware update is running on the stick; try again when it's done",
    "size": "this face is too big for the stick (64 KB at most)",
    "full": "your stick has no room for another face (12 at most); remove one first",
    "format": "your stick can't read this kind of face; update the stick's firmware",
    "firmware": "this face needs newer firmware on the stick; update the stick first",
    "sha": "the face arrived damaged (its checksum didn't match); nothing changed on the stick, try again",
    "invalid": "the stick couldn't load this face ({why})",
    "timeout": "the stick gave up waiting for data; try again",
    "not_started": "the stick had no install running; try again",
    "incomplete": "the stick is missing part of the face; try again",
    "write": "writing to the stick's storage failed",
    "read": "reading the face back on the stick failed",
    "open": "the stick couldn't start writing the face",
    "notfound": "that face isn't on the stick",
    "id": "this face's id belongs to a built-in face; rename it",
}


class FaceError(Exception):
    pass


class FaceCancelled(FaceError):
    pass


def reason(msg):
    code = msg.get("error") or "?"
    text = REASONS.get(code, f"the stick said {code}" + (f" ({msg['why']})" if msg.get("why") else ""))
    return text.format(why=msg.get("why") or ("dictation" if code == "busy" else "no reason given"))


def faces_dir():
    return config.HOME / "faces"


def valid_config(body):
    """'' if faces_catalog_url in a PUT /api/config body is fine, else what's wrong."""
    url = body.get("faces_catalog_url")
    if url is None:
        return ""
    u = urlparse(url) if isinstance(url, str) else None
    local = u is not None and u.scheme == "http" and u.hostname in ("localhost", "127.0.0.1")
    if not u or not u.netloc or not (u.scheme == "https" or local):
        return "faces_catalog_url must be an https:// URL (or http://localhost for testing)"
    return ""


def needs_firmware_text(fw=None):
    return f"update the stick to {MIN_FIRMWARE} to add faces" + (f" (it runs {fw})" if fw else "")


# --- the compiled face (.sfb) --------------------------------------------------------------

def _unpackbits(data, size):
    out, i = bytearray(), 0
    while len(out) < size and i < len(data):
        c = data[i]
        i += 1
        if c < 128:
            out += data[i:i + c + 1]
            i += c + 1
        elif c > 128:
            out += bytes([data[i]]) * (257 - c) if i < len(data) else b""
            i += 1
    return bytes(out[:size])


def check_sfb(blob):
    """Parses a compiled face and checks it against the budgets. Returns its manifest (dict);
    raises FaceError saying what's wrong. The same reading as tools/faces/facepkg.read_sfb()
    (the stick checks everything again)."""
    def bad(why):
        return FaceError(f"not a valid face file: {why}")
    if len(blob) < 16 or blob[:4] != b"SFB1":
        raise bad("it doesn't start like a compiled face (.sfb)")
    if len(blob) > MAX_SFB:
        raise bad(f"{len(blob)} bytes, the limit is {MAX_SFB}")
    ver, fmt, mlen, nimg, _ = struct.unpack("<HHIHH", blob[4:16])
    if ver != SFB_VERSION:
        raise bad(f"container version {ver} (this StickyBot reads {SFB_VERSION}; update StickyBot)")
    if fmt not in FORMATS:
        raise bad(f"face format {fmt} (this StickyBot reads {', '.join(map(str, FORMATS))}; update StickyBot)")
    if mlen > MAX_MANIFEST or 16 + mlen > len(blob) or nimg > MAX_IMAGES:
        raise bad("bad sizes in the header")
    try:
        man = json.loads(blob[16:16 + mlen].decode())
    except (UnicodeDecodeError, ValueError) as e:
        raise bad("its manifest isn't JSON") from e
    if not isinstance(man, dict):
        raise bad("its manifest isn't an object")
    if not ID_RE.match(str(man.get("id") or "")):
        raise bad("no valid id")
    if not isinstance(man.get("version"), str) or not flasher.version_tuple(man["version"]):
        raise bad("no valid version")
    if man.get("format", fmt) != fmt:
        raise bad("the header and the manifest disagree on the format")
    pos = 16 + mlen + (-mlen % 4)
    pixels = 0
    for n in range(nimg):
        if pos + 16 > len(blob):
            raise bad(f"image {n}: truncated header")
        w, h, _fw, bits, nc1, _clear, enc, _, dlen = struct.unpack("<HHHBBBBHI", blob[pos:pos + 16])
        pos += 16
        nc = nc1 + 1
        if bits not in (1, 2, 4, 8) or not (0 < w <= MAX_IMAGE_SIDE and 0 < h <= MAX_IMAGE_SIDE) or enc > 1:
            raise bad(f"image {n}: bad header")
        pixels += w * h
        if pixels > MAX_PIXELS:
            raise bad(f"more than {MAX_PIXELS} pixels of images")
        pos += 2 * nc + nc % 2 * 2
        if pos + dlen > len(blob):
            raise bad(f"image {n}: data past the end of the file")
        data = blob[pos:pos + dlen]
        pos += dlen + (-dlen % 4)
        stride = (w * bits + 7) // 8
        packed = _unpackbits(data, stride * h) if enc == 1 else data
        if len(packed) != stride * h:
            raise bad(f"image {n}: data too short")
        if nc < 1 << bits:  # every index must be in the palette
            per, mask = 8 // bits, (1 << bits) - 1
            for y in range(h):
                row = packed[y * stride:(y + 1) * stride]
                for x in range(w):
                    if (row[x // per] >> (8 - bits * (x % per + 1))) & mask >= nc:
                        raise bad(f"image {n}: a pixel past its palette")
    if pos > len(blob) + 3:
        raise bad("truncated")
    return man


def sfb_info(blob):
    """{"id", "version", "name", "minFirmware", "format", "size", "sha256", "manifest"} of a
    checked .sfb (raises FaceError)."""
    man = check_sfb(blob)
    return {"id": man["id"], "version": man["version"], "name": man.get("name") or man["id"],
            "minFirmware": man.get("minFirmware") or MIN_FIRMWARE, "format": man.get("format", 1),
            "size": len(blob), "sha256": hashlib.sha256(blob).hexdigest(), "manifest": man}


# --- what the stick can take ---------------------------------------------------------------

def stick_support(state):
    """From the stick's @state: {"supported", "fw", "fmt", "n"} (supported: it has the face
    engine, i.e. `pkgFaces`)."""
    st = state or {}
    pkg = st.get("pkgFaces") if isinstance(st.get("pkgFaces"), dict) else None
    return {"supported": pkg is not None, "fw": st.get("fw"),
            "fmt": [int(f) for f in (pkg or {}).get("fmt") or [] if isinstance(f, int)],
            "n": int((pkg or {}).get("n") or 0)}


def why_not(face, support):
    """None if the stick can take this face (catalog entry or sfb_info), else why not, in words."""
    if not support or not support.get("supported"):
        return needs_firmware_text(support and support.get("fw"))
    need = face.get("minFirmware") or MIN_FIRMWARE
    fw = support.get("fw")
    if fw and flasher.version_tuple(need) > flasher.version_tuple(fw):
        return f"needs firmware {need} (the stick runs {fw}); update the stick first"
    if support.get("fmt") and face.get("format", 1) not in support["fmt"]:
        return f"uses face format {face.get('format')}, which the stick's firmware doesn't read; update the stick"
    return None


# --- the catalog ---------------------------------------------------------------------------

def http_get(url, headers=None, limit=MAX_CATALOG):
    """GET -> (status, headers dict, body bytes); 304 has an empty body. Raises OSError."""
    req = urllib.request.Request(url, headers={"User-Agent": "StickyBot", **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT_S) as r:
            body = r.read(limit + 1)
            if len(body) > limit:
                raise OSError(f"{url}: more than {limit} bytes")
            return r.status, dict(r.headers.items()), body
    except urllib.error.HTTPError as e:
        if e.code == 304:
            return 304, dict(e.headers.items()), b""
        raise OSError(f"{url}: HTTP {e.code}") from e


def verify_signature(data, sig_text, keys=None):
    """True if sig_text (base64 of a raw 64-byte ed25519 signature) is a signature of data by one
    of the keys (base64 raw public keys)."""
    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    try:
        sig = base64.b64decode((sig_text or "").strip(), validate=True)
    except (binascii.Error, ValueError):
        return False
    if len(sig) != 64:
        return False
    for k in CATALOG_KEYS if keys is None else keys:
        try:
            Ed25519PublicKey.from_public_bytes(base64.b64decode(k)).verify(sig, data)
            return True
        except (InvalidSignature, ValueError, binascii.Error):
            continue
    return False


def _https_or_same_host(url, base):
    u, b = urlparse(url), urlparse(base)
    return u.scheme == "https" or (u.scheme == b.scheme and u.netloc == b.netloc)


def parse_catalog(data, url):
    """catalog.json bytes -> the catalog (dict) with absolute URLs; raises FaceError."""
    try:
        cat = json.loads(data.decode())
    except (UnicodeDecodeError, ValueError) as e:
        raise FaceError("the faces catalog isn't valid JSON") from e
    if not isinstance(cat, dict) or not isinstance(cat.get("faces"), list):
        raise FaceError("the faces catalog has no list of faces")
    if cat.get("catalog") != CATALOG_VERSION:
        raise FaceError("the faces catalog is newer than this StickyBot; update StickyBot to browse faces")
    faces = []
    for f in cat["faces"]:
        if not isinstance(f, dict) or not ID_RE.match(str(f.get("id") or "")):
            continue
        if not SHA_RE.match(str(f.get("sha256") or "")) or not isinstance(f.get("size"), int):
            continue
        f = dict(f)
        f["sfb"] = urljoin(url, str(f.get("sfb") or ""))
        if not _https_or_same_host(f["sfb"], url):
            continue
        prev = f.get("previews") if isinstance(f.get("previews"), dict) else {}
        f["previews"] = {k: urljoin(url, str(v)) for k, v in prev.items() if k in ("portrait", "landscape")}
        faces.append(f)
    cat["faces"] = faces
    cat["retired"] = [r for r in cat.get("retired") or [] if isinstance(r, str)]
    return cat


class Catalog:
    """The catalog cache in ~/.stick-agent/faces/: catalog.json (+ .sig, + meta.json with the
    validators and when it was fetched), each face's .sfb and previews under <id>/."""

    def __init__(self, url=None, fetch=None, clock=time.time):
        self.url = url or DEFAULT_CATALOG_URL
        self.fetch = fetch or http_get
        self.clock = clock
        self.error = None  # the last fetch's problem, in words

    @property
    def signed_required(self):
        return self.url == DEFAULT_CATALOG_URL

    def _dir(self):
        d = faces_dir()
        d.mkdir(parents=True, exist_ok=True)
        return d

    def _meta(self):
        try:
            m = json.loads((self._dir() / "meta.json").read_text())
            return m if isinstance(m, dict) and m.get("url") == self.url else {}
        except (OSError, ValueError):
            return {}

    def cached(self):
        """The cached catalog ({"catalog", "fetched", "signed"}) or None (none, another URL,
        or it no longer verifies)."""
        meta = self._meta()
        if not meta:
            return None
        try:
            data = (self._dir() / "catalog.json").read_bytes()
        except OSError:
            return None
        try:
            sig = (self._dir() / "catalog.json.sig").read_text()
        except OSError:
            sig = ""
        signed = bool(sig) and verify_signature(data, sig)
        if self.signed_required and not signed:
            return None
        try:
            return {"catalog": parse_catalog(data, self.url), "fetched": meta.get("fetched"), "signed": signed}
        except FaceError:
            return None

    async def _get(self, url, headers=None, limit=MAX_CATALOG):
        return await asyncio.get_running_loop().run_in_executor(None, self.fetch, url, headers or {}, limit)

    async def refresh(self, force=False):
        """Fetches the catalog if it's due (or force), keeping the last good copy on any
        problem. Returns cached() (or None) and sets self.error."""
        meta = self._meta()
        have = self.cached()
        if have and not force and self.clock() - float(meta.get("fetched") or 0) < REFRESH_S:
            self.error = None
            return have
        headers = {}
        if have and meta.get("etag"):
            headers["If-None-Match"] = meta["etag"]
        if have and meta.get("modified"):
            headers["If-Modified-Since"] = meta["modified"]
        try:
            status, hdrs, data = await self._get(self.url, headers)
            if status == 304 and have:
                meta["fetched"] = self.clock()
                self._save_meta(meta)
                self.error = None
                return self.cached()
            if status != 200:
                raise OSError(f"HTTP {status}")
            cat = parse_catalog(data, self.url)
            sig = ""
            try:
                sstatus, _, sdata = await self._get(self.url + ".sig", {}, 1024)
                sig = sdata.decode(errors="replace") if sstatus == 200 else ""
            except OSError:
                sig = ""
            signed = bool(sig) and verify_signature(data, sig)
            if self.signed_required and not signed:
                raise FaceError("the faces catalog's signature doesn't check out, so it was ignored")
            d = self._dir()
            (d / "catalog.json").write_bytes(data)
            sigf = d / "catalog.json.sig"
            if sig:
                sigf.write_text(sig)
            elif sigf.exists():
                sigf.unlink()
            low = {k.lower(): v for k, v in hdrs.items()}
            self._save_meta({"url": self.url, "fetched": self.clock(), "etag": low.get("etag"),
                             "modified": low.get("last-modified") or formatdate(usegmt=True)})
            self.error = None
            log.info("faces catalog: %d faces (%s)", len(cat["faces"]), "signed" if signed else "unsigned")
            return {"catalog": cat, "fetched": self.clock(), "signed": signed}
        except FaceError as e:
            self.error = str(e)
        except (OSError, ValueError) as e:
            log.info("faces catalog: %s", e)
            self.error = "can't reach the faces catalog right now" + (" (showing the last copy)" if have else "")
        return have

    def _save_meta(self, meta):
        (self._dir() / "meta.json").write_text(json.dumps(meta))

    def entry(self, fid):
        have = self.cached()
        if not have:
            return None
        return next((f for f in have["catalog"]["faces"] if f["id"] == fid), None)

    async def download(self, entry):
        """The entry's .sfb, verified (size, sha256, container, id / version), from the cache or
        the catalog. Raises FaceError."""
        fid, version = entry["id"], entry.get("version") or "?"
        d = self._dir() / fid
        path = d / f"{fid}-{version}.sfb"
        blob = None
        if path.is_file():
            blob = path.read_bytes()
            if hashlib.sha256(blob).hexdigest() != entry["sha256"]:
                blob = None
        if blob is None:
            try:
                status, _, blob = await self._get(entry["sfb"], {}, MAX_SFB)
            except OSError as e:
                raise FaceError("couldn't download the face; check the internet connection and try again") from e
            if status != 200:
                raise FaceError(f"couldn't download the face (HTTP {status}); try again later")
        if len(blob) != entry["size"] or hashlib.sha256(blob).hexdigest() != entry["sha256"]:
            raise FaceError("the download was damaged (it doesn't match the catalog); try again")
        info = sfb_info(blob)
        if info["id"] != fid or info["version"] != entry.get("version"):
            raise FaceError("the downloaded face isn't the one the catalog describes; try again later")
        d.mkdir(parents=True, exist_ok=True)
        for old in d.glob(f"{fid}-*.sfb"):
            if old != path:
                old.unlink()
        path.write_bytes(blob)
        return blob

    async def preview(self, fid, side):
        """A face's preview PNG (bytes), cached by version; None if there's none."""
        if side not in ("portrait", "landscape") or not ID_RE.match(fid or ""):
            return None
        entry = self.entry(fid)
        d = faces_dir() / fid
        if entry:
            path = d / f"{side}-{entry.get('version') or '0'}.png"
        else:  # left the catalog: whatever we have
            found = sorted(d.glob(f"{side}-*.png")) if d.is_dir() else []
            path = found[-1] if found else None
        if path and path.is_file():
            return path.read_bytes()
        url = entry and entry["previews"].get(side)
        if not url:
            return None
        try:
            status, _, data = await self._get(url, {}, MAX_PREVIEW)
        except OSError:
            return None
        if status != 200 or not data.startswith(b"\x89PNG\r\n\x1a\n"):
            return None
        d.mkdir(parents=True, exist_ok=True)
        for old in d.glob(f"{side}-*.png"):
            old.unlink()
        path.write_bytes(data)
        return data


# --- the stick: install, list, remove, use -------------------------------------------------

class Sender(ota.Transfer):
    """Sends one compiled face: FORMAT.md §18 (`faces put` ... `done`)."""

    KIND = "faces"
    Error = FaceError
    Cancelled = FaceCancelled
    WORDS = {"no_answer": "the stick didn't answer the install request; " + needs_firmware_text(),
             "stalled": "the stick stopped answering during the install",
             "unconfirmed": "the stick didn't confirm the face", "failed": "install failed"}

    def begin_message(self, img, force):
        return {"put": {"id": img["id"], "version": img["version"], "size": img["size"], "sha256": img["sha256"]}}

    def reason(self, msg):
        return reason(msg)


async def stick_state(ask):
    st = await ask("@state")
    if not st:
        raise FaceError("the stick didn't answer; check it's on and connected")
    return st


async def installed(ask):
    """The faces on the stick from `@faces` (all pages):
    {"faces": [{"id","v","sz","sha","err"?}], "free", "max"}. Raises FaceError."""
    out, extra = [], {}
    for page in range(MAX_PAGES):
        r = await ask("@faces" if page == 0 else f"@faces page={page}")
        if not r or r.get("ok") is False or not isinstance(r.get("faces"), list):
            if page == 0:
                why = f" ({r['error']})" if r and r.get("error") else ""
                raise FaceError(f"the stick didn't list its faces{why}")
            break
        out += [f for f in r["faces"] if isinstance(f, dict) and f.get("id")]
        extra = {k: r[k] for k in ("free", "max") if k in r}
        if not r.get("more"):
            break
    return {"faces": out, **extra}


async def install(agent, ask, blob, entry=None, emit=None, sender=None):
    """Checks blob (a compiled face; against `entry` too when it's from the catalog) and what
    the stick takes, then sends it and waits for "done". Returns {"id","version","name",...stats}.
    ask(line) -> the stick's reply to a protocol line (or None). Raises FaceError."""
    emit = emit or (lambda e: None)
    if not agent.link.connected:
        raise FaceError("the stick isn't connected by Bluetooth")
    info = sfb_info(blob)
    if entry is not None:
        if info["sha256"] != entry.get("sha256") or info["size"] != entry.get("size"):
            raise FaceError("the face doesn't match the catalog (damaged download); try again")
        if info["id"] != entry["id"] or info["version"] != entry.get("version"):
            raise FaceError("the face file isn't the one the catalog describes")
    st = await stick_state(ask)
    why = why_not(info, stick_support(st))
    if why:
        raise FaceError(why)
    emit({"line": f"installing {info['name']} {info['version']} ({info['size']} bytes) on "
                  f"{st.get('name') or 'the stick'}"})
    s = sender or Sender(agent.link, emit)
    agent.listeners.append(s.on_message)
    try:
        stats = await s.run({"data": blob, "size": info["size"], "sha256": info["sha256"], "id": info["id"],
                             "version": info["version"]})
    finally:
        if s.on_message in agent.listeners:
            agent.listeners.remove(s.on_message)
    done = stats.get("done") or {}
    if done.get("id") and done["id"] != info["id"]:
        raise FaceError(f"the stick confirmed {done['id']}, not {info['id']}")
    emit({"line": f"{info['name']} is on the stick"})
    return {"id": info["id"], "version": done.get("version") or info["version"], "name": info["name"],
            **{k: v for k, v in stats.items() if k != "done"}}


async def _request(agent, msg, states, timeout=REPLY_S):
    """Sends a faces message and returns the stick's first {"evt":"faces"} answer whose state
    is in `states` (None on no answer)."""
    q = asyncio.Queue()

    def listen(m):
        if m.get("src") == "agent" and m.get("evt") == "faces" and m.get("state") in states:
            q.put_nowait(m)
    agent.listeners.append(listen)
    try:
        if not await agent.link.send({"src": "agent", "faces": msg}):
            return None
        try:
            return await asyncio.wait_for(q.get(), timeout)
        except TimeoutError:
            return None
    finally:
        if listen in agent.listeners:
            agent.listeners.remove(listen)


async def remove(agent, fid):
    """Removes a face from the stick (`faces rm`). Raises FaceError."""
    if not ID_RE.match(fid or ""):
        raise FaceError("that isn't a face id")
    if not agent.link.connected:
        raise FaceError("the stick isn't connected by Bluetooth")
    r = await _request(agent, {"rm": fid}, {"removed", "refused", "error"})
    if r is None:
        raise FaceError("the stick didn't answer; check it's on and connected")
    if r["state"] != "removed":
        raise FaceError(reason(r))
    return {"id": r.get("id") or fid}


async def use(ask, fid):
    """Makes it the Home face (`@face id=`). Raises FaceError."""
    if not ID_RE.match(fid or ""):
        raise FaceError("that isn't a face id")
    r = await ask(f"@face id={fid}")
    if not r:
        raise FaceError("the stick didn't answer; check it's on and connected")
    if r.get("ok") is False or r.get("error"):
        raise FaceError(f"the stick didn't switch: {r.get('error') or 'it refused'}")
    return {"id": fid}


def annotate(catalog, support, on_stick):
    """The dashboard's view: each catalog face with "fits" (None, or why the stick can't take
    it) and "installed" / "update"; and each installed face with its catalog entry's name,
    "update", "gone" (left the catalog) and "err" (didn't load on the stick)."""
    faces = (catalog or {}).get("faces") or []
    retired = set((catalog or {}).get("retired") or [])
    by_id = {f["id"]: f for f in faces}
    inst = {f["id"]: f for f in (on_stick or [])}
    out = []
    for f in faces:
        have = inst.get(f["id"])
        out.append({**f, "fits": why_not(f, support) if support else None,
                    "installed": have.get("v") if have else None,
                    "update": bool(have) and _newer(f, have)})
    mine = []
    for f in on_stick or []:
        c = by_id.get(f["id"])
        mine.append({"id": f["id"], "version": f.get("v"), "size": f.get("sz"), "sha": f.get("sha"),
                     "err": f.get("err"), "name": c["name"] if c else f["id"],
                     "author": c.get("author") if c else None, "inCatalog": bool(c),
                     "retired": f["id"] in retired,
                     "update": bool(c) and _newer(c, f) and not (support and why_not(c, support)),
                     "latest": c.get("version") if c else None})
    return out, mine


def _newer(entry, have):
    """The catalog's entry differs from what's installed: a newer version, or the same version
    with another sha (a rebuilt face)."""
    v_cat, v_have = flasher.version_tuple(entry.get("version")), flasher.version_tuple(have.get("v"))
    if v_cat != v_have:
        return v_cat > v_have
    sha = str(have.get("sha") or "").lower()
    return bool(sha) and not entry["sha256"].startswith(sha)


def read_upload(data, name=""):
    """A file the user picked: a compiled face (.sfb). Returns the bytes; raises FaceError."""
    if not data:
        raise FaceError("the file is empty")
    if len(data) > MAX_SFB:
        raise FaceError(f"the file is {len(data) // 1024} KB; a face is 64 KB at most")
    if data[:4] != b"SFB1":
        hint = " (that's a package's manifest: compile it with tools/faces/validate.py --compile DIR)" \
            if Path(name).name == "manifest.json" or data.lstrip()[:1] == b"{" else ""
        raise FaceError("that isn't a compiled face (.sfb)" + hint)
    return data
