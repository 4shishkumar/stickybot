"""The stick's IR remote (firmware 2.5.0; PROTOCOL.md "IR remote"): what's learned on the stick,
learning / sending / renaming from the dashboard, voice quick commands ({"ir": {"dev", "btn"}})
and the brain's ir_list() / ir_send() tools.

The stick keeps the codes (/ir/<device>.json in its flash) so its Remote screen works without the
Mac; the agent asks for them with "@ir ..." lines over Bluetooth. Built-in code sets (BRANDS) for a
few TV brands work without learning: a device can be mapped to a brand (config "ir_brands",
{"tv": "samsung"}) or a brand's buttons installed on the stick as a device.
"""
import asyncio
import json
import logging
import re
import urllib.parse
import uuid

from . import config

log = logging.getLogger("ir")

MIN_FIRMWARE = "2.5.0"
REPLY_S = 4.0
LEARN_STEP_S = 30
MAX_DEVICES, MAX_BUTTONS = 16, 48

# --- Built-in code sets ------------------------------------------------------------------------
# Only codes published in more than one well-known place; each set says where. The code strings
# are the stick's (logic/ir_codec.h): <protocol>:<address hex>:<command hex>.
BRANDS = {
    # Samsung TVs, Samsung32 protocol, address 0x07 (0xE0E0 as LIRC writes the first two bytes):
    # power E0E040BF, vol+ E0E0E01F, vol- E0E0D02F, mute E0E0F00F, source E0E0807F,
    # ch+ E0E048B7, ch- E0E008F7. LIRC remotes database, samsung/BN59-00603A
    # (https://sourceforge.net/p/lirc-remotes/code/ci/master/tree/remotes/samsung/) and
    # Flipper-IRDB TVs/Samsung (https://github.com/Lucaslhm/Flipper-IRDB), "Samsung32 07 ...".
    "samsung": {"title": "Samsung TV", "buttons": {
        "power": "samsung:7:2", "volume up": "samsung:7:7", "volume down": "samsung:7:b", "mute": "samsung:7:f",
        "input": "samsung:7:1", "channel up": "samsung:7:12", "channel down": "samsung:7:10"}},
    # LG TVs speak NEC, address 0x04 (0x20DF in LIRC): power 20DF10EF, vol+ 20DF40BF, vol- 20DFC03F,
    # mute 20DF906F, input 20DFD02F, ch+ 20DF00FF, ch- 20DF807F. LIRC lg/AKB72915207
    # (https://sourceforge.net/p/lirc-remotes/code/ci/master/tree/remotes/lg/) and Flipper-IRDB TVs/LG.
    "lg": {"title": "LG TV", "buttons": {
        "power": "nec:4:8", "volume up": "nec:4:2", "volume down": "nec:4:3", "mute": "nec:4:9",
        "input": "nec:4:b", "channel up": "nec:4:0", "channel down": "nec:4:1"}},
    # Sony TVs, SIRC 12-bit, device 1: 16 ch+, 17 ch-, 18 vol+, 19 vol-, 20 mute, 21 power, 37 input.
    # SB-Projects (https://www.sbprojects.net/knowledge/ir/sirc.php) and the JP1 Sony TV table
    # (http://www.hifi-remote.com/sony/Sony_tv.htm).
    "sony": {"title": "Sony TV", "buttons": {
        "power": "sony12:1:15", "volume up": "sony12:1:12", "volume down": "sony12:1:13", "mute": "sony12:1:14",
        "input": "sony12:1:25", "channel up": "sony12:1:10", "channel down": "sony12:1:11"}},
    # Philips TVs, RC5 system 0: 12 standby, 13 mute, 16 vol+, 17 vol-, 32 ch+, 33 ch-, 56 input (AV).
    # SB-Projects' RC-5 command table (https://www.sbprojects.net/knowledge/ir/rc5.php) and
    # https://en.wikipedia.org/wiki/RC-5 (system 0 = TV1). Newer Philips sets use RC6 with the same numbers.
    "philips": {"title": "Philips TV (RC5)", "buttons": {
        "power": "rc5:0:c", "volume up": "rc5:0:10", "volume down": "rc5:0:11", "mute": "rc5:0:d",
        "channel up": "rc5:0:20", "channel down": "rc5:0:21", "input": "rc5:0:38"}},
}

CODE_RE = re.compile(r"^(?:(?:nec|samsung|lg|sony|sony12|sony15|sony20|rc5|rc6|panasonic)"
                     r":[0-9a-fA-F]{1,8}:[0-9a-fA-F]{1,8}"
                     r"|kaseikyo:[0-9a-fA-F]{1,4}:[0-9a-fA-F]{1,3}:[0-9a-fA-F]{1,2}"
                     r"|raw:\d{2}:[0-9,]{1,4000}(?::[0-9a-z]{1,800})?)$")

# Default voice phrases (commands.json); "tv" works with a learned "tv" remote or a brand set
VOICE_COMMANDS = [
    {"name": "TV power", "phrases": ["turn off the tv", "turn on the tv", "tv off", "tv on", "tv power",
                                     "switch off the tv", "switch on the tv"], "ir": {"dev": "tv", "btn": "power"}},
    {"name": "TV volume up", "phrases": ["tv volume up", "turn the tv up", "louder"],
     "ir": {"dev": "tv", "btn": "volume up", "repeat": 2}},
    {"name": "TV volume down", "phrases": ["tv volume down", "turn the tv down", "quieter"],
     "ir": {"dev": "tv", "btn": "volume down", "repeat": 2}},
    {"name": "TV mute", "phrases": ["mute the tv", "unmute the tv", "tv mute"], "ir": {"dev": "tv", "btn": "mute"}},
]


class IRError(Exception):
    pass


def device_id(name):
    """Like the stick's irnames::deviceId: "Living Room TV" -> "living-room-tv" ('' if nothing's left)."""
    s = re.sub(r"[\s_-]+", "-", str(name or "").lower())
    s = re.sub(r"[^a-z0-9-]", "", s).strip("-")
    return s[:16].rstrip("-")


def button_name(name):
    """Like irnames::buttonName: "  Volume  Up " -> "volume up"."""
    s = re.sub(r"[^a-z0-9 +\-_.#\t]", "", str(name or "").lower())
    return " ".join(s.split())[:24].strip()


def valid_code(code):
    return bool(CODE_RE.match(str(code or "")))


def q(v):
    return urllib.parse.quote(str(v), safe="")


def line(op, **args):
    """'@ir op=send&dev=tv&btn=volume%20up' (the &-form: values may have spaces)."""
    parts = [f"op={op}"] + [f"{k}={q(v)}" for k, v in args.items() if v is not None and v != ""]
    return "@ir " + "&".join(parts)


def supported(state):
    """@state -> (supported, why)."""
    if not state:
        return False, "the stick didn't answer"
    if "ir" in state:
        return True, None
    return False, f"the stick's firmware ({state.get('fw') or '?'}) has no IR remote: it needs {MIN_FIRMWARE}"


def ensure_voice_commands(cfg, save):
    """Adds the IR voice phrases to an existing commands.json once (new installs get them from the defaults)."""
    if cfg.get("ir_voice_added"):
        return False
    try:
        doc = json.loads(config.COMMANDS.read_text())
    except (OSError, ValueError):
        return False
    cmds = doc.setdefault("commands", [])
    if not any("ir" in c for c in cmds):
        cmds.extend(json.loads(json.dumps(VOICE_COMMANDS)))
        config.COMMANDS.write_text(json.dumps(doc, indent=2))
    save({"ir_voice_added": True})
    return True


class IR:
    """The agent's side; agent: core.Agent (link, listeners, emit, cfg)."""

    def __init__(self, agent):
        self.agent = agent
        self.learn = None  # the last learn's state as the stick reported it (for the dashboard)
        self._waiters = []  # (predicate, future) for evt "ir"
        agent.listeners.append(self._on_message)

    # --- the link ---------------------------------------------------------------------------

    def _on_message(self, msg):
        if msg.get("src") != "agent" or msg.get("evt") != "ir":
            return
        state = msg.get("state")
        if state:
            self.learn = {k: v for k, v in msg.items() if k not in ("src", "evt")}
            self.agent.emit("ir", **self.learn)
        for pred, fut in list(self._waiters):
            if not fut.done() and pred(msg):
                fut.set_result(msg)

    async def ask(self, text, timeout=REPLY_S):
        """A protocol line over Bluetooth -> its reply dict (None: no answer / not connected)."""
        link = self.agent.link
        if not link.connected:
            return None
        pid = "ir" + uuid.uuid4().hex[:6]
        fut = asyncio.get_running_loop().create_future()

        def listen(m):
            if m.get("evt") == "proto" and m.get("id") == pid and not fut.done():
                fut.set_result(m.get("reply"))
        self.agent.listeners.append(listen)
        try:
            if not await link.send({"src": "agent", "proto": text, "id": pid}):
                return None
            return await asyncio.wait_for(fut, timeout)
        except TimeoutError:
            return None
        finally:
            self.agent.listeners.remove(listen)

    async def call(self, op, **args):
        """@ir <op> -> reply; raises IRError with the stick's words."""
        if not self.agent.link.connected:
            raise IRError("the stick isn't connected")
        r = await self.ask(line(op, **args))
        if r is None:
            raise IRError("the stick didn't answer")
        if not r.get("ok", False):
            if r.get("code") == "op" or "Unknown command" in str(r.get("error")):
                raise IRError(f"the stick's firmware has no IR remote: it needs {MIN_FIRMWARE}")
            raise IRError(r.get("error") or "the stick refused")
        return r

    # --- reading ----------------------------------------------------------------------------

    async def devices(self, with_buttons=True):
        r = await self.call("list")
        out = []
        for d in r.get("devices") or []:
            entry = {"id": d.get("id"), "n": d.get("n", 0), "fav": d.get("fav", 0)}
            if with_buttons:
                entry["buttons"] = await self.buttons(d["id"])
            out.append(entry)
        return {"devices": out, "max": r.get("max"), "free": r.get("free")}

    async def buttons(self, dev):
        out, page = [], 0
        while page < 10:
            r = await self.call("list", dev=dev, page=page)
            out += [{"name": b.get("n"), "proto": b.get("p"), "fav": bool(b.get("f"))} for b in r.get("buttons") or []]
            if not r.get("more"):
                break
            page += 1
        return out

    def brand_for(self, dev):
        return (self.agent.cfg.get("ir_brands") or {}).get(device_id(dev))

    # --- doing ------------------------------------------------------------------------------

    async def send(self, dev, btn, repeat=0):
        """Sends a learned button, or the device's brand code. -> a short line for the stick / Claude."""
        dev_id, name = device_id(dev), button_name(btn)
        if not dev_id or not name:
            raise IRError("say which device and which button")
        repeat = max(0, min(int(repeat or 0), 20))
        try:
            await self.call("send", dev=dev_id, btn=name, repeat=repeat)
            return f"{dev_id}: {name} sent"
        except IRError as e:
            if "No such" not in str(e):
                raise
        brand = self.brand_for(dev_id)
        if not brand:
            raise IRError(f"no '{name}' button on the '{dev_id}' remote (learn it, or pick a brand for "
                          f"'{dev_id}' in the dashboard's Remote page)")
        code = (BRANDS.get(brand) or {}).get("buttons", {}).get(name)
        if not code:
            raise IRError(f"the {BRANDS.get(brand, {}).get('title', brand)} code set has no '{name}' button")
        await self.call("send", code=code, repeat=repeat)
        return f"{dev_id}: {name} sent"

    async def send_code(self, code, repeat=0):
        if not valid_code(code):
            raise IRError("not a code the stick can send")
        await self.call("send", code=code, repeat=max(0, min(int(repeat or 0), 20)))
        return "sent"

    async def start_learn(self, dev, btn="", once=False):
        dev_id = device_id(dev) or "remote"
        name = button_name(btn) if btn else ""
        if btn and not name:
            raise IRError("button names: letters, digits, spaces, + - _ . #")
        r = await self.call("learn", dev=dev_id, btn=name, once=1 if once else None)
        self.learn = {"state": "learning", "dev": r.get("dev", dev_id), "btn": r.get("btn", name), "step": 1}
        self.agent.emit("ir", **self.learn)
        return self.learn

    async def learn_and_wait(self, dev, btn="", once=False, timeout=LEARN_STEP_S * 2 + 5):
        """Starts a learn and waits for its end: -> the final event (saved / timeout / error / cancelled)."""
        fut = asyncio.get_running_loop().create_future()
        pred = lambda m: m.get("state") in ("saved", "timeout", "error", "cancelled")
        self._waiters.append((pred, fut))
        try:
            await self.start_learn(dev, btn, once)
            return await asyncio.wait_for(fut, timeout)
        finally:
            self._waiters = [w for w in self._waiters if w[1] is not fut]

    async def cancel(self):
        return await self.call("cancel")

    async def rename(self, dev, to, btn=None):
        if btn:
            return await self.call("rename", dev=device_id(dev), btn=button_name(btn), to=to)
        return await self.call("rename", dev=device_id(dev), to=to)

    async def remove(self, dev, btn=None):
        return await self.call("rm", dev=device_id(dev), btn=button_name(btn) if btn else None)

    async def fav(self, dev, btn, on):
        return await self.call("fav", dev=device_id(dev), btn=button_name(btn), on=1 if on else 0)

    async def put(self, dev, btn, code, fav=False):
        if not valid_code(code):
            raise IRError(f"'{btn}': not a code the stick can send")
        return await self.call("put", dev=device_id(dev), btn=button_name(btn), code=code, fav=1 if fav else 0)

    async def install_brand(self, dev, brand):
        """Puts a brand's buttons on the stick as remote `dev`."""
        if brand not in BRANDS:
            raise IRError(f"unknown code set: {brand}")
        for name, code in BRANDS[brand]["buttons"].items():
            await self.put(dev, name, code, fav=name == "power")
        return {"dev": device_id(dev), "n": len(BRANDS[brand]["buttons"])}

    async def export(self):
        """Everything learned, codes included: {"stickybot_ir": 1, "devices": {id: [{n, c, f}]}}."""
        out = {}
        for d in (await self.devices(with_buttons=True))["devices"]:
            rows = []
            for b in d["buttons"]:
                r = await self.call("get", dev=d["id"], btn=b["name"])
                rows.append({"n": b["name"], "c": r.get("c"), **({"f": 1} if b["fav"] else {})})
            out[d["id"]] = rows
        return {"stickybot_ir": 1, "devices": out}

    async def import_doc(self, doc):
        """The export's format back onto the stick (buttons with the same name are replaced).
        -> {"added": n, "skipped": [why...]}"""
        devs = (doc or {}).get("devices") if isinstance(doc, dict) else None
        if not isinstance(devs, dict) or not devs:
            raise IRError('expected {"devices": {"tv": [{"n": "power", "c": "nec:4:8"}]}}')
        if len(devs) > MAX_DEVICES:
            raise IRError(f"at most {MAX_DEVICES} remotes")
        added, skipped = 0, []
        for dev, rows in devs.items():
            if not device_id(dev) or not isinstance(rows, list):
                skipped.append(f"{dev}: not a remote")
                continue
            for row in rows[:MAX_BUTTONS]:
                if not isinstance(row, dict):
                    continue
                name, code = button_name(row.get("n")), row.get("c")
                if not name or not valid_code(code):
                    skipped.append(f"{dev}/{row.get('n')}: bad name or code")
                    continue
                try:
                    await self.put(dev, name, code, fav=bool(row.get("f")))
                    added += 1
                except IRError as e:
                    skipped.append(f"{dev}/{name}: {e}")
        return {"added": added, "skipped": skipped}

    async def diag(self):
        return await self.call("diag")

    # --- voice and tools -------------------------------------------------------------------

    async def voice(self, spec):
        """A quick command's {"ir": {"dev", "btn", "repeat"}} -> (ok, line)."""
        spec = spec if isinstance(spec, dict) else {}
        try:
            return True, await self.send(spec.get("dev", ""), spec.get("btn", ""), spec.get("repeat", 0))
        except IRError as e:
            return False, str(e)[:1].upper() + str(e)[1:]

    async def listing(self):
        """For ir_list(): the remotes and their buttons (learned on the stick, plus brand sets mapped to devices)."""
        have = (await self.devices(with_buttons=True))["devices"]
        out = {d["id"]: [b["name"] for b in d["buttons"]] for d in have}
        for dev, brand in (self.agent.cfg.get("ir_brands") or {}).items():
            if brand in BRANDS:
                names = out.setdefault(dev, [])
                names += [n for n in BRANDS[brand]["buttons"] if n not in names]
        return out
