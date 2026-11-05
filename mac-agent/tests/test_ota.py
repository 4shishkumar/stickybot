"""Firmware update over Bluetooth (ota.py): the sender against a fake stick that
speaks the protocol (acks, gaps, resume, sha check), dropped packets and links,
cancel, progress, the version / support checks, the API and the CLI; and the
Link's write pacing (canSendWriteWithoutResponse)."""
import asyncio
import hashlib
import json
import struct

import pytest

from stick_agent import config, firmware_api, flasher, link, ota, server

HOST = {"Host": f"localhost:{config.PORT}"}


def app_image(n=50_000):
    data = bytearray((i * 7 + (i >> 8)) & 0xFF for i in range(n))
    data[0] = 0xE9
    return bytes(data)


class FakeStick:
    """The stick's side of the protocol (like logic/ota_rx.cpp), in memory."""

    ACK_EVERY = 4096

    def __init__(self, deliver, window=16384, chunk=240):
        self.deliver = deliver  # msg -> the agent (asynchronously)
        self.window, self.chunk = window, chunk
        self.session = None     # {"size", "sha", "data"}
        self.received = 0
        self.acked = 0
        self.gap = False
        self.begins = []
        self.refuse = None      # error code for the next begin
        self.drop = set()       # packet offsets to lose once
        self.disconnect_at = None  # offset: the link drops when data reaches it
        self.cancel_at = None   # offset: B pressed on the stick
        self.corrupt = False
        self.ends = 0
        self.committed = False
        self.on_disconnect = None

    def reply(self, **kw):
        self.deliver({"src": "agent", "evt": "ota", **kw})

    def message(self, msg):
        o = msg.get("ota")
        if not o:
            return
        if "begin" in o:
            b = o["begin"]
            self.begins.append(b)
            if self.refuse:
                self.reply(state="refused", error=self.refuse, why="dictation")
                return
            if self.session and self.session["sha"] == b["sha256"] and self.session["size"] == b["size"]:
                self.gap = False
                self.reply(state="ready", offset=len(self.session["data"]), size=b["size"], resumed=True,
                           window=self.window, chunk=self.chunk)
                return
            self.session = {"size": b["size"], "sha": b["sha256"], "data": bytearray()}
            self.reply(state="ready", offset=0, size=b["size"], resumed=False, window=self.window, chunk=self.chunk)
        elif "end" in o:
            self.ends += 1
            s = self.session
            if not s:
                self.reply(state="error", error="not_started", offset=0)
            elif len(s["data"]) != s["size"]:
                self.reply(state="error", error="incomplete", offset=len(s["data"]))
            elif hashlib.sha256(bytes(s["data"])).hexdigest() != s["sha"]:
                self.session = None
                self.reply(state="error", error="sha", offset=s["size"])
            else:
                self.committed = True
                self.reply(state="done", offset=s["size"])
        elif "cancel" in o:
            self.session = None
            self.reply(state="cancelled", by="agent", offset=0)

    def packet(self, pkt):
        off = struct.unpack("<I", pkt[:4])[0]
        body = pkt[4:]
        if off in self.drop:
            self.drop.discard(off)
            return
        s = self.session
        if not s:
            return
        have = len(s["data"])
        if off > have:
            if not self.gap:
                self.gap = True
                self.reply(state="ack", offset=have, gap=True)
            return
        if off + len(body) <= have:
            return
        if self.corrupt and have == 0:
            body = bytes([body[0]]) + bytes([body[1] ^ 1]) + body[2:]
        s["data"] += body[have - off:]
        self.received += len(body) - (have - off)
        self.gap = False
        have = len(s["data"])
        if have - self.acked >= self.ACK_EVERY or have == s["size"]:
            self.acked = have
            self.reply(state="ack", offset=have)
        if self.cancel_at is not None and have >= self.cancel_at:
            self.cancel_at = None
            self.session = None
            self.reply(state="cancelled", by="stick", offset=have)
        if self.disconnect_at is not None and have >= self.disconnect_at:
            self.disconnect_at = None
            if self.on_disconnect:
                self.on_disconnect()


class OtaLink:
    """FakeLink + the OTA characteristic, wired to a FakeStick."""

    def __init__(self):
        self.connected = True
        self.sent = []
        self.writes = 0
        self.listener = None
        self.stick = FakeStick(self._deliver)
        self.stick.on_disconnect = self.drop_link
        self.reconnect_after = 0.05
        self.mtu = 247

    def _deliver(self, msg):
        asyncio.get_running_loop().call_soon(lambda: self.listener and self.listener(msg))

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
        self.writes += 1
        if self.writes % 8 == 0:
            await asyncio.sleep(0)  # let replies in, like real Bluetooth pacing
        self.stick.packet(pkt)
        return True

    def ota_max_write(self):
        return 244


@pytest.fixture(autouse=True)
def quick(monkeypatch):
    monkeypatch.setattr(ota, "ACK_WAIT_S", 0.05)
    monkeypatch.setattr(ota, "REPLY_S", 0.3)
    monkeypatch.setattr(ota, "END_REPLY_S", 0.3)
    monkeypatch.setattr(ota, "STALL_S", 1.0)
    monkeypatch.setattr(ota, "RECONNECT_S", 1.0)


def img(data=None, version="1.8.1"):
    data = data or app_image()
    return {"data": data, "sha256": hashlib.sha256(data).hexdigest(), "size": len(data), "version": version}


def sender_for(lk, events=None):
    s = ota.Sender(lk, (events.append if events is not None else None))
    lk.listener = s.on_message
    return s


async def test_sends_the_whole_image_and_gets_done():
    lk, events = OtaLink(), []
    s = sender_for(lk, events)
    im = img()
    stats = await s.run(im)
    assert lk.stick.committed and bytes(lk.stick.session["data"]) == im["data"]
    assert stats["bytes"] == im["size"] and stats["resent"] == 0 and stats["reconnects"] == 0
    assert stats["kbps"] > 0
    begin = lk.sent[0]["ota"]["begin"]
    assert begin == {"size": im["size"], "sha256": im["sha256"], "version": "1.8.1", "compressed": False,
                     "force": False}
    assert {"src": "agent", "ota": {"end": 1}} in lk.sent
    # packets: [offset u32][<= chunk bytes]; chunk = min(stick's, max write - 4)
    progress = [e["progress"] for e in events if e.get("stage") == "send"]
    assert progress[0] == 0 and progress[-1] == 100 and progress == sorted(progress)
    assert any(e.get("stage") == "verify" and e["progress"] == 100 for e in events)
    assert any("KB/s" in e.get("line", "") for e in events)


async def test_never_more_than_the_window_in_flight():
    lk = OtaLink()
    lk.stick.window = 4096
    s = sender_for(lk)
    seen = []
    real = lk.ota_write

    async def track(pkt):
        off = struct.unpack("<I", pkt[:4])[0]
        seen.append(off + len(pkt) - 4 - len(lk.stick.session["data"]))
        return await real(pkt)
    lk.ota_write = track
    await s.run(img())
    assert max(seen) <= 4096 + 240


async def test_chunk_is_limited_by_the_mac_side():
    lk = OtaLink()
    lk.ota_max_write = lambda: 100
    sizes = []
    real = lk.ota_write

    async def track(pkt):
        sizes.append(len(pkt))
        return await real(pkt)
    lk.ota_write = track
    await sender_for(lk).run(img(app_image(5000)))
    assert max(sizes) == 100


async def test_dropped_packets_are_resent_after_a_gap():
    lk = OtaLink()
    lk.stick.drop = {240 * 10, 240 * 50, 240 * 51}
    s = sender_for(lk)
    stats = await s.run(img())
    assert lk.stick.committed
    assert stats["resent"] > 0


async def test_lost_ack_times_out_and_resends():
    lk = OtaLink()
    s = sender_for(lk)
    real = lk._deliver
    state = {"lost": 0}

    def lossy(msg):  # the first two acks never arrive
        if msg.get("state") == "ack" and state["lost"] < 2:
            state["lost"] += 1
            return
        real(msg)
    lk.stick.deliver = lossy
    stats = await s.run(img())
    assert lk.stick.committed and state["lost"] == 2
    assert stats["bytes"] == 50_000


async def test_resumes_after_the_link_drops():
    lk, events = OtaLink(), []
    lk.stick.disconnect_at = 20_000
    s = sender_for(lk, events)
    stats = await s.run(img())
    assert lk.stick.committed
    assert stats["reconnects"] == 1
    assert len(lk.stick.begins) == 2  # the second begin resumed
    assert any("resuming" in e.get("line", "") for e in events)
    assert lk.stick.received == 50_000  # nothing before the drop was sent twice... into the image


async def test_gives_up_when_the_stick_doesnt_come_back(monkeypatch):
    lk = OtaLink()
    lk.stick.disconnect_at = 10_000
    lk.reconnect_after = 30
    with pytest.raises(ota.OtaError, match="didn't come back"):
        await sender_for(lk).run(img())


async def test_refused_begin_explains_why():
    lk = OtaLink()
    lk.stick.refuse = "busy"
    with pytest.raises(ota.OtaError, match="busy \\(dictation\\)"):
        await sender_for(lk).run(img())
    lk.stick.refuse = "battery"
    with pytest.raises(ota.OtaError, match="30 %"):
        await sender_for(lk).run(img())


async def test_no_answer_to_begin_means_old_firmware():
    lk = OtaLink()
    lk.stick.message = lambda msg: None  # 1.7.x ignores {"ota":...}
    with pytest.raises(ota.OtaError, match="older than 1.8.0"):
        await sender_for(lk).run(img())


async def test_sha_mismatch_on_the_stick_fails():
    lk = OtaLink()
    lk.stick.corrupt = True
    with pytest.raises(ota.OtaError, match="checksum"):
        await sender_for(lk).run(img())
    assert not lk.stick.committed


async def test_cancel_from_the_mac():
    lk = OtaLink()
    s = sender_for(lk)
    real = lk.ota_write

    async def slow(pkt):
        if struct.unpack("<I", pkt[:4])[0] >= 10_000:
            s.cancel()
        return await real(pkt)
    lk.ota_write = slow
    with pytest.raises(ota.OtaCancelled):
        await s.run(img())
    assert {"src": "agent", "ota": {"cancel": 1}} in lk.sent
    assert not lk.stick.committed and lk.stick.session is None


async def test_cancel_on_the_stick():
    lk = OtaLink()
    lk.stick.cancel_at = 8000
    with pytest.raises(ota.OtaCancelled, match="on the stick"):
        await sender_for(lk).run(img())


async def test_incomplete_end_resumes():
    lk = OtaLink()
    s = sender_for(lk)
    real_message = lk.stick.message
    state = {"cut": False}

    def message(msg):
        if "end" in msg.get("ota", {}) and not state["cut"]:  # pretend the stick lost the last bytes
            state["cut"] = True
            del lk.stick.session["data"][-100:]
        real_message(msg)
    lk.stick.message = message
    await s.run(img())
    assert lk.stick.committed and lk.stick.ends == 2


# --- the bundle, support and version checks --------------------------------------

@pytest.fixture
def bundle(tmp_path, monkeypatch):
    d = tmp_path / "firmware"
    d.mkdir()
    app = app_image(3000)
    parts = []
    for offset, name, data in [("0x0", "bootloader.bin", b"\xe9" + b"\0" * 99),
                               ("0x8000", "partitions.bin", b"\0" * 64),
                               ("0xe000", "boot_app0.bin", b"\xff" * 64), ("0x10000", "app.bin", app)]:
        (d / name).write_bytes(data)
        parts.append({"offset": offset, "file": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    (d / "manifest.json").write_text(json.dumps({"version": "1.8.1", "board": "m5-sticks3", "built": "today",
                                                  "sha256": "x", "size": 1, "parts": parts}))
    monkeypatch.setattr(flasher, "BUNDLE", d)
    return d


def test_bundled_app_uses_only_the_app_part(bundle):
    a = ota.bundled_app()
    assert a["size"] == 3000 and a["version"] == "1.8.1"
    assert a["sha256"] == hashlib.sha256((bundle / "app.bin").read_bytes()).hexdigest()


def test_bundled_app_refuses_a_tampered_image(bundle):
    (bundle / "app.bin").write_bytes(b"\xe9" + b"\1" * 2999)
    with pytest.raises(ota.OtaError, match="sha256"):
        ota.bundled_app()


def test_bundled_app_needs_a_bundle(monkeypatch):
    monkeypatch.setattr(flasher, "bundled", lambda: None)
    with pytest.raises(ota.OtaError, match="no bundled firmware"):
        ota.bundled_app()


def test_support_and_versions():
    assert ota.supported({"fw": "1.8.0", "ota": 1})
    assert not ota.supported({"fw": "1.7.2"}) and not ota.supported(None)
    assert ota.check_versions("1.8.1", "1.8.0") is None
    assert "downgrade" in ota.check_versions("1.7.2", "1.8.0")
    assert "already" in ota.check_versions("1.8.0", "1.8.0")
    assert ota.check_versions("1.7.2", "1.8.0", force=True) is None


class AgentStub:
    def __init__(self, lk):
        self.link = lk
        self.listeners = []


async def test_update_refuses_old_firmware(bundle):
    lk = OtaLink()

    async def state():
        return {"fw": "1.7.2", "name": "stick"}
    with pytest.raises(ota.OtaError, match="over USB"):
        await ota.update(AgentStub(lk), state)
    assert lk.sent == []


async def test_update_refuses_a_downgrade_without_force(bundle):
    async def state():
        return {"fw": "1.9.0", "ota": 1}
    with pytest.raises(ota.OtaError, match="force"):
        await ota.update(AgentStub(OtaLink()), state)


async def test_update_end_to_end_waits_for_the_new_version(bundle, monkeypatch):
    lk = OtaLink()
    agent = AgentStub(lk)
    lk.listener = lambda m: [f(m) for f in list(agent.listeners)]
    answers = iter([{"fw": "1.8.0", "ota": 1, "name": "stick"}, None, {"fw": "1.8.1", "ota": 1, "otaPending": True},
                    {"fw": "1.8.1", "ota": 1, "otaPending": False}])

    async def state():
        return next(answers)
    real_sleep = asyncio.sleep
    monkeypatch.setattr(ota.asyncio, "sleep", lambda s: real_sleep(0))
    events = []
    r = await ota.update(agent, state, events.append)
    assert r["from"] == "1.8.0" and r["to"] == "1.8.1" and r["version"] == "1.8.1" and r["confirmed"]
    assert agent.listeners == []  # cleaned up
    assert any("confirmed" in e.get("line", "") for e in events)


async def test_update_reports_a_rollback(bundle, monkeypatch):
    lk = OtaLink()
    agent = AgentStub(lk)
    lk.listener = lambda m: [f(m) for f in list(agent.listeners)]
    first = {"fw": "1.8.0", "ota": 1}

    async def state():
        return first
    real_sleep = asyncio.sleep
    monkeypatch.setattr(ota.asyncio, "sleep", lambda s: real_sleep(0))
    with pytest.raises(ota.OtaError, match="rolled back"):
        await ota.update(agent, state, reboot_s=0.2)


# --- Link pacing -----------------------------------------------------------------

class Periph:
    def __init__(self, busy):
        self.busy = busy
        self.checks = 0

    def canSendWriteWithoutResponse(self):  # noqa: N802 — Core Bluetooth's name
        self.checks += 1
        if self.busy:
            self.busy -= 1
            return False
        return True


class Client:
    def __init__(self, periph):
        self.is_connected = True
        self._backend = type("B", (), {"_peripheral": periph})()
        self.writes = []
        self.services = type("S", (), {"get_characteristic": lambda _s, u: type(
            "C", (), {"max_write_without_response_size": 244})()})()

    async def write_gatt_char(self, uuid, data, response):
        self.writes.append((uuid, bytes(data), response))


async def test_link_ota_write_waits_for_room():
    lk = link.Link(lambda m: None, lambda a: None, lambda s: None)
    p = Periph(busy=3)
    lk.client = Client(p)
    assert await lk.ota_write(b"\0\0\0\0abc")
    assert p.checks == 4
    assert lk.client.writes == [(link.OTA_DATA, b"\0\0\0\0abc", False)]
    assert lk.ota_max_write() == 244


async def test_link_ota_write_when_down():
    lk = link.Link(lambda m: None, lambda a: None, lambda s: None)
    assert not await lk.ota_write(b"x")


# --- API + CLI ---------------------------------------------------------------------

@pytest.fixture
async def client(agent, aiohttp_client, bundle, monkeypatch):
    monkeypatch.setattr(flasher, "espressif_ports", lambda: [])
    c = await aiohttp_client(server.make_app(agent))
    c.agent = agent
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def test_api_needs_the_token(client):
    for path in ("/api/firmware/ota", "/api/firmware/jobs/x/cancel"):
        r = await client.post(path, headers=HOST, json={})
        assert r.status == 403


async def test_api_needs_the_link(client):
    client.agent.link.connected = False
    r = await client.post("/api/firmware/ota", headers=client.auth, json={})
    assert r.status == 409 and "Bluetooth" in (await r.json())["error"]


async def test_api_runs_a_job_and_streams_it(client, monkeypatch):
    seen = {}

    async def fake_update(agent, query_state, emit, force, sender=None):
        seen.update(force=force, sender=sender)
        emit({"progress": 0, "stage": "send"})
        emit({"line": "sending"})
        emit({"progress": 100, "stage": "reboot"})
        return {"to": "1.8.1", "version": "1.8.1", "confirmed": True}

    monkeypatch.setattr(ota, "update", fake_update)
    r = await client.post("/api/firmware/ota", headers=client.auth, json={"force": True})
    job = (await r.json())["job"]
    r = await client.get(f"/api/firmware/jobs/{job}", headers=HOST)
    events = [json.loads(line[5:]) for line in (await r.text()).splitlines() if line.startswith("data:")]
    assert events[-1] == {"done": True, "ok": True, "error": None,
                          "result": {"to": "1.8.1", "version": "1.8.1", "confirmed": True}}
    assert seen["force"] is True and isinstance(seen["sender"], ota.Sender)


async def test_api_cancel_reaches_the_sender(client, monkeypatch):
    gate = asyncio.Event()

    async def fake_update(agent, query_state, emit, force, sender=None):
        await gate.wait()
        if sender.cancelled.is_set():
            raise ota.OtaCancelled("cancelled")
        return {}

    monkeypatch.setattr(ota, "update", fake_update)
    job = (await (await client.post("/api/firmware/ota", headers=client.auth, json={})).json())["job"]
    r = await client.post("/api/firmware/ota", headers=client.auth, json={})
    assert r.status == 409  # one at a time
    r = await client.post(f"/api/firmware/jobs/{job}/cancel", headers=client.auth, json={})
    assert r.status == 200
    gate.set()
    r = await client.get(f"/api/firmware/jobs/{job}", headers=HOST)
    last = [json.loads(line[5:]) for line in (await r.text()).splitlines() if line.startswith("data:")][-1]
    assert last["ok"] is False and last["error"] == "cancelled"
    r = await client.post(f"/api/firmware/jobs/{job}/cancel", headers=client.auth, json={})
    assert r.status == 409


async def test_connected_stick_reports_ota(client):
    async def send(msg):
        if "proto" in msg:
            asyncio.get_running_loop().call_soon(client.agent._on_message, {
                "src": "agent", "evt": "proto", "id": msg["id"],
                "reply": {"ok": True, "fw": "1.8.0", "name": "s", "ota": 1}})
        return True
    client.agent.link.send = send
    d = await (await client.get("/api/firmware", headers=HOST)).json()
    assert d["connected"] == {"name": "s", "version": "1.8.0", "ota": True}


class FakeResp:
    def __init__(self, body=b"", lines=()):
        self.body, self.lines = body, list(lines)

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def read(self, *a):
        return self.body

    def __iter__(self):
        return iter(self.lines)

    def close(self):
        pass


def test_cli_follows_the_job(monkeypatch, home):
    calls = []

    def api(path, body=None, timeout=10):
        calls.append((path, body))
        if path == "/api/firmware/ota":
            return FakeResp(b'{"job": "j1"}')
        return FakeResp(lines=[b": keep-alive\n", b'data: {"line": "updating"}\n',
                               b'data: {"progress": 50, "stage": "send"}\n',
                               b'data: {"done": true, "ok": true, "error": null}\n'])
    monkeypatch.setattr(ota, "_api", api)
    out = []
    assert ota.cli(force=True, out=lambda *a, **k: out.append(a[0])) == 0
    assert calls[0] == ("/api/firmware/ota", {"force": True})
    assert "updating" in out


def test_cli_reports_failure(monkeypatch, home):
    def api(path, body=None, timeout=10):
        if path == "/api/firmware/ota":
            return FakeResp(b'{"job": "j1"}')
        return FakeResp(lines=[b'data: {"done": true, "ok": false, "error": "the stick is busy"}\n'])
    monkeypatch.setattr(ota, "_api", api)
    out = []
    assert ota.cli(out=lambda *a, **k: out.append(a[0])) == 1
    assert "failed: the stick is busy" in out


def test_cli_without_agent(monkeypatch, home):
    def api(path, body=None, timeout=10):
        raise OSError("connection refused")
    monkeypatch.setattr(ota, "_api", api)
    out = []
    assert ota.cli(out=lambda *a, **k: out.append(a[0])) == 1
    assert "isn't running" in out[0]


def test_firmware_api_doc_lists_ota():
    assert "/api/firmware/ota" in firmware_api.__doc__
