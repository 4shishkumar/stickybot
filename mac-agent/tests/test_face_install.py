"""Face packages on the stick (faces.py, faces_api.py; docs/faces/FORMAT.md §18-19, CATALOG.md):
the .sfb check against the reference reader, the install sender against a fake stick that speaks
the `faces` messages (acks, gaps, resume, refusals), `@faces` paging, remove / use, the catalog
cache (signature, conditional fetch, offline, damaged downloads), and the API."""
import asyncio
import base64
import hashlib
import json
import struct
import sys
from pathlib import Path

import pytest
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from test_ota import FakeStick

from stick_agent import config, faces, ota, server

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "faces"))
import facepkg  # noqa: E402

HOST = {"Host": f"localhost:{config.PORT}"}
EXAMPLES = ROOT / "faces" / "examples"


def compiled(fid="catnap"):
    return facepkg.compile_face(facepkg.load(EXAMPLES / fid))


# --- the .sfb check -----------------------------------------------------------------------

@pytest.mark.parametrize("fid", ["textclock", "dialclock", "catnap"])
def test_check_sfb_reads_what_the_compiler_writes(fid):
    blob = compiled(fid)
    man = faces.check_sfb(blob)
    assert man == facepkg.read_sfb(blob)[0]
    info = faces.sfb_info(blob)
    assert info["id"] == fid and info["size"] == len(blob) and info["minFirmware"] == "2.4.0"
    assert info["sha256"] == hashlib.sha256(blob).hexdigest()


def test_check_sfb_refuses_broken_files():
    blob = compiled("catnap")
    for bad, words in [(b"PNG!" + blob[4:], "doesn't start"), (blob[:40], "bad sizes"),
                       (blob + b"\0" * (faces.MAX_SFB - len(blob) + 1), "limit"),
                       (blob[:4] + struct.pack("<H", 2) + blob[6:], "container version 2"),
                       (blob[:6] + struct.pack("<H", 9) + blob[8:], "face format 9"),
                       (blob[:len(blob) - 600], "image")]:
        with pytest.raises(faces.FaceError, match=words):
            faces.check_sfb(bad)


def test_check_sfb_catches_a_pixel_past_the_palette():
    # one 2x1 image, 1 colour at 2 bits per pixel, pixel value 3
    man = json.dumps({"id": "tiny", "version": "1.0.0", "format": 1}).encode()
    blob = b"SFB1" + struct.pack("<HHIHH", 1, 1, len(man), 1, 0) + man + b"\0" * (-len(man) % 4)
    blob += struct.pack("<HHHBBBBHI", 2, 1, 0, 2, 0, 255, 0, 0, 1) + struct.pack("<H", 0) + b"\0\0"
    blob += bytes([0b11000000]) + b"\0\0\0"
    with pytest.raises(faces.FaceError, match="past its palette"):
        faces.check_sfb(blob)


def test_reasons_are_in_words():
    assert "12" in faces.reason({"error": "full"})
    assert "dictation" in faces.reason({"error": "busy", "why": "dictation"})
    assert "image 2: bad header" in faces.reason({"error": "invalid", "why": "image 2: bad header"})
    assert "weird" in faces.reason({"error": "weird"})


def test_support_and_fit():
    old = faces.stick_support({"fw": "2.3.0"})
    assert not old["supported"]
    assert "2.4.0" in faces.why_not({"minFirmware": "2.4.0", "format": 1}, old)
    new = faces.stick_support({"fw": "2.4.0", "pkgFaces": {"n": 2, "fmt": [1]}})
    assert new == {"supported": True, "fw": "2.4.0", "fmt": [1], "n": 2}
    assert faces.why_not({"minFirmware": "2.4.0", "format": 1}, new) is None
    assert "2.6.0" in faces.why_not({"minFirmware": "2.6.0", "format": 1}, new)
    assert "format 2" in faces.why_not({"minFirmware": "2.4.0", "format": 2}, new)


# --- a stick that takes faces ---------------------------------------------------------------

class FacesStick(FakeStick):
    """FakeStick (test_ota.py) speaking the faces family: put / end / cancel / rm, and the
    @state / @faces protocol lines."""

    def __init__(self, deliver, **kw):
        super().__init__(deliver, **kw)
        self.installed = {}     # id -> (version, sha)
        self.state = {"fw": "2.4.0", "name": "stick", "pkgFaces": {"n": 0, "fmt": [1]}}
        self.put = None
        self.removed = []
        self.invalid = None     # "why" for an "invalid" error at the end
        self.pages = []

    def reply(self, **kw):
        self.deliver({"src": "agent", "evt": "faces", **kw})

    def message(self, msg):
        if "proto" in msg:
            self.deliver({"src": "agent", "evt": "proto", "id": msg.get("id"), "reply": self.proto(msg["proto"])})
            return
        o = msg.get("faces")
        if not o:
            return
        if "put" in o:
            p = o["put"]
            self.put = p
            self.begins.append(p)
            if self.refuse:
                self.reply(state="refused", error=self.refuse, why="dictation")
                return
            if self.session and self.session["sha"] == p["sha256"] and self.session["size"] == p["size"]:
                self.gap = False
                self.reply(state="ready", id=p["id"], offset=len(self.session["data"]), window=self.window,
                           chunk=self.chunk, resumed=True)
                return
            self.session = {"size": p["size"], "sha": p["sha256"], "data": bytearray()}
            self.acked = 0
            self.reply(state="ready", id=p["id"], offset=0, window=self.window, chunk=self.chunk, resumed=False)
        elif "end" in o:
            self.ends += 1
            s = self.session
            if not s:
                self.reply(state="error", error="not_started")
            elif len(s["data"]) != s["size"]:
                self.reply(state="error", error="incomplete")
            elif hashlib.sha256(bytes(s["data"])).hexdigest() != s["sha"]:
                self.session = None
                self.reply(state="error", error="sha")
            elif self.invalid:
                self.session = None
                self.reply(state="error", error="invalid", why=self.invalid)
            else:
                self.committed = True
                self.installed[self.put["id"]] = (self.put["version"], s["sha"])
                self.reply(state="done", id=self.put["id"], version=self.put["version"])
        elif "cancel" in o:
            self.session = None
            self.reply(state="cancelled")
        elif "rm" in o:
            if o["rm"] in self.installed:
                del self.installed[o["rm"]]
                self.removed.append(o["rm"])
                self.reply(state="removed", id=o["rm"])
            else:
                self.reply(state="error", error="notfound")

    def proto(self, line):
        self.pages.append(line)
        if line == "@state":
            return {**self.state, "face": "classic"}
        if line.startswith("@faces"):
            page = int(line.split("page=")[1]) if "page=" in line else 0
            items = [{"id": k, "v": v, "sz": 100, "sha": sha[:8]} for k, (v, sha) in sorted(self.installed.items())]
            return {"ok": True, "faces": items[page * 6:(page + 1) * 6], "free": 700, "max": 12,
                    "more": len(items) > (page + 1) * 6}
        if line.startswith("@face id="):
            return {"ok": True}
        return {"ok": False, "error": "unknown"}


class FacesLink:
    """A Bluetooth link to a FacesStick; stick messages go to the agent's listeners."""

    def __init__(self):
        self.connected = True
        self.sent = []
        self.listeners = []
        self.stick = FacesStick(self._deliver)
        self.stick.on_disconnect = self.drop_link
        self.reconnect_after = 0.05
        self.mtu = 247

    def _deliver(self, msg):
        asyncio.get_running_loop().call_soon(lambda: [f(msg) for f in list(self.listeners)])

    def drop_link(self):
        self.connected = False

        async def back():
            await asyncio.sleep(self.reconnect_after)
            self.connected = True
        asyncio.ensure_future(back())

    async def send(self, msg):
        if not self.connected:
            return False
        self.sent.append(msg)
        self.stick.message(msg)
        return True

    async def ota_write(self, pkt):
        if not self.connected:
            return False
        await asyncio.sleep(0)
        self.stick.packet(pkt)
        return True

    def ota_max_write(self):
        return 244


class AgentStub:
    def __init__(self):
        self.link = FacesLink()
        self.listeners = self.link.listeners


def ask_for(agent):
    """ask(line): a protocol line answered by the fake stick (like firmware_api.Stick.ble)."""
    async def ask(line):
        fut = asyncio.get_running_loop().create_future()
        pid = f"p{len(agent.link.sent)}"

        def listen(m):
            if m.get("evt") == "proto" and m.get("id") == pid and not fut.done():
                fut.set_result(m["reply"])
        agent.listeners.append(listen)
        try:
            await agent.link.send({"src": "agent", "proto": line, "id": pid})
            return await asyncio.wait_for(fut, 1)
        finally:
            agent.listeners.remove(listen)
    return ask


@pytest.fixture(autouse=True)
def quick(monkeypatch):
    monkeypatch.setattr(ota, "ACK_WAIT_S", 0.05)
    monkeypatch.setattr(ota, "REPLY_S", 0.3)
    monkeypatch.setattr(ota, "END_REPLY_S", 0.3)
    monkeypatch.setattr(ota, "STALL_S", 1.0)
    monkeypatch.setattr(ota, "RECONNECT_S", 1.0)
    monkeypatch.setattr(faces, "REPLY_S", 0.3)


async def test_install_sends_the_face_and_gets_done():
    agent, events = AgentStub(), []
    blob = compiled("catnap")
    r = await faces.install(agent, ask_for(agent), blob, emit=events.append)
    st = agent.link.stick
    assert st.committed and bytes(st.session["data"]) == blob
    assert st.put == {"id": "catnap", "version": "1.0.0", "size": len(blob),
                      "sha256": hashlib.sha256(blob).hexdigest()}
    assert {"src": "agent", "faces": {"end": 1}} in agent.link.sent
    assert r["id"] == "catnap" and r["version"] == "1.0.0" and r["name"] == "Cat Nap" and r["bytes"] == len(blob)
    assert agent.listeners == []  # cleaned up
    progress = [e["progress"] for e in events if e.get("stage") == "send"]
    assert progress[0] == 0 and progress[-1] == 100
    assert any("is on the stick" in e.get("line", "") for e in events)


async def test_install_resends_after_a_gap_and_resumes_after_a_drop():
    agent = AgentStub()
    blob = compiled("catnap")
    agent.link.stick.drop = {240 * 3}
    agent.link.stick.disconnect_at = 5000
    r = await faces.install(agent, ask_for(agent), blob)
    assert agent.link.stick.committed and r["reconnects"] == 1 and r["resent"] > 0
    assert len(agent.link.stick.begins) == 2


async def test_install_refuses_a_stick_without_the_face_engine():
    agent = AgentStub()
    agent.link.stick.state = {"fw": "2.3.0", "name": "stick"}
    with pytest.raises(faces.FaceError, match="update the stick to 2.4.0"):
        await faces.install(agent, ask_for(agent), compiled("textclock"))
    assert not agent.link.stick.begins


async def test_install_refuses_a_face_that_needs_newer_firmware():
    agent = AgentStub()
    agent.link.stick.state["fw"] = "2.4.0"
    pkg = facepkg.load(EXAMPLES / "textclock")
    pkg.manifest["minFirmware"] = "2.5.0"
    with pytest.raises(faces.FaceError, match="needs firmware 2.5.0"):
        await faces.install(agent, ask_for(agent), facepkg.compile_face(pkg))
    assert not agent.link.stick.begins


async def test_install_checks_the_catalog_entry():
    agent = AgentStub()
    blob = compiled("textclock")
    entry = {"id": "textclock", "version": "1.0.0", "size": len(blob), "sha256": "0" * 64}
    with pytest.raises(faces.FaceError, match="damaged"):
        await faces.install(agent, ask_for(agent), blob, entry)
    entry = {"id": "textclock", "version": "1.0.1", "size": len(blob), "sha256": hashlib.sha256(blob).hexdigest()}
    with pytest.raises(faces.FaceError, match="isn't the one"):
        await faces.install(agent, ask_for(agent), blob, entry)


@pytest.mark.parametrize("code,words", [("full", "remove one"), ("busy", "busy \\(dictation\\)"),
                                        ("ota", "firmware update is running")])
async def test_install_refusals_in_words(code, words):
    agent = AgentStub()
    agent.link.stick.refuse = code
    with pytest.raises(faces.FaceError, match=words):
        await faces.install(agent, ask_for(agent), compiled("textclock"))


async def test_install_reports_a_face_the_stick_finds_invalid():
    agent = AgentStub()
    agent.link.stick.invalid = "image 2: bad header"
    with pytest.raises(faces.FaceError, match="couldn't load this face \\(image 2: bad header\\)"):
        await faces.install(agent, ask_for(agent), compiled("catnap"))


async def test_install_cancel():
    agent = AgentStub()
    s = faces.Sender(agent.link)
    real = agent.link.ota_write

    async def cancel_midway(pkt):
        if struct.unpack("<I", pkt[:4])[0] >= 2000:
            s.cancel()
        return await real(pkt)
    agent.link.ota_write = cancel_midway
    with pytest.raises(faces.FaceCancelled):
        await faces.install(agent, ask_for(agent), compiled("catnap"), sender=s)
    assert {"src": "agent", "faces": {"cancel": 1}} in agent.link.sent
    assert not agent.link.stick.committed


async def test_install_no_answer():
    agent = AgentStub()
    real = agent.link.stick.message
    agent.link.stick.message = lambda m: real(m) if "proto" in m else None
    with pytest.raises(faces.FaceError, match="didn't answer the install request"):
        await faces.install(agent, ask_for(agent), compiled("textclock"))


async def test_installed_reads_every_page():
    agent = AgentStub()
    st = agent.link.stick
    st.installed = {f"face{i:02d}": ("1.0.0", f"{i:064x}") for i in range(9)}
    r = await faces.installed(ask_for(agent))
    assert [f["id"] for f in r["faces"]] == sorted(st.installed)
    assert r["free"] == 700 and r["max"] == 12
    assert st.pages == ["@faces", "@faces page=1"]


async def test_installed_needs_an_answer():
    async def ask(line):
        return None
    with pytest.raises(faces.FaceError, match="didn't list"):
        await faces.installed(ask)


async def test_remove_and_use():
    agent = AgentStub()
    agent.link.stick.installed = {"catnap": ("1.0.0", "ab" * 32)}
    assert await faces.remove(agent, "catnap") == {"id": "catnap"}
    assert {"src": "agent", "faces": {"rm": "catnap"}} in agent.link.sent
    with pytest.raises(faces.FaceError, match="isn't on the stick"):
        await faces.remove(agent, "catnap")
    with pytest.raises(faces.FaceError, match="isn't a face id"):
        await faces.remove(agent, "../x")
    assert await faces.use(ask_for(agent), "catnap") == {"id": "catnap"}
    assert agent.link.stick.pages[-1] == "@face id=catnap"


async def test_remove_no_answer():
    agent = AgentStub()
    agent.link.stick.message = lambda m: None
    with pytest.raises(faces.FaceError, match="didn't answer"):
        await faces.remove(agent, "catnap")


def test_annotate_compares_by_version_and_sha():
    blob = compiled("catnap")
    sha = hashlib.sha256(blob).hexdigest()
    cat = {"faces": [{"id": "catnap", "name": "Cat Nap", "version": "1.1.0", "sha256": sha, "format": 1,
                      "minFirmware": "2.4.0"},
                     {"id": "textclock", "name": "Text Clock", "version": "1.0.0", "sha256": "ab" * 32,
                      "format": 1, "minFirmware": "2.4.0"},
                     {"id": "future", "name": "Future", "version": "1.0.0", "sha256": "cd" * 32, "format": 1,
                      "minFirmware": "2.9.0"}],
           "retired": ["oldface"]}
    support = faces.stick_support({"fw": "2.4.0", "pkgFaces": {"n": 3, "fmt": [1]}})
    on_stick = [{"id": "catnap", "v": "1.0.0", "sha": "12345678"},
                {"id": "textclock", "v": "1.0.0", "sha": "abababab"},
                {"id": "oldface", "v": "1.0.0", "sha": "00000000", "err": "image 0: bad header"}]
    listed, mine = faces.annotate(cat, support, on_stick)
    by = {f["id"]: f for f in listed}
    assert by["catnap"]["update"] and by["catnap"]["installed"] == "1.0.0"
    assert not by["textclock"]["update"]
    assert "2.9.0" in by["future"]["fits"] and by["future"]["installed"] is None
    m = {f["id"]: f for f in mine}
    assert m["catnap"]["update"] and m["catnap"]["name"] == "Cat Nap" and m["catnap"]["latest"] == "1.1.0"
    assert not m["textclock"]["update"]
    assert m["oldface"]["retired"] and not m["oldface"]["inCatalog"] and m["oldface"]["err"]
    # same version, another sha: a rebuilt face is an update too
    _, mine = faces.annotate({"faces": [dict(cat["faces"][1], sha256="ff" * 32)]}, support, on_stick[1:2])
    assert mine[0]["update"]


# --- the catalog ----------------------------------------------------------------------------

@pytest.fixture
def signer(monkeypatch):
    key = Ed25519PrivateKey.generate()
    pub = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    monkeypatch.setattr(faces, "CATALOG_KEYS", ["bm90IGEga2V5IGF0IGFsbCBidXQgMzIgYnl0ZXMhIQ==",
                                                base64.b64encode(pub).decode()])
    return lambda data: base64.b64encode(key.sign(data)).decode() + "\n"


class Web:
    """A fake catalog server: path -> bytes; records requests."""

    def __init__(self, files, base="https://pages.example/stickybot/faces/"):
        self.files, self.base = files, base
        self.calls = []
        self.down = False
        self.etag = '"v1"'

    def __call__(self, url, headers, limit):
        self.calls.append((url, dict(headers)))
        if self.down:
            raise OSError("offline")
        if headers.get("If-None-Match") == self.etag:
            return 304, {}, b""
        path = url[len(self.base):] if url.startswith(self.base) else None
        if path not in self.files:
            raise OSError("HTTP 404")
        body = self.files[path]
        return 200, {"ETag": self.etag, "Last-Modified": "Fri, 06 Nov 2026 12:00:00 GMT"}, body[:limit + 1]


def make_catalog(signer=None, base="https://pages.example/stickybot/faces/"):
    blob = compiled("catnap")
    cat = {"catalog": 1, "generated": "2026-11-06T12:00:00Z", "formats": [1], "retired": [],
           "faces": [{"id": "catnap", "name": "Cat Nap", "version": "1.0.0", "about": "A cat",
                      "author": {"name": "StickyBot", "github": "x"}, "license": "CC0-1.0", "tags": ["animated"],
                      "channel": "examples", "format": 1, "minFirmware": "2.4.0", "motion": "calm",
                      "sfb": "catnap/catnap-1.0.0.sfb", "size": len(blob),
                      "sha256": hashlib.sha256(blob).hexdigest(),
                      "previews": {"portrait": "catnap/portrait.png", "landscape": "catnap/landscape.png"}}]}
    data = json.dumps(cat).encode()
    files = {"catalog.json": data, "catnap/catnap-1.0.0.sfb": blob,
             "catnap/portrait.png": b"\x89PNG\r\n\x1a\nportrait", "catnap/landscape.png": b"\x89PNG\r\n\x1a\nland"}
    if signer:
        files["catalog.json.sig"] = signer(data).encode()
    return Web(files, base), blob


async def test_signature_verification(signer):
    sig = signer(b"hello")
    assert faces.verify_signature(b"hello", sig)
    assert not faces.verify_signature(b"hellO", sig)
    assert not faces.verify_signature(b"hello", "not base64!")
    assert not faces.verify_signature(b"hello", base64.b64encode(b"x" * 64).decode())
    assert not faces.verify_signature(b"hello", sig, keys=[])


def test_the_built_in_key_is_a_key():
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    assert faces.CATALOG_KEYS
    for k in faces.CATALOG_KEYS:
        Ed25519PublicKey.from_public_bytes(base64.b64decode(k))


async def test_catalog_fetch_cache_and_offline(home, signer, monkeypatch):
    web, _ = make_catalog(signer)
    monkeypatch.setattr(faces, "DEFAULT_CATALOG_URL", web.base + "catalog.json")
    now = [1000.0]
    cat = faces.Catalog(web.base + "catalog.json", fetch=web, clock=lambda: now[0])
    have = await cat.refresh()
    assert have["signed"] and have["catalog"]["faces"][0]["sfb"] == web.base + "catnap/catnap-1.0.0.sfb"
    assert (home / "faces" / "catalog.json").is_file() and cat.error is None
    n = len(web.calls)
    assert (await cat.refresh())["catalog"]["faces"][0]["id"] == "catnap"
    assert len(web.calls) == n  # fresh enough: no request
    now[0] += faces.REFRESH_S + 1
    await cat.refresh()
    assert web.calls[-1][1].get("If-None-Match") == '"v1"'  # conditional, answered 304
    web.down = True
    have = await cat.refresh(force=True)
    assert have and "last copy" in cat.error  # offline: the cached copy
    # a fresh agent (another Catalog) also browses offline from the cache
    assert faces.Catalog(web.base + "catalog.json", fetch=web).cached()["catalog"]["faces"]


async def test_the_default_catalog_must_be_signed(home, signer, monkeypatch):
    web, _ = make_catalog(None)
    monkeypatch.setattr(faces, "DEFAULT_CATALOG_URL", web.base + "catalog.json")
    cat = faces.Catalog(web.base + "catalog.json", fetch=web)
    assert await cat.refresh() is None and "signature" in cat.error
    web.files["catalog.json.sig"] = base64.b64encode(b"x" * 64)
    assert await cat.refresh(force=True) is None and "signature" in cat.error
    # another URL (a fork, a test server) may be unsigned
    other = faces.Catalog(web.base + "catalog.json", fetch=make_catalog(None)[0])
    monkeypatch.setattr(faces, "DEFAULT_CATALOG_URL", "https://elsewhere/catalog.json")
    have = await other.refresh()
    assert have and not have["signed"]


async def test_a_tampered_cache_is_ignored(home, signer, monkeypatch):
    web, _ = make_catalog(signer)
    monkeypatch.setattr(faces, "DEFAULT_CATALOG_URL", web.base + "catalog.json")
    cat = faces.Catalog(web.base + "catalog.json", fetch=web)
    await cat.refresh()
    p = home / "faces" / "catalog.json"
    p.write_bytes(p.read_bytes().replace(b"Cat Nap", b"Cat Nab"))
    assert cat.cached() is None


async def test_a_newer_catalog_asks_for_an_update(home):
    web, _ = make_catalog()
    web.files["catalog.json"] = json.dumps({"catalog": 2, "faces": []}).encode()
    cat = faces.Catalog(web.base + "catalog.json", fetch=web)
    assert await cat.refresh() is None and "update StickyBot" in cat.error


async def test_download_verifies_and_caches(home):
    web, blob = make_catalog()
    cat = faces.Catalog(web.base + "catalog.json", fetch=web)
    await cat.refresh()
    entry = cat.entry("catnap")
    assert await cat.download(entry) == blob
    assert (home / "faces" / "catnap" / "catnap-1.0.0.sfb").read_bytes() == blob
    web.down = True
    assert await cat.download(entry) == blob  # from the cache
    web.down = False
    (home / "faces" / "catnap" / "catnap-1.0.0.sfb").unlink()
    web.files["catnap/catnap-1.0.0.sfb"] = blob[:-1] + b"\x01"
    with pytest.raises(faces.FaceError, match="damaged"):
        await cat.download(entry)
    assert not (home / "faces" / "catnap" / "catnap-1.0.0.sfb").exists()


async def test_previews_are_cached(home):
    web, _ = make_catalog()
    cat = faces.Catalog(web.base + "catalog.json", fetch=web)
    await cat.refresh()
    assert (await cat.preview("catnap", "portrait")).endswith(b"portrait")
    web.down = True
    assert (await cat.preview("catnap", "portrait")).endswith(b"portrait")
    assert await cat.preview("catnap", "../../etc") is None
    assert await cat.preview("nope", "portrait") is None


def test_upload_must_be_a_compiled_face():
    assert faces.read_upload(compiled("textclock"))
    with pytest.raises(faces.FaceError, match="validate.py --compile"):
        faces.read_upload(b'{"id": "x"}', "manifest.json")
    with pytest.raises(faces.FaceError, match="64 KB"):
        faces.read_upload(b"SFB1" + b"\0" * faces.MAX_SFB)
    with pytest.raises(faces.FaceError, match="empty"):
        faces.read_upload(b"")


def test_config_url_must_be_https():
    assert faces.valid_config({"faces_catalog_url": "https://x.example/catalog.json"}) == ""
    assert faces.valid_config({"faces_catalog_url": "http://localhost:8000/catalog.json"}) == ""
    assert faces.valid_config({"faces_catalog_url": "http://evil.example/c.json"})
    assert faces.valid_config({"faces_catalog_url": 3})
    assert faces.valid_config({}) == ""


# --- API --------------------------------------------------------------------------------------

@pytest.fixture
async def client(agent, aiohttp_client, monkeypatch, signer):
    from stick_agent import flasher
    monkeypatch.setattr(flasher, "espressif_ports", lambda: [])
    lk = FacesLink()
    agent.link = lk
    lk.listeners.append(agent._on_message)
    web, blob = make_catalog(signer)
    monkeypatch.setattr(faces, "http_get", web)
    monkeypatch.setattr(faces, "DEFAULT_CATALOG_URL", web.base + "catalog.json")
    agent.cfg["faces_catalog_url"] = web.base + "catalog.json"
    c = await aiohttp_client(server.make_app(agent))
    c.agent, c.web, c.blob = agent, web, blob
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def events_of(client, job):
    r = await client.get(f"/api/faces/jobs/{job}", headers=HOST)
    return [json.loads(line[5:]) for line in (await r.text()).splitlines() if line.startswith("data:")]


async def test_api_needs_the_token(client):
    for path in ("/api/faces/install", "/api/faces/remove", "/api/faces/use", "/api/faces/jobs/x/cancel"):
        r = await client.post(path, headers=HOST, json={})
        assert r.status == 403


async def test_api_catalog_with_the_stick(client):
    client.agent.link.stick.installed = {"catnap": ("0.9.0", "12345678" + "0" * 56)}
    d = await (await client.get("/api/faces/catalog", headers=HOST)).json()
    assert d["signed"] and d["error"] is None
    assert d["faces"][0]["id"] == "catnap" and d["faces"][0]["fits"] is None and d["faces"][0]["update"]
    assert d["faces"][0]["previews"]["portrait"] == "/api/faces/preview/catnap/portrait"
    assert d["installed"][0]["update"] and d["installed"][0]["name"] == "Cat Nap"
    assert d["stick"]["supported"] and d["stick"]["max"] == 12 and d["stick"]["fw"] == "2.4.0"
    r = await client.get("/api/faces/preview/catnap/landscape", headers=HOST)
    assert r.status == 200 and r.content_type == "image/png"


async def test_api_catalog_with_old_firmware(client):
    client.agent.link.stick.state = {"fw": "2.3.0", "name": "stick"}
    d = await (await client.get("/api/faces/catalog", headers=HOST)).json()
    assert not d["stick"]["supported"] and "2.4.0" in d["stick"]["error"]
    assert d["installed"] is None and "2.4.0" in d["faces"][0]["fits"]


async def test_api_catalog_offline_without_a_stick(client):
    await client.get("/api/faces/catalog", headers=HOST)
    client.web.down = True
    client.agent.link.connected = False
    d = await (await client.get("/api/faces/catalog?refresh=1", headers=HOST)).json()
    assert d["offline"] and d["faces"] and not d["stick"]["connected"]


async def test_api_install_from_the_catalog(client):
    r = await client.post("/api/faces/install", headers=client.auth, json={"id": "catnap"})
    events = await events_of(client, (await r.json())["job"])
    last = events[-1]
    assert last["ok"], last
    assert last["result"]["id"] == "catnap"
    assert any(e.get("stage") == "download" for e in events)
    assert client.agent.link.stick.installed["catnap"][0] == "1.0.0"
    assert not client.agent.faces_busy


async def test_api_install_from_a_file(client):
    blob = compiled("textclock")
    r = await client.post("/api/faces/install", headers=client.auth,
                          json={"file": base64.b64encode(blob).decode(), "name": "textclock.sfb"})
    last = (await events_of(client, (await r.json())["job"]))[-1]
    assert last["ok"] and last["result"]["id"] == "textclock"
    r = await client.post("/api/faces/install", headers=client.auth,
                          json={"file": base64.b64encode(b"nope").decode(), "name": "x.png"})
    assert r.status == 400 and "compiled face" in (await r.json())["error"]


async def test_api_install_errors(client):
    r = await client.post("/api/faces/install", headers=client.auth, json={"id": "nosuch"})
    assert r.status == 404
    client.agent.link.stick.refuse = "full"
    r = await client.post("/api/faces/install", headers=client.auth, json={"id": "catnap"})
    last = (await events_of(client, (await r.json())["job"]))[-1]
    assert not last["ok"] and last["error"].startswith("Your stick has no room")
    client.agent.link.connected = False
    r = await client.post("/api/faces/install", headers=client.auth, json={"id": "catnap"})
    assert r.status == 409


async def test_api_remove_and_use(client):
    client.agent.link.stick.installed = {"catnap": ("1.0.0", "ab" * 32)}
    r = await client.post("/api/faces/remove", headers=client.auth, json={"id": "catnap"})
    assert (await r.json()) == {"ok": True, "id": "catnap"}
    r = await client.post("/api/faces/remove", headers=client.auth, json={"id": "catnap"})
    assert r.status == 409
    r = await client.post("/api/faces/use", headers=client.auth, json={"id": "catnap"})
    assert (await r.json())["ok"]


async def test_firmware_update_waits_for_a_face_install(client):
    client.agent.faces_busy = True
    r = await client.post("/api/firmware/ota", headers=client.auth, json={})
    assert r.status == 409 and "face" in (await r.json())["error"]
