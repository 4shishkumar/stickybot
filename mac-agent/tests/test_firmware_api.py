"""Firmware / stick / onboarding API: contract shapes, token, flash job SSE,
port safety, protocol passthrough (Bluetooth and USB), onboarding steps."""
import asyncio
import json

import pytest

from stick_agent import config, firmware_api, flasher, server

HOST = {"Host": f"localhost:{config.PORT}"}
STICK_PORT = {"port": "/dev/cu.usbmodemSTICK", "serial": "ABC123", "product": "stick-key", "pid": 0x1001}
BOOT_PORT = {"port": "/dev/cu.usbmodem1101", "serial": "", "product": "USB JTAG/serial debug unit", "pid": 0x1001}
STATE = {"ok": True, "board": "m5-sticks3", "fw": "1.6.0", "name": "my-stick"}


@pytest.fixture
def bundle(tmp_path, monkeypatch):
    d = tmp_path / "firmware"
    d.mkdir()
    parts = []
    for offset, name, size in [("0x0", "bootloader.bin", 20000), ("0x8000", "partitions.bin", 3072),
                               ("0xe000", "boot_app0.bin", 8192), ("0x10000", "app.bin", 101)]:
        (d / name).write_bytes(b"\xe9" + b"\0" * (size - 1))
        parts.append({"offset": offset, "file": name, "size": size, "sha256": "p"})
    (d / "manifest.json").write_text(json.dumps({"version": "1.6.0", "board": "m5-sticks3", "built": "today",
                                                  "sha256": "x", "size": 101, "parts": parts}))
    monkeypatch.setattr(flasher, "BUNDLE", d)
    return d


@pytest.fixture
def ports(monkeypatch):
    found = [STICK_PORT]
    monkeypatch.setattr(flasher, "espressif_ports", lambda: list(found))
    return found


@pytest.fixture
def usb(monkeypatch):
    calls = []

    def fake(port, line, timeout_s=2.5):
        calls.append((port, line))
        return {**STATE, "line": line}

    monkeypatch.setattr(firmware_api, "usb_proto", fake)
    return calls


def stick_replies(agent, reply=None):
    """Makes the FakeLink answer {"proto"} messages like the stick does."""
    real_send = agent.link.send

    async def send(msg):
        ok = await real_send(msg)
        if ok and "proto" in msg:
            asyncio.get_running_loop().call_soon(
                agent._on_message, {"src": "agent", "evt": "proto", "id": msg["id"],
                                    "reply": reply if reply is not None else {**STATE, "line": msg["proto"]}})
        return ok

    agent.link.send = send


@pytest.fixture
async def client(agent, aiohttp_client, bundle, ports, usb):
    c = await aiohttp_client(server.make_app(agent))
    c.agent = agent
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def test_firmware_shape(client):
    stick_replies(client.agent)
    r = await client.get("/api/firmware", headers=HOST)
    assert r.status == 200
    d = await r.json()
    assert set(d) == {"bundled", "usb", "connected"}
    assert d["bundled"] == {"version": "1.6.0", "board": "m5-sticks3", "built": "today", "sha256": "x", "size": 101}
    assert d["usb"] == [{"port": STICK_PORT["port"], "serial": "ABC123", "version": "1.6.0", "board": "m5-sticks3"}]
    assert d["connected"] == {"name": "my-stick", "version": "1.6.0"}


async def test_firmware_without_bundle_or_link(client, monkeypatch):
    monkeypatch.setattr(flasher, "bundled", lambda: None)
    client.agent.link.connected = False
    d = await (await client.get("/api/firmware", headers=HOST)).json()
    assert d["bundled"] is None and d["connected"] is None


async def test_download_mode_port_listed_without_opening_it(client, ports, usb):
    ports[:] = [BOOT_PORT]
    d = await (await client.get("/api/firmware", headers=HOST)).json()
    assert d["usb"][0]["board"] == "download mode" and d["usb"][0]["version"] is None
    assert usb == []


async def test_writes_need_the_token(client):
    for path in ("/api/firmware/flash", "/api/stick/proto", "/api/onboarding"):
        r = await client.post(path, headers=HOST, json={})
        assert r.status == 403, path


async def test_flash_refuses_non_stick_ports(client):
    r = await client.post("/api/firmware/flash", headers=client.auth, json={"port": "/dev/cu.usbmodemLGMONITOR"})
    assert r.status == 400 and "refusing" in (await r.json())["error"]


async def test_flash_job_streams_events(client, monkeypatch):
    seen = {}

    async def fake_flash(port, backup=False, emit=None, parts=None):
        seen.update(port=port, backup=backup)
        emit({"progress": 0, "stage": "reset"})
        emit({"line": "Writing..."})
        emit({"progress": 55, "stage": "flash"})
        emit({"progress": 100, "stage": "reboot"})

    monkeypatch.setattr(flasher, "flash", fake_flash)
    r = await client.post("/api/firmware/flash", headers=client.auth, json={"port": STICK_PORT["port"], "backup": True})
    job = (await r.json())["job"]
    r = await client.get(f"/api/firmware/jobs/{job}", headers=HOST)
    body = (await r.text()).strip().split("\n\n")
    events = [json.loads(b.removeprefix("data: ")) for b in body if b.startswith("data: ")]
    assert events[0] == {"progress": 0, "stage": "reset"}
    assert {"line": "Writing..."} in events
    assert events[-1] == {"done": True, "ok": True, "error": None}
    assert seen == {"port": STICK_PORT["port"], "backup": True}


async def test_flash_failure_reported(client, monkeypatch):
    async def boom(port, backup=False, emit=None, parts=None):
        raise flasher.FlashError("esptool failed (exit 2)")

    monkeypatch.setattr(flasher, "flash", boom)
    job = (await (await client.post("/api/firmware/flash", headers=client.auth,
                                    json={"port": STICK_PORT["port"]})).json())["job"]
    text = await (await client.get(f"/api/firmware/jobs/{job}", headers=HOST)).text()
    last = json.loads(text.strip().split("\n\n")[-1].removeprefix("data: "))
    assert last == {"done": True, "ok": False, "error": "esptool failed (exit 2)"}


async def test_unknown_job(client):
    r = await client.get("/api/firmware/jobs/nope", headers=HOST)
    assert r.status == 404


async def test_proto_over_bluetooth(client, usb):
    stick_replies(client.agent)
    d = await (await client.post("/api/stick/proto", headers=client.auth, json={"line": "@state"})).json()
    assert d["ok"] and d["via"] == "ble" and d["reply"]["line"] == "@state" and d["error"] is None
    sent = [m for m in client.agent.link.sent if "proto" in m]
    assert sent and sent[0]["proto"] == "@state" and sent[0]["src"] == "agent"
    assert usb == []  # didn't touch USB


async def test_proto_falls_back_to_usb(client, usb):
    client.agent.link.connected = False
    d = await (await client.post("/api/stick/proto", headers=client.auth,
                                 json={"line": "@combo text=Option+Cmd"})).json()
    assert d["ok"] and d["via"] == "usb"
    assert usb == [(STICK_PORT["port"], "@combo text=Option+Cmd")]


async def test_proto_errors(client, ports):
    client.agent.link.connected = False
    d = await (await client.post("/api/stick/proto", headers=client.auth, json={"line": "state"})).json()
    assert not d["ok"] and "expected" in d["error"]
    ports[:] = []
    d = await (await client.post("/api/stick/proto", headers=client.auth, json={"line": "@state"})).json()
    assert not d["ok"] and d["via"] is None and "no stick" in d["error"]
    ports[:] = [STICK_PORT, {**STICK_PORT, "port": "/dev/cu.other"}]
    d = await (await client.post("/api/stick/proto", headers=client.auth, json={"line": "@state"})).json()
    assert "several" in d["error"]


async def test_proto_stick_error_reply(client):
    stick_replies(client.agent, {"ok": False, "error": "unknown action"})
    d = await (await client.post("/api/stick/proto", headers=client.auth, json={"line": "@nope"})).json()
    assert d["via"] == "ble" and not d["ok"] and d["error"] == "unknown action"


async def test_onboarding_auto_steps(client, home):
    stick_replies(client.agent, {**STATE, "name": "stick-key"})
    client.agent.cfg["stick"] = {"address": "X", "name": "stick-key"}
    d = await (await client.get("/api/onboarding", headers=HOST)).json()
    s = d["steps"]
    assert s["firmware"] and s["paired"] and s["picked"]
    assert not s["named"]           # still the default name
    assert not s["repo"]            # default commands still have <org>/<repo>
    assert not s["tested"] and not d["done"]
    config.COMMANDS.write_text(json.dumps({"commands": [{"open_url": "https://github.com/a/b/pull/{n}"}]}))
    config.HISTORY.write_text(json.dumps({"heard": "hi", "source": "voice"}) + "\n")
    s = (await (await client.get("/api/onboarding", headers=HOST)).json())["steps"]
    assert s["repo"] and s["tested"]


async def test_onboarding_old_firmware(client, usb, monkeypatch):
    stick_replies(client.agent, {**STATE, "fw": "1.5.0"})
    monkeypatch.setattr(firmware_api, "usb_proto", lambda port, line, timeout_s=2.5: {**STATE, "fw": "1.5.0"})
    s = (await (await client.get("/api/onboarding", headers=HOST)).json())["steps"]
    assert not s["firmware"]


async def test_onboarding_marks_and_reset(client):
    stick_replies(client.agent, {**STATE, "name": "stick-key"})
    d = await (await client.post("/api/onboarding", headers=client.auth, json={"step": "named", "done": True})).json()
    assert d["steps"]["named"]
    assert client.agent.cfg["onboarding"] == {"named": True}
    d = await (await client.post("/api/onboarding", headers=client.auth, json={"step": "reset"})).json()
    assert not d["steps"]["named"] and client.agent.cfg["onboarding"] == {}
    r = await client.post("/api/onboarding", headers=client.auth, json={"step": "bogus"})
    assert r.status == 400


# --- flasher (no real esptool / serial) ----------------------------------------------

def test_version_tuple():
    assert flasher.version_tuple("1.6.0") == (1, 6, 0)
    assert flasher.version_tuple("1.10.2") > flasher.version_tuple("1.9.9")
    assert flasher.version_tuple(None) == ()


def test_check_port(ports):
    assert flasher.check_port(STICK_PORT["port"])["serial"] == "ABC123"
    with pytest.raises(flasher.FlashError):
        flasher.check_port("/dev/cu.usbmodemLGMONITOR")


def test_in_bootloader():
    assert flasher.in_bootloader(BOOT_PORT) and not flasher.in_bootloader(STICK_PORT)


async def test_flash_sequence(bundle, ports, monkeypatch):
    """Touch -> bootloader port -> write-flash with the manifest's settings."""
    calls = []
    monkeypatch.setattr(flasher, "touch_1200", lambda port: calls.append(("touch", port)))
    monkeypatch.setattr(flasher, "wait_for_bootloader", lambda port, timeout_s=12.0: BOOT_PORT["port"])

    async def fake_esptool(args, emit, stage):
        calls.append((stage, args))
        emit({"progress": 100, "stage": stage})

    monkeypatch.setattr(flasher, "run_esptool", fake_esptool)
    events = []
    await flasher.flash(STICK_PORT["port"], backup=True, emit=events.append)
    assert calls[0] == ("touch", STICK_PORT["port"])
    backup = next(a for s, a in calls if s == "backup")
    assert "read-flash" in backup and BOOT_PORT["port"] in backup
    write = next(a for s, a in calls if s == "flash")
    assert "write-flash" in write
    assert write[-8:] == ["0x0", str(bundle / "bootloader.bin"), "0x8000", str(bundle / "partitions.bin"),
                          "0xe000", str(bundle / "boot_app0.bin"), "0x10000", str(bundle / "app.bin")]
    i = write.index("--before")
    assert write[i:i + 4] == ["--before", "usb-reset", "--after", "hard-reset"]
    assert events[-1]["line"].startswith("done")


async def test_flash_already_in_download_mode(bundle, ports, monkeypatch):
    ports[:] = [BOOT_PORT]
    monkeypatch.setattr(flasher, "touch_1200", lambda port: pytest.fail("no touch needed"))

    async def fake_esptool(args, emit, stage):
        pass

    monkeypatch.setattr(flasher, "run_esptool", fake_esptool)
    lines = []
    await flasher.flash(BOOT_PORT["port"], emit=lines.append)
    assert any("already in download mode" in e.get("line", "") for e in lines)


async def test_run_esptool_parses_progress(monkeypatch):
    """Progress redraws (\\r) become progress events; other lines are passed through."""
    script = ("import sys; print('Connecting...'); "
              "sys.stdout.write('Writing 10.0%\\rWriting 55.5%\\r'); print('Hash of data verified.')")

    real_exec = asyncio.create_subprocess_exec

    async def fake_exec(*cmd, **kw):
        return await real_exec("/usr/bin/env", "python3", "-c", script, **kw)

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_exec)
    events = []
    await flasher.run_esptool(["write-flash"], events.append, "flash")
    assert {"line": "Connecting..."} in events and {"line": "Hash of data verified."} in events
    assert {"progress": 10, "stage": "flash"} in events and {"progress": 55, "stage": "flash"} in events


async def test_flash_without_bundle(monkeypatch, ports):
    monkeypatch.setattr(flasher, "bundled", lambda: None)
    with pytest.raises(flasher.FlashError, match="no bundled firmware"):
        await flasher.flash(STICK_PORT["port"])


def test_parts_never_touch_the_settings(tmp_path, bundle):
    """The stick's NVS (0x9000-0xDFFF: pairings, combo, Mac password...) must never be written."""
    flasher.check_parts(flasher.bundled()["files"])  # the real layout is fine
    big = tmp_path / "merged.bin"
    big.write_bytes(b"\xe9" * 0x20000)  # a merged image from 0x0 would cover NVS
    with pytest.raises(flasher.FlashError, match="settings"):
        flasher.check_parts([("0x0", str(big))])
    small = tmp_path / "boot.bin"
    small.write_bytes(b"\xe9" * 0x8001)  # its last sector is 0x8000-0x9000: right up to NVS, not into it
    flasher.check_parts([("0x0", str(small))])
    edge = tmp_path / "edge.bin"
    edge.write_bytes(b"\xe9" * 0x9001)  # its last sector (0x9000-0xA000) is inside NVS
    with pytest.raises(flasher.FlashError):
        flasher.check_parts([("0x0", str(edge))])
