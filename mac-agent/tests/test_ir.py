"""The IR remote on the Mac (ir.py, ir_api.py, the MCP tools, voice quick commands) against a fake
stick that speaks "@ir" (firmware 2.5.0): listing, sending learned buttons and built-in code sets,
learning with its events, rename / remove / star, export / import, and an old firmware."""
import asyncio
import json
import urllib.parse

import pytest
from conftest import FakeLink
from test_mcp import client  # noqa: F401 — the MCP client fixture

from stick_agent import config, ir, server

HOST = {"Host": f"localhost:{config.PORT}"}


class IRStick:
    """The stick's side of @ir (like src/ir/ir.cpp), in memory."""

    def __init__(self, fw="2.5.0"):
        self.fw = fw
        self.devices = {}   # id -> [{"n","c","f"}]
        self.sent = []      # (dev, btn) or ("code", code), repeat
        self.learning = None

    def state(self):
        st = {"ok": True, "fw": self.fw, "name": "stick"}
        if self.fw >= "2.5.0":
            st["ir"] = {"v": 1, "n": len(self.devices), "tx": 46, "rx": 42}
        return st

    def run(self, text, emit):
        if text == "@state":
            return self.state()
        if not text.startswith("@ir ") or self.fw < "2.5.0":
            return {"ok": False, "error": "Unknown command: ir"}
        a = {k: urllib.parse.unquote(v) for k, v in (p.split("=", 1) for p in text[4:].split("&"))}
        op, dev, btn = a.get("op"), a.get("dev"), a.get("btn")
        rows = self.devices.get(dev)
        find = lambda: next((r for r in rows or [] if r["n"] == btn), None)
        nf = {"ok": False, "error": "No such remote or button", "code": "notfound"}
        if op == "list":
            if not dev:
                return {"ok": True, "devices": [{"id": d, "n": len(r), "fav": sum(x.get("f", 0) for x in r)}
                                                for d, r in sorted(self.devices.items())], "max": 16, "free": 700}
            if rows is None:
                return nf
            page = int(a.get("page", 0))
            part = rows[page * 12:(page + 1) * 12]
            buttons = [{"n": r["n"], "p": r["c"].split(":")[0], **({"f": 1} if r.get("f") else {})} for r in part]
            return {"ok": True, "dev": dev, "buttons": buttons, "more": len(rows) > (page + 1) * 12}
        if op == "get":
            r = find()
            return {"ok": True, "dev": dev, "btn": btn, "c": r["c"]} if r else nf
        if op == "send":
            if a.get("code"):
                self.sent.append((("code", a["code"]), int(a.get("repeat", 0))))
                return {"ok": True, "sent": "code"}
            if not find():
                return nf
            self.sent.append(((dev, btn), int(a.get("repeat", 0))))
            return {"ok": True, "sent": btn, "dev": dev}
        if op == "put":
            rows = self.devices.setdefault(dev, [])
            old = find()
            if old:
                old.update(c=a["code"], f=int(a.get("fav", 0)))
            else:
                rows.append({"n": btn, "c": a["code"], "f": int(a.get("fav", 0))})
            return {"ok": True, "dev": dev, "btn": btn}
        if op == "rm":
            if rows is None:
                return nf
            if not btn:
                del self.devices[dev]
            elif find():
                rows.remove(find())
            else:
                return nf
            return {"ok": True}
        if op == "rename":
            if rows is None:
                return nf
            if btn:
                find()["n"] = a["to"]
            else:
                self.devices[a["to"]] = self.devices.pop(dev)
            return {"ok": True}
        if op == "fav":
            r = find()
            r["f"] = int(a.get("on", 0))
            return {"ok": True, "fav": bool(r["f"])}
        if op == "learn":
            self.learning = (dev, btn)
            emit({"src": "agent", "evt": "ir", "state": "learning", "dev": dev, "btn": btn, "step": 1})
            return {"ok": True, "learning": True, "dev": dev, "btn": btn or ""}
        if op == "cancel":
            self.learning = None
            emit({"src": "agent", "evt": "ir", "state": "cancelled", "by": "agent"})
            return {"ok": True}
        if op == "diag":
            return {"ok": True, "tx": 46, "rx": 42, "txIdle": 0, "rxIdle": 1, "pins": "ok"}
        return {"ok": False, "error": "Unknown @ir op", "code": "op"}

    def press(self, emit, code="nec:4:8"):
        """The user presses the remote twice: the stick saves it."""
        dev, btn = self.learning
        btn = btn or "button 1"
        emit({"src": "agent", "evt": "ir", "state": "learning", "dev": dev, "btn": btn, "step": 2})
        self.devices.setdefault(dev, []).append({"n": btn, "c": code})
        self.learning = None
        emit({"src": "agent", "evt": "ir", "state": "saved", "dev": dev, "btn": btn, "p": code.split(":")[0]})


class IRLink(FakeLink):
    def __init__(self, agent, stick):
        super().__init__()
        self.agent, self.stick = agent, stick

    def deliver(self, msg):
        asyncio.get_running_loop().call_soon(self.agent._on_message, msg)

    async def send(self, msg):
        if not await super().send(msg):
            return False
        if "proto" in msg:
            reply = self.stick.run(msg["proto"], self.deliver)
            self.deliver({"src": "agent", "evt": "proto", "id": msg.get("id"), "reply": reply})
        return True


@pytest.fixture
def stick(agent):
    s = IRStick()
    agent.link = IRLink(agent, s)
    return s


@pytest.fixture
async def api(agent, stick, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.hdr = {**HOST, "X-Stick-Token": config.token()}
    return c


# --- names, lines, codes -----------------------------------------------------------------------

def test_names_match_the_sticks():
    assert ir.device_id("  Living Room_TV ") == "living-room-tv"
    assert ir.device_id("AC!") == "ac"
    assert ir.device_id("a very long device name indeed") == "a-very-long-devi"
    assert ir.device_id(" -- ") == ""
    assert ir.button_name("  Volume   Up ") == "volume up"
    assert ir.button_name("Cool 24°C") == "cool 24c"
    assert ir.line("send", dev="tv", btn="volume up", repeat=0) == "@ir op=send&dev=tv&btn=volume%20up&repeat=0"
    assert ir.line("learn", dev="tv", btn="", once=None) == "@ir op=learn&dev=tv"


def test_codes():
    for good in ("nec:4:8", "samsung:7:2", "sony12:1:15", "rc5:0:c", "kaseikyo:2002:8:3d", "panasonic:8:3d",
                 "raw:38:9000,4500,560", "raw:38:3400,1700,420:0120"):
        assert ir.valid_code(good), good
    for bad in ("", "nec:4", "tv:1:2", "raw:38:", "nec:4:8; rm -rf ~", "raw:38:1,2:ABC"):
        assert not ir.valid_code(bad), bad
    for b in ir.BRANDS.values():  # every built-in code is one the stick reads
        assert all(ir.valid_code(c) for c in b["buttons"].values())
        assert {"power", "volume up", "volume down", "mute"} <= set(b["buttons"])


# --- the agent side ----------------------------------------------------------------------------

async def test_list_and_send(agent, stick):
    stick.devices = {"tv": [{"n": "power", "c": "nec:4:8", "f": 1}, {"n": "volume up", "c": "nec:4:2"}],
                     "ac": [{"n": f"cool {t}", "c": "raw:38:1,2,3"} for t in range(16, 31)]}
    d = await agent.ir.devices()
    tv = next(x for x in d["devices"] if x["id"] == "tv")
    assert tv["buttons"][0] == {"name": "power", "proto": "nec", "fav": True}
    ac = next(x for x in d["devices"] if x["id"] == "ac")
    assert len(ac["buttons"]) == 15  # two pages
    assert await agent.ir.send("TV", "Volume Up", repeat=2) == "tv: volume up sent"
    assert stick.sent == [(("tv", "volume up"), 2)]
    with pytest.raises(ir.IRError, match="no 'mute' button on the 'tv' remote"):
        await agent.ir.send("tv", "mute")


async def test_brand_fallback(agent, stick):
    agent.cfg["ir_brands"] = {"tv": "samsung"}
    assert await agent.ir.send("tv", "power") == "tv: power sent"
    assert stick.sent == [(("code", "samsung:7:2"), 0)]
    listing = await agent.ir.listing()
    assert "power" in listing["tv"] and "mute" in listing["tv"]
    stick.devices = {"tv": [{"n": "power", "c": "nec:4:8"}]}  # a learned one wins
    await agent.ir.send("tv", "power")
    assert stick.sent[-1] == (("tv", "power"), 0)


async def test_old_firmware(agent, stick):
    stick.fw = "2.4.0"
    with pytest.raises(ir.IRError, match="needs 2.5.0"):
        await agent.ir.send("tv", "power")
    assert ir.supported(stick.state()) == (False, "the stick's firmware (2.4.0) has no IR remote: it needs 2.5.0")
    agent.link.connected = False
    with pytest.raises(ir.IRError, match="isn't connected"):
        await agent.ir.devices()


async def test_learning_reports_its_steps(agent, stick):
    seen = []
    agent.subscribers.add(q := asyncio.Queue())
    task = asyncio.ensure_future(agent.ir.learn_and_wait("TV", "Power"))
    await asyncio.sleep(0.05)
    assert stick.learning == ("tv", "power")
    stick.press(agent.link.deliver)
    final = await asyncio.wait_for(task, 1)
    assert final["state"] == "saved" and final["btn"] == "power"
    while not q.empty():
        ev = q.get_nowait()
        if ev["kind"] == "ir":
            seen.append((ev["state"], ev.get("step")))
    assert ("learning", 1) in seen and ("learning", 2) in seen and seen[-1][0] == "saved"
    assert agent.ir.learn["state"] == "saved"
    assert stick.devices["tv"] == [{"n": "power", "c": "nec:4:8"}]


async def test_export_import_round_trip(agent, stick):
    stick.devices = {"tv": [{"n": "power", "c": "nec:4:8", "f": 1}],
                     "ac": [{"n": "cool 24", "c": "raw:38:3400,1700,420:0120"}]}
    doc = await agent.ir.export()
    assert doc == {"stickybot_ir": 1, "devices": {"ac": [{"n": "cool 24", "c": "raw:38:3400,1700,420:0120"}],
                                                  "tv": [{"n": "power", "c": "nec:4:8", "f": 1}]}}
    stick.devices = {}
    doc["devices"]["tv"].append({"n": "bad", "c": "nec:zz"})
    r = await agent.ir.import_doc(json.loads(json.dumps(doc)))
    assert r["added"] == 2 and len(r["skipped"]) == 1
    assert stick.devices["tv"] == [{"n": "power", "c": "nec:4:8", "f": 1}]
    with pytest.raises(ir.IRError, match="expected"):
        await agent.ir.import_doc({"nope": 1})


async def test_install_brand(agent, stick):
    r = await agent.ir.install_brand("Bedroom TV", "lg")
    assert r == {"dev": "bedroom-tv", "n": 7}
    rows = stick.devices["bedroom-tv"]
    assert {"n": "power", "c": "nec:4:8", "f": 1} in rows


# --- voice -------------------------------------------------------------------------------------

async def test_voice_quick_command(agent, stick):
    stick.devices = {"tv": [{"n": "power", "c": "nec:4:8"}]}
    await agent.handle_text("Turn off the TV.")
    assert stick.sent == [(("tv", "power"), 0)]
    assert ("done", "tv: power sent") in agent.link.states()
    await agent.handle_text("mute the tv")  # no such button, no brand: says so (not Claude)
    assert agent.link.states()[-1][0] == "error" and "mute" in agent.link.states()[-1][1]


def test_voice_commands_added_once(home):
    doc = {"commands": [{"name": "Mine", "phrases": ["hello"], "shell": "echo hi"}]}
    config.COMMANDS.write_text(json.dumps(doc))
    cfg, saved = {}, []
    assert ir.ensure_voice_commands(cfg, lambda c: (saved.append(c), cfg.update(c)))
    cmds = json.loads(config.COMMANDS.read_text())["commands"]
    assert cmds[0]["name"] == "Mine" and any(c.get("ir") == {"dev": "tv", "btn": "power"} for c in cmds)
    assert not ir.ensure_voice_commands(cfg, saved.append)  # once
    defaults = json.loads((config.DEFAULTS / "commands.json").read_text())["commands"]
    assert any("turn off the tv" in c["phrases"] for c in defaults if "ir" in c)


# --- the brain's tools --------------------------------------------------------------------------

async def test_mcp_tools(client, stick):  # noqa: F811
    stick.devices = {"ac": [{"n": "cool 24", "c": "raw:38:1,2,3"}, {"n": "off", "c": "nec:1:2"}]}
    text, err, res = await client.call("ir_list")
    assert not err and "ac: cool 24, off" in text
    assert res["structuredContent"]["devices"]["ac"] == ["cool 24", "off"] if "structuredContent" in res else True
    text, err, _ = await client.call("ir_send", {"device": "ac", "button": "cool 24"})
    assert not err and text == "ac: cool 24 sent"
    text, err, _ = await client.call("ir_send", {"device": "ac", "button": "heat 30"})
    assert err and "no 'heat 30' button" in text
    from stick_agent import mcp_server
    assert "mcp__stick__ir_send" in mcp_server.allowed_tool_names()


# --- the dashboard API -------------------------------------------------------------------------

async def test_api(api, stick, agent):
    stick.devices = {"tv": [{"n": "power", "c": "nec:4:8"}]}
    r = await (await api.get("/api/ir", headers=HOST)).json()
    assert r["supported"] and r["devices"][0]["buttons"][0]["name"] == "power" and "samsung" in r["brands"]
    r = await api.post("/api/ir/send", json={"dev": "tv", "btn": "power"}, headers=api.hdr)
    assert (await r.json()) == {"ok": True, "result": "tv: power sent"}
    r = await api.post("/api/ir/send", json={"brand": "sony", "btn": "Mute"}, headers=api.hdr)
    assert r.status == 200 and stick.sent[-1] == (("code", "sony12:1:14"), 0)
    r = await api.post("/api/ir/send", json={"dev": "tv", "btn": "nope"}, headers=api.hdr)
    assert r.status == 409
    r = await api.post("/api/ir/brand", json={"dev": "TV", "brand": "lg"}, headers=api.hdr)
    assert (await r.json())["mapped"] == {"tv": "lg"} and agent.cfg["ir_brands"] == {"tv": "lg"}
    r = await api.post("/api/ir/learn", json={"dev": "tv", "btn": "input"}, headers=api.hdr)
    assert r.status == 200 and stick.learning == ("tv", "input")
    r = await api.post("/api/ir/rename", json={"dev": "tv", "btn": "power", "to": "on off"}, headers=api.hdr)
    assert r.status == 200 and stick.devices["tv"][0]["n"] == "on off"
    r = await api.get("/api/ir/export", headers=HOST)
    assert "attachment" in r.headers["Content-Disposition"] and (await r.json())["devices"]["tv"]
    r = await api.post("/api/ir/remove", json={"dev": "tv"}, headers=api.hdr)
    assert r.status == 200 and stick.devices == {}
    r = await api.post("/api/ir/send", json={"dev": "tv", "btn": "power"}, headers=HOST)  # no token
    assert r.status == 403
    r = await (await api.get("/api/ir/diag", headers=HOST)).json()
    assert r["pins"] == "ok"
