"""The stick tools: an MCP server inside the agent, so any Claude session can page you on the stick.

    POST http://127.0.0.1:8660/mcp      (MCP "Streamable HTTP", JSON-RPC 2.0, one message per POST)

Who uses it: the agent's own voice session (brain.py passes `--mcp-config`) and, once you run
`stickybot mcp install`, every Claude Code session in a terminal (`claude mcp add --scope user`).
In Claude Code the tools are called mcp__stick__<tool>.

Guarded like the rest of the local API (server.py): Host must be localhost / 127.0.0.1, an Origin
header (browsers send one) must be the dashboard's own, and every request needs the token in
~/.stick-agent/token as `Authorization: Bearer <token>` (or X-Stick-Token). Off in the dashboard
(config "mcp_server": false) -> 503 for everything.

Protocol, checked against the spec at modelcontextprotocol.io (Nov 2026):
- Dual era (https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning ,
  "Backward Compatibility with Initialization-Based Versions"):
  * legacy clients (2025-03-26 .. 2025-11-25, Claude Code 2.1.x): `initialize` -> we answer with their
    version (or our newest legacy one), `notifications/initialized` -> 202, then tools/list, tools/call,
    ping. We don't mint Mcp-Session-Id (optional in those revisions), so nothing breaks when the agent
    restarts. https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
  * modern clients (2026-07-28): no handshake; every request carries
    params._meta["io.modelcontextprotocol/protocolVersion"], mirrored in the MCP-Protocol-Version,
    Mcp-Method and (tools/call) Mcp-Name headers, which must match the body (else 400, -32020
    HeaderMismatch); unknown version -> 400, -32022 with our supported list; `server/discover`;
    results carry "resultType": "complete"; unknown method -> 404, -32601.
    https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
    Claude Code 2.1.317 speaks this one (seen on the wire: server/discover, tools/list, tools/call
    with MCP-Protocol-Version 2026-07-28, Mcp-Method, Mcp-Name; it expands ${PWD} in headers).
- GET / DELETE -> 405 (no standalone SSE stream, no sessions). A notification -> 202, no body.
- Replies are application/json, except ask(), which can wait a minute: if the client accepts
  text/event-stream it gets an SSE stream with keep-alive comments and the response as one event;
  the client closing that stream cancels the question (clears it on the stick).
- Bad arguments are tool errors (isError: true) so Claude can correct itself; an unknown tool is a
  JSON-RPC error (-32602). https://modelcontextprotocol.io/specification/2026-07-28/server/tools

Tools that change the Mac (open_url, open_app, quick_command) are deliberately not pre-allowed
anywhere: Claude Code asks for permission as for any tool, which our PermissionRequest hook puts
on the stick (A / B). Brains without Claude Code (OpenRouter, openai_brain.py) run the same tools
through StickTools.call_approved(), which shows that same prompt first. Everything fails closed
("stick not connected") without the stick, except stick_status. Alerts (notify, celebrate, speak) are
rate-limited so a loop can't spam the stick.
"""
import asyncio
import base64
import json
import logging
import os
import re
import subprocess
import time
import urllib.parse

from aiohttp import web

from . import commands, config, events
from .core import stick_text

log = logging.getLogger("mcp")

SERVER_NAME = "stick"
SERVER_VERSION = "1.0.0"
MODERN = ("2026-07-28",)
LEGACY = ("2025-11-25", "2025-06-18", "2025-03-26")
SUPPORTED = MODERN + LEGACY
META = "io.modelcontextprotocol/"

# JSON-RPC / MCP error codes
PARSE_ERROR, INVALID_REQUEST, METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL = -32700, -32600, -32601, -32602, -32603
HEADER_MISMATCH, UNSUPPORTED_VERSION = -32020, -32022

ALERT_GAP_S = 1.0       # notify / celebrate / speak: at least this far apart (all sessions together)
ALERT_PER_MIN = 20      # ... and at most this many a minute
CARD_PUSH_GAP_S = 0.5   # progress / pin_home: the card goes to the stick at most this often (the rest
                        # rides on the next hello, <= 8 s later)
SSE_KEEPALIVE_S = 15

LEVELS = {"info": "nice", "nice": "nice", "celebrate": "celebrate", "heads_up": "headsup"}
NOT_CONNECTED = "stick not connected"

INSTRUCTIONS = (
    "Tools for the user's M5StickS3, a small device on their desk with a 135x240 screen, two buttons "
    "(A, B) and a speaker. Use it like a pager: notify() when a long task finishes or needs them, "
    "ask() for a quick A/B decision instead of waiting in the terminal, progress() for long jobs, "
    "celebrate() for real wins. Keep texts short (a few words). Don't notify for every small step. "
    "stick_status() tells whether they're at their desk.")


def _text(maxlen, desc):
    return {"type": "string", "maxLength": maxlen, "description": desc}


TOOLS = [
    {"name": "notify", "title": "Notify on the stick",
     "description": ("Show a short message on the user's stick (a toast, with a sound). Use it when a long "
                     "task finishes, fails, or needs the user's attention — e.g. 'Tests passed', 'Build "
                     "failed: 3 errors'. level: info (quiet toast), nice (toast + chime), celebrate "
                     "(full-screen confetti), heads_up (orange, stays until a button press). "
                     "At most one per second. Returns 'shown' or why not."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["text"], "properties": {
         "text": _text(130, "What to say, a few words (the first ~40 characters are the title)."),
         "level": {"type": "string", "enum": list(LEVELS), "default": "info"},
         "sound": {"type": "boolean", "description": "Play a sound (default: info no, the others yes)."}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "idempotentHint": False,
                     "openWorldHint": False}},
    {"name": "ask", "title": "Ask on the stick",
     "description": ("Ask the user a quick question on the stick and wait for the answer: button A picks the "
                     "first option, B the second (default options: Yes / No). Shaking the stick dismisses "
                     "it. Returns the chosen option's label, or 'timeout', 'dismissed', 'busy: …' (another "
                     "prompt or a voice request is in progress — try again later) or 'stick not connected'. "
                     "Use for small decisions; keep the question under ~100 characters."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["question"], "properties": {
         "question": _text(160, "The question, short."),
         "options": {"type": "array", "items": {"type": "string", "maxLength": 16}, "minItems": 1,
                     "maxItems": 2, "description": "1-2 short labels (<= 16 characters): A, then B."},
         "timeout_s": {"type": "integer", "minimum": 5, "maximum": 120, "default": 60}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False}},
    {"name": "celebrate", "title": "Celebrate on the stick",
     "description": "Full-screen confetti and a fanfare on the stick, for real wins (a release, a green CI "
                    "after a long fight). At most one per second.",
     "inputSchema": {"type": "object", "additionalProperties": False, "properties": {
         "text": _text(130, "What happened (default 'Done!').")}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False}},
    {"name": "progress", "title": "Progress on the stick",
     "description": ("Show progress of a long job on the stick's home screen card, e.g. text='Running tests', "
                     "done=40, total=120. Call again to update (replaces the line); it disappears 10 minutes "
                     "after the last update, 2 minutes after done == total. Silent — no sound."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["text"], "properties": {
         "text": _text(140, "What's happening, short."),
         "done": {"type": "integer", "minimum": 0},
         "total": {"type": "integer", "minimum": 1}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "idempotentHint": True,
                     "openWorldHint": False}},
    {"name": "pin_home", "title": "Pin a line on the stick",
     "description": ("Pin a line on the stick's home screen card (instead of the facts/news rotation) for "
                     "`minutes` (default 30), e.g. 'PR #123 waiting for review'. A new pin replaces the old "
                     "one; empty text removes it."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["text"], "properties": {
         "text": _text(140, "The line to show ('' removes the pin)."),
         "minutes": {"type": "integer", "minimum": 1, "maximum": 480, "default": 30}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "idempotentHint": True,
                     "openWorldHint": False}},
    {"name": "open_url", "title": "Open a URL on the Mac",
     "description": "Open an http(s) URL in the user's browser on their Mac. Needs the user's approval.",
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["url"], "properties": {
         "url": {"type": "string", "maxLength": 2000, "description": "http:// or https:// only."}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": True}},
    {"name": "open_app", "title": "Open an app on the Mac",
     "description": ("Open (or bring to the front) an installed Mac app by name, e.g. 'Slack', 'VS Code'. "
                     "Needs the user's approval."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["name"], "properties": {
         "name": {"type": "string", "maxLength": 60, "description": "The app's name."}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False}},
    {"name": "quick_command", "title": "Run a quick command",
     "description": ("Run one of the user's quick commands (the phrases they set up for the stick, e.g. "
                     "'PR 1234', 'open slack'). Needs the user's approval. Returns the command's result, or "
                     "'no quick command matches'."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["phrase"], "properties": {
         "phrase": {"type": "string", "maxLength": 120, "description": "The phrase, as the user would say it."}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": True}},
    {"name": "stick_status", "title": "Stick status",
     "description": ("The stick's state: connected, battery, on USB, what its screen shows, when a button was "
                     "last pressed, and a hint whether the user is at their desk. Read-only."),
     "inputSchema": {"type": "object", "additionalProperties": False, "properties": {}},
     "annotations": {"readOnlyHint": True, "openWorldHint": False}},
    {"name": "speak", "title": "Speak on the stick",
     "description": ("Say a short sentence out loud (the stick's speaker or the Mac's, as the user set it). "
                     "Use sparingly — only when a spoken word is worth it. Silent in quiet hours. At most "
                     "one per second."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["text"], "properties": {
         "text": _text(300, "What to say, one or two short sentences.")}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False}},
    {"name": "ir_list", "title": "List the IR remotes",
     "description": ("The infrared remotes the stick can play (TV, AC, soundbar...) and their buttons, as "
                     "{device: [button, ...]}: buttons learned on the stick plus built-in TV code sets. Read-only. "
                     "Call it before ir_send to pick the button whose name fits the request (e.g. 'set the AC to "
                     "24' -> device 'ac', button 'cool 24')."),
     "inputSchema": {"type": "object", "additionalProperties": False, "properties": {}},
     "annotations": {"readOnlyHint": True, "openWorldHint": False}},
    {"name": "ir_send", "title": "Press a remote button",
     "description": ("Send an infrared remote button from the stick (it must point roughly at the device), "
                     "e.g. device='tv', button='power'. Names as ir_list() gives them. repeat: extra presses "
                     "(volume steps). Returns 'tv: power sent' or why not."),
     "inputSchema": {"type": "object", "additionalProperties": False, "required": ["device", "button"],
                     "properties": {
                         "device": {"type": "string", "maxLength": 32, "description": "The remote, e.g. 'tv'."},
                         "button": {"type": "string", "maxLength": 32, "description": "The button, e.g. 'volume up'."},
                         "repeat": {"type": "integer", "minimum": 0, "maximum": 10, "default": 0}}},
     "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": True}},
]
BY_NAME = {t["name"]: t for t in TOOLS}
FEEDBACK_TOOLS = ("notify", "ask", "celebrate", "progress", "pin_home", "stick_status", "speak",
                  "ir_list", "ir_send")  # pre-allowed (ir_send: the user asked for it by voice)
MAC_TOOLS = ("open_url", "open_app", "quick_command")  # change the Mac: Claude Code asks first


def allowed_tool_names():
    """The read-only / feedback tools, as Claude Code names them (for --allowedTools)."""
    return [f"mcp__{SERVER_NAME}__{t}" for t in FEEDBACK_TOOLS]


def openai_tools():
    """The same tools as OpenAI-style function definitions (the OpenRouter / Ollama brains)."""
    return [{"type": "function", "function": {"name": t["name"], "description": t["description"],
                                              "parameters": t["inputSchema"]}} for t in TOOLS]


def approval_hint(name, args):
    """What the approval prompt on the stick shows for a Mac tool (like hook.py's describe())."""
    args = args if isinstance(args, dict) else {}
    key = {"open_url": "url", "open_app": "name", "quick_command": "phrase"}.get(name)
    return str(args.get(key, "")) if key else summarize(args)


class ToolError(Exception):
    """A tool can't do it: the text goes back to Claude with isError (it can correct itself)."""


def check_args(schema, args):
    """A small JSON Schema check for our own schemas (object, string/integer/boolean/array of strings,
    enum, min/max, minItems/maxItems, required, no extra keys). Too-long strings are cut, not refused.
    Returns the cleaned arguments with defaults filled in; raises ToolError."""
    if args is None:
        args = {}
    if not isinstance(args, dict):
        raise ToolError("arguments must be an object")
    props = schema.get("properties", {})
    extra = [k for k in args if k not in props]
    if extra:
        raise ToolError(f"unknown argument(s): {', '.join(sorted(extra))} (expected: {', '.join(props) or 'none'})")
    out = {}
    for name in schema.get("required", []):
        if args.get(name) is None:
            raise ToolError(f"missing argument: {name}")
    for name, p in props.items():
        if args.get(name) is None:
            if "default" in p:
                out[name] = p["default"]
            continue
        v, t = args[name], p.get("type")
        if t == "string":
            if not isinstance(v, str):
                raise ToolError(f"{name} must be a string")
            v = v[:p["maxLength"]] if "maxLength" in p else v
        elif t == "integer":
            if isinstance(v, float) and v.is_integer():
                v = int(v)
            if isinstance(v, bool) or not isinstance(v, int):
                raise ToolError(f"{name} must be an integer")
            if "minimum" in p and v < p["minimum"] or "maximum" in p and v > p["maximum"]:
                raise ToolError(f"{name} must be between {p.get('minimum', '-inf')} and {p.get('maximum', 'inf')}")
        elif t == "boolean":
            if not isinstance(v, bool):
                raise ToolError(f"{name} must be true or false")
        elif t == "array":
            if not isinstance(v, list) or not all(isinstance(x, str) for x in v):
                raise ToolError(f"{name} must be a list of strings")
            if len(v) < p.get("minItems", 0) or len(v) > p.get("maxItems", len(v)):
                raise ToolError(f"{name} takes {p.get('minItems', 0)}-{p.get('maxItems')} items")
            cap = p.get("items", {}).get("maxLength")
            v = [x[:cap] if cap else x for x in v]
        if "enum" in p and v not in p["enum"]:
            raise ToolError(f"{name} must be one of: {', '.join(p['enum'])}")
        out[name] = v
    return out


class RateLimit:
    """At least `gap` seconds between, and at most `per_min` a minute."""

    def __init__(self, gap, per_min):
        self.gap, self.per_min = gap, per_min
        self.times = []

    def take(self, now=None):
        """'' if allowed now (and counts it), else why not."""
        now = time.monotonic() if now is None else now
        self.times = [t for t in self.times if now - t < 60]
        if self.times and now - self.times[-1] < self.gap:
            return f"rate limited: wait {self.gap - (now - self.times[-1]):.1f} s (at most one alert a second)"
        if len(self.times) >= self.per_min:
            return f"rate limited: {self.per_min} alerts in the last minute — wait a bit"
        self.times.append(now)
        return ""


def split_title(text, width=40):
    """'Tests passed: 120 of 120, 3 skipped' -> a title that fits the stick (40) and the rest."""
    text = " ".join(text.split())
    if len(text) <= width:
        return text, ""
    cut = text.rfind(" ", 0, width)
    cut = cut if cut > width // 2 else width
    return text[:cut].rstrip(" ,;:-"), text[cut:].strip()


def caller(request):
    """Who's calling, for the stick ("web asks") and the activity log: the voice session sends
    X-Stick-Client: voice; terminal installs send X-Stick-Cwd: ${PWD} (expanded by Claude Code)."""
    if request.headers.get("X-Stick-Client") == "voice":
        return "Agent"
    cwd = request.headers.get("X-Stick-Cwd", "")
    if cwd and "${" not in cwd:
        name = os.path.basename(urllib.parse.unquote(cwd).rstrip("/"))
        if name:
            return stick_text(name)[:24]
    return "Claude Code"


def summarize(args, n=60):
    bits = []
    for k, v in (args or {}).items():
        s = json.dumps(v) if not isinstance(v, str) else repr(v)
        bits.append(f"{k}={s[:n] + '…' if len(s) > n else s}")
    return ", ".join(bits)


class StickTools:
    """The tools themselves, on top of the Agent (core.py)."""

    def __init__(self, agent):
        self.agent = agent
        self.alerts = RateLimit(ALERT_GAP_S, ALERT_PER_MIN)
        self._card_at = 0.0
        self._card_later = None
        self.calls = 0
        self.last_call = None  # {"t", "tool", "from"} for the dashboard

    async def call(self, name, args, who):
        """-> (text, is_error, structured or None)."""
        tool = BY_NAME[name]
        started = time.monotonic()
        try:
            clean = check_args(tool["inputSchema"], args)
            if name != "stick_status" and not self.agent.link.connected:
                raise ToolError(NOT_CONNECTED)
            out = await getattr(self, f"t_{name}")(clean, who)
            text, structured = out if isinstance(out, tuple) else (out, None)
            err = False
        except ToolError as e:
            text, structured, err = str(e), None, True
        except Exception as e:  # noqa: BLE001 — a bug in a tool shouldn't take the server down
            log.exception("tool %s failed", name)
            text, structured, err = f"failed: {e}", None, True
        ms = int((time.monotonic() - started) * 1000)
        self.calls += 1
        self.last_call = {"t": time.time(), "tool": name, "from": who}
        log.info("%s: %s(%s) -> %s%s (%d ms)", who, name, summarize(args, 40), "error: " if err else "", text[:80], ms)
        self.agent.emit("mcp", tool=name, origin=who, args=summarize(args), result=text[:200], ok=not err, ms=ms)
        return text, err, structured

    async def call_approved(self, name, args, who):
        """call() for a brain without Claude Code's permission prompt (OpenRouter, Ollama): the Mac
        tools ask on the stick first (A / B), the same prompt Claude Code's hook shows."""
        if name in MAC_TOOLS and name in BY_NAME:
            hint = approval_hint(name, args)
            decision = await self.agent.ask_permission(name, hint[:300], who)
            if decision != "allow":
                why = ("needs the user's approval, but stick approvals are turned off (dashboard → Settings)"
                       if decision is None and not self.agent.cfg.get("stick_approvals", True)
                       else "the user didn't approve it" if decision == "deny" else "no answer on the stick")
                log.info("%s: %s(%s) not run: %s", who, name, summarize(args, 40), why)
                self.agent.emit("mcp", tool=name, origin=who, args=summarize(args), result=why, ok=False, ms=0)
                return why, True, None
        return await self.call(name, args, who)

    # --- feedback -----------------------------------------------------------------------

    async def _event(self, level, text, who, sound):
        cfg = self.agent.cfg
        if events.rules(cfg).get("claude_tool") == "off":
            raise ToolError("the user turned notifications from Claude sessions off (dashboard → Events)")
        why = self.alerts.take()
        if why:
            raise ToolError(why)
        title, rest = split_title(stick_text(text))
        sub = rest or f"from {who}"
        ev = await self.agent.notifier.notify("claude_tool", title, sub, icon="claude", level=level,
                                              sound="" if sound else "off")
        if not ev:
            raise ToolError(NOT_CONNECTED)
        return "shown" + (" (quiet hours: no sound)" if ev.get("sound") == "off" and sound else "")

    async def t_notify(self, a, who):
        level = a.get("level") or "info"
        sound = a.get("sound")
        if sound is None:
            sound = level != "info"
        return await self._event(LEVELS[level], a["text"].strip() or "Ping", who, sound)

    async def t_celebrate(self, a, who):
        return await self._event("celebrate", (a.get("text") or "").strip() or "Done!", who, True)

    async def t_ask(self, a, who):
        question = " ".join(a["question"].split())
        if not question:
            raise ToolError("the question is empty")
        options = [" ".join(o.split()) for o in (a.get("options") or ["Yes", "No"])]
        if not all(options):
            raise ToolError("options can't be empty")
        if len(options) == 2 and options[0].lower() == options[1].lower():
            raise ToolError("the two options must differ")
        answer = await self.agent.ask_user(question, options, a.get("timeout_s") or 60, who)
        if answer == NOT_CONNECTED:
            raise ToolError(answer)
        return answer, {"answer": answer}

    async def _card(self, slot, line_a, line_b, secs):
        self.agent.home.put(slot, line_a, line_b, secs)
        now = time.monotonic()
        if now - self._card_at >= CARD_PUSH_GAP_S:
            self._card_at = now
            await self.agent.push_home()
        elif self._card_later is None:  # coalesce: one push a little later with the newest line
            async def later():
                await asyncio.sleep(CARD_PUSH_GAP_S)
                self._card_later = None
                self._card_at = time.monotonic()
                await self.agent.push_home()
            self._card_later = asyncio.ensure_future(later())

    async def t_progress(self, a, who):
        from . import home
        done, total = a.get("done"), a.get("total")
        if done is not None and total is not None and done > total:
            raise ToolError("done can't be more than total")
        if total is not None and done is not None:
            pct = int(done * 100 / total)
            head = f"{who}: {done}/{total} ({pct}%)"
        elif done is not None:
            head = f"{who}: {done} done"
        else:
            head = f"{who}: working"
        finished = done is not None and total is not None and done >= total
        await self._card("progress", head, stick_text(a["text"]).strip(),
                         home.PROGRESS_DONE_S if finished else home.PROGRESS_S)
        return "shown on the home screen"

    async def t_pin_home(self, a, who):
        text = stick_text(a["text"]).strip()
        if not text:
            had = self.agent.home.clear("pin")
            await self.agent.push_home()
            return "pin removed" if had else "nothing was pinned"
        mins = a.get("minutes") or 30
        await self._card("pin", f"Pinned · {who}", text, mins * 60)
        return f"pinned for {mins} min"

    async def t_ir_list(self, a, who):
        from .ir import IRError
        try:
            devices = await self.agent.ir.listing()
        except IRError as e:
            raise ToolError(str(e)) from None
        if not devices:
            return "no remotes yet: learn one on the stick (menu: Remote) or in the dashboard", {"devices": {}}
        text = "; ".join(f"{d}: {', '.join(b) or '(no buttons)'}" for d, b in devices.items())
        return text, {"devices": devices}

    async def t_ir_send(self, a, who):
        from .ir import IRError
        try:
            return await self.agent.ir.send(a["device"], a["button"], a.get("repeat") or 0)
        except IRError as e:
            raise ToolError(str(e)) from None

    async def t_speak(self, a, who):
        text = " ".join(a["text"].split())
        if not text:
            raise ToolError("nothing to say")
        if events.quiet_now(self.agent.cfg):
            return "quiet hours: not spoken"
        why = self.alerts.take()
        if why:
            raise ToolError(why)
        await self.agent.speak(text, force=True)
        return f"speaking on the {self.agent._speech_output()}"

    async def t_stick_status(self, a, who):
        ag = self.agent
        connected = bool(ag.link.connected)
        info = {"connected": connected}
        if connected:
            rep = await ag.stick_report() or {}
            bat = rep.get("bat") or {}
            info.update({"battery_pct": bat.get("pct"), "usb_power": bat.get("usb"),
                         "plugged_into_computer": ag.stick_usb,
                         "uptime_s": (rep.get("sys") or {}).get("up")})
        ago = int(time.time() - ag.last_input) if ag.last_input else None
        info["last_button_press_s_ago"] = ago
        if ag.prompts:
            screen = "an approval prompt"
        elif ag.asks:
            screen = "a question"
        elif ag.state in ("listening", "transcribing", "thinking", "working"):
            screen = f"the agent ({ag.state})"
        else:
            screen = "the home screen (or asleep)"
        info["screen"] = screen if connected else None
        if not connected:
            desk = "unknown (the stick isn't connected — it may be off, asleep or out of range)"
        elif ag.stick_usb or (ago is not None and ago < 300):
            desk = "likely"
        elif ago is None or ago > 1800:
            desk = "maybe not (no button pressed for a while)"
        else:
            desk = "unknown"
        info["at_desk"] = desk
        if not connected:
            return "stick not connected (it may be off, asleep or out of range)", info
        parts = ["connected"]
        if info.get("battery_pct") is not None:
            power = " (charging / on USB power)" if info["usb_power"] else ""
            parts.append(f"battery {info['battery_pct']}%{power}")
        if ag.stick_usb:
            parts.append("plugged into a computer")
        parts.append(f"screen: {screen}")
        parts.append("last button press: " + (f"{_ago(ago)} ago" if ago is not None else "not since the agent started"))
        parts.append(f"at the desk: {desk}")
        return " · ".join(parts), info

    # --- the Mac (Claude Code asks the user before these) -------------------------------------

    async def t_open_url(self, a, who):
        url = a["url"].strip()
        parts = urllib.parse.urlsplit(url)
        if parts.scheme not in ("http", "https") or not parts.netloc or re.search(r"[\x00-\x20\x7f]", url):
            raise ToolError("only http:// or https:// URLs (no spaces or control characters)")
        await _run(["open", url])
        return f"opened {parts.netloc}"

    async def t_open_app(self, a, who):
        name = a["name"].strip()
        if not name or not re.fullmatch(r"[\w .&+'()-]+", name):
            raise ToolError("not an app name")
        app = await asyncio.get_running_loop().run_in_executor(None, commands.find_app, name)
        if not app:
            raise ToolError(f"no app called {name!r} in /Applications")
        await _run(["open", "-a", app])
        return f"opened {app}"

    async def t_quick_command(self, a, who):
        phrase = " ".join(a["phrase"].split())
        hit = commands.match(phrase)
        if not hit:
            raise ToolError("no quick command matches that phrase")
        cmd, slots = hit
        result = await asyncio.get_running_loop().run_in_executor(
            None, lambda: commands.run(cmd, slots, quote_shell=True))
        if result is None:
            raise ToolError(f"{cmd.get('name', 'that command')} doesn't apply here")
        return stick_text(result)[:200]


def _ago(s):
    return f"{s} s" if s < 90 else f"{s // 60} min" if s < 5400 else f"{s // 3600} h"


async def _run(args):
    """A list of arguments, never a shell: nothing in them is interpreted."""
    def go():
        subprocess.run(args, check=False, capture_output=True, timeout=15)
    await asyncio.get_running_loop().run_in_executor(None, go)


# --- the JSON-RPC endpoint ---------------------------------------------------------------------

class RpcError(Exception):
    def __init__(self, code, message, status=200, data=None):
        super().__init__(message)
        self.code, self.message, self.status, self.data = code, message, status, data


def _rpc_error(id_, code, message, status, data=None):
    err = {"code": code, "message": message}
    if data is not None:
        err["data"] = data
    body = {"jsonrpc": "2.0", "error": err}
    if id_ is not None:
        body["id"] = id_
    return web.json_response(body, status=status)


def _decode_header(v):
    """Mcp-Name may be "=?base64?...?=" (non-ASCII names)."""
    if v and v.startswith("=?base64?") and v.endswith("?="):
        try:
            return base64.b64decode(v[9:-2]).decode()
        except (ValueError, UnicodeDecodeError):
            return None
    return v


def ok_origin(request):
    origin = request.headers.get("Origin")
    return origin is None or origin in (f"http://localhost:{config.PORT}", f"http://127.0.0.1:{config.PORT}")


def routes(agent):
    tools = getattr(agent, "mcp_tools", None) or StickTools(agent)  # (an OpenRouter brain may have made it)
    agent.mcp_tools = tools
    inflight = {}  # (caller, request id) -> task (legacy clients cancel with notifications/cancelled)

    async def get_or_delete(_):
        return web.Response(status=405, headers={"Allow": "POST"})

    async def post(request):
        if not ok_origin(request):
            return _rpc_error(None, INVALID_REQUEST, "bad origin", 403)
        if not agent.cfg.get("mcp_server", True):
            return _rpc_error(None, INTERNAL, "the stick tools are turned off in the StickyBot dashboard", 503)
        try:
            msg = await request.json()
        except ValueError:
            return _rpc_error(None, PARSE_ERROR, "parse error", 400)
        if isinstance(msg, list):
            return _rpc_error(None, INVALID_REQUEST, "batches aren't supported", 400)
        if not isinstance(msg, dict) or msg.get("jsonrpc") != "2.0":
            return _rpc_error(None, INVALID_REQUEST, "not a JSON-RPC 2.0 message", 400)
        if "method" not in msg:
            if "result" in msg or "error" in msg:  # a response from a legacy client: nothing asked, ignore
                return web.Response(status=202)
            return _rpc_error(msg.get("id"), INVALID_REQUEST, "no method", 400)
        method, id_ = msg["method"], msg.get("id")
        params = msg.get("params") or {}
        if not isinstance(method, str) or not isinstance(params, dict):
            return _rpc_error(id_, INVALID_REQUEST, "bad method or params", 400)
        who = caller(request)
        if "id" not in msg:  # a notification
            if method == "notifications/cancelled":
                task = inflight.get((who, str(params.get("requestId"))))
                if task:
                    task.cancel()
            return web.Response(status=202)
        try:
            modern = _era(request, method, params)
        except RpcError as e:
            return _rpc_error(id_, e.code, e.message, e.status, e.data)

        def reply(result):
            if modern:
                result = {"resultType": "complete", **result}
            return {"jsonrpc": "2.0", "id": id_, "result": result}

        try:
            if method == "initialize":
                result = _initialize(params)
            elif method == "server/discover":
                result = {"supportedVersions": list(SUPPORTED), "capabilities": {"tools": {"listChanged": False}},
                          "_meta": {META + "serverInfo": _server_info()}, "instructions": INSTRUCTIONS}
            elif method == "ping":
                result = {}
            elif method == "tools/list":
                result = {"tools": [dict(t) for t in TOOLS]}
            elif method == "tools/call":
                name = params.get("name")
                if name not in BY_NAME:
                    raise RpcError(INVALID_PARAMS, f"Unknown tool: {name}")
                if name == "ask" and "text/event-stream" in request.headers.get("Accept", ""):
                    return await _streamed(request, tools, params, who, reply, inflight, id_)
                task = asyncio.ensure_future(tools.call(name, params.get("arguments"), who))
                inflight[(who, str(id_))] = task
                try:
                    text, err, structured = await task
                except asyncio.CancelledError:
                    raise RpcError(INTERNAL, "cancelled") from None
                finally:
                    inflight.pop((who, str(id_)), None)
                result = _tool_result(text, err, structured)
            else:
                raise RpcError(METHOD_NOT_FOUND, f"Method not found: {method}", status=404 if modern else 200)
        except RpcError as e:
            return _rpc_error(id_, e.code, e.message, e.status, e.data)
        return web.json_response(reply(result))

    return [web.post("/mcp", post), web.get("/mcp", get_or_delete), web.delete("/mcp", get_or_delete)]


def _era(request, method, params):
    """True for a modern (2026-07-28) request, False for a legacy one; RpcError if it's malformed."""
    meta = params.get("_meta") if isinstance(params.get("_meta"), dict) else {}
    version = meta.get(META + "protocolVersion")
    header = request.headers.get("MCP-Protocol-Version")
    if version is None:  # legacy (initialize-based) — the header, if any, is the negotiated version
        if header in MODERN:
            raise RpcError(HEADER_MISMATCH, f"MCP-Protocol-Version {header} but no _meta protocol version", 400)
        if header is not None and header not in LEGACY:
            raise RpcError(UNSUPPORTED_VERSION, "Unsupported protocol version", 400,
                           {"supported": list(SUPPORTED), "requested": header})
        return False
    if version not in MODERN:  # (the legacy versions negotiate with initialize, not _meta)
        raise RpcError(UNSUPPORTED_VERSION, "Unsupported protocol version", 400,
                       {"supported": list(SUPPORTED), "requested": version})
    if header != version:
        raise RpcError(HEADER_MISMATCH, f"Header mismatch: MCP-Protocol-Version {header!r} vs body {version!r}", 400)
    if request.headers.get("Mcp-Method") != method:
        raise RpcError(HEADER_MISMATCH, f"Header mismatch: Mcp-Method {request.headers.get('Mcp-Method')!r} "
                                        f"vs body {method!r}", 400)
    if method in ("tools/call", "resources/read", "prompts/get"):
        want = params.get("name") if method != "resources/read" else params.get("uri")
        got = _decode_header(request.headers.get("Mcp-Name"))
        if got != want:
            raise RpcError(HEADER_MISMATCH, f"Header mismatch: Mcp-Name {got!r} vs body {want!r}", 400)
    return True


def _server_info():
    return {"name": SERVER_NAME, "title": "stick (M5StickS3 pager)", "version": SERVER_VERSION}


def _initialize(params):
    asked = params.get("protocolVersion")
    version = asked if asked in LEGACY else LEGACY[0]
    return {"protocolVersion": version, "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": _server_info(), "instructions": INSTRUCTIONS}


def _tool_result(text, err, structured):
    out = {"content": [{"type": "text", "text": text}], "isError": bool(err)}
    if structured is not None and not err:
        out["structuredContent"] = structured
    return out


async def _streamed(request, tools, params, who, reply, inflight, id_):
    """ask() as an SSE stream: keep-alives while the user decides; the client closing it cancels."""
    resp = web.StreamResponse(headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache",
                                       "X-Accel-Buffering": "no"})
    await resp.prepare(request)
    task = asyncio.ensure_future(tools.call("ask", params.get("arguments"), who))
    inflight[(who, str(id_))] = task
    try:
        while True:
            done, _ = await asyncio.wait([task], timeout=SSE_KEEPALIVE_S)
            if done:
                break
            await resp.write(b": keep-alive\n\n")
        text, err, structured = task.result()
        body = json.dumps(reply(_tool_result(text, err, structured)))
        await resp.write(f"event: message\ndata: {body}\n\n".encode())
    except (ConnectionResetError, asyncio.CancelledError):
        task.cancel()  # the client gave up: ask_user() clears the question on the stick
    finally:
        inflight.pop((who, str(id_)), None)
    return resp
