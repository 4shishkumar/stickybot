"""Hands a request to Claude Code (headless, the user's own login).

Safe tools (web search, reading) run without asking; anything else triggers
Claude Code's permission prompt, which our hook answers from the stick or the
dashboard. Requests within a few minutes continue the same conversation.

Two ways to run it (config "brain_mode"):
- "session" (default): one long-lived `claude -p --input-format stream-json` process;
  each request is a user message on its stdin, so there's no start-up per request.
- "oneshot": `claude -p "<text>"` per request (the original way), `--resume` for follow-ups.

The stream-json protocol, as used here (checked against Claude Code 2.1.317):
- Input (stdin, one JSON object per line), the message shape the Agent SDK writes
  (claude-agent-sdk-python, client.py `query()`):
    {"type": "user", "message": {"role": "user", "content": "<text>"},
     "parent_tool_use_id": null, "session_id": "<id or empty>"}
- Cancel: a control request, as the SDK's `interrupt()` sends it (_internal/query.py):
    {"type": "control_request", "request_id": "req_<n>_<hex>", "request": {"subtype": "interrupt"}}
  The CLI answers {"type": "control_response", "response": {"subtype": "success", ...}} and ends
  the turn with a `result` event (subtype "error_during_execution", is_error true, result null).
- Output (stdout): `system` events (init once per turn, hooks...), `assistant` messages (one per
  content block; tool_use blocks become the "Working: ..." lines), and with
  --include-partial-messages `stream_event`s wrapping the raw API stream: message_start,
  content_block_start (text | tool_use), content_block_delta (delta.type "text_delta" -> delta.text),
  content_block_stop, message_delta, message_stop. Every turn ends with one `result` event.
  Docs: https://code.claude.com/docs/en/headless ("Stream responses"),
  https://code.claude.com/docs/en/agent-sdk/streaming-output ("StreamEvent reference", "Message flow").
"""
import asyncio
import collections
import json
import logging
import os
import re
import time

from . import config, mcp_setup

log = logging.getLogger("claude")


def mcp_server_tools():
    """mcp__stick__notify, ... (mcp_server imports core, which imports us: import it late)."""
    from .mcp_server import allowed_tool_names
    return allowed_tool_names()
TIMEOUT_S = 240
INTERRUPT_WAIT_S = 3   # after an interrupt, the turn should end at once; else the process is killed

# "new chat", "start over", ... on its own, or before the request ("Start over. What's ...?")
NEW_CHAT = re.compile(
    r"^\s*(?:new (?:chat|conversation)|(?:let'?s )?start (?:over|again|a new (?:chat|conversation))"
    r"|forget (?:that|everything)|reset (?:the )?(?:chat|conversation))(?: please)?\s*"
    r"(?:[.,!?;:]+\s*(?P<rest>.*))?$", re.I | re.S)


def split_new_chat(text):
    """(True, rest) if the request starts with a "new chat" phrase, else (False, text)."""
    m = NEW_CHAT.match(text or "")
    if not m:
        return False, text
    return True, (m.group("rest") or "").strip()


def describe_tool(name, inp):
    if name == "Bash":
        return "Running: " + inp.get("command", "")[:120]
    if name in ("WebSearch",):
        return "Searching: " + inp.get("query", "")[:100]
    if name == "WebFetch":
        return "Reading " + re.sub(r"^https?://", "", inp.get("url", ""))[:80]
    for k in ("file_path", "path", "pattern"):
        if k in inp:
            return f"{name}: {str(inp[k])[:90]}"
    return name


def plain(text):
    """Makes Claude's reply fit the stick's screen: no markdown, short."""
    t = re.sub(r"```.*?```", "", text or "", flags=re.S)
    t = t.split("```", 1)[0]  # a code block still being written (a streamed reply)
    t = re.sub(r"[*_`#>]+", "", t)
    t = re.sub(r"\s+", " ", t).strip()
    return t if len(t) <= 280 else t[:277].rsplit(" ", 1)[0] + "…"


class Turn:
    """One request's stream-json events -> progress lines, the answer as it's written, the result.

    on_text(text) gets the text of the current assistant message so far. Text that turns out to be
    a preamble before a tool call ("Let me search...") is withdrawn with on_text("")."""

    def __init__(self, on_progress, on_text):
        self.on_progress, self.on_text = on_progress, on_text
        self.started = time.monotonic()
        self.first_token = None   # seconds after start
        self.events = 0
        self.text = ""            # the current assistant message's text
        self.result = None        # set by the "result" event
        self.ok = False
        self.session = None

    async def feed(self, ev):
        """Handles one event. Returns True once the turn is over."""
        self.events += 1
        kind = ev.get("type")
        if kind == "stream_event" and not ev.get("parent_tool_use_id"):
            e = ev.get("event") or {}
            if e.get("type") == "message_start":
                await self._set_text("")
            elif e.get("type") == "content_block_start" and (e.get("content_block") or {}).get("type") == "tool_use":
                await self._set_text("")  # what it said so far was a preamble, not the answer
            elif e.get("type") == "content_block_delta" and (e.get("delta") or {}).get("type") == "text_delta":
                if self.first_token is None:
                    self.first_token = time.monotonic() - self.started
                await self._set_text(self.text + (e["delta"].get("text") or ""))
        elif kind == "assistant":
            for part in ev.get("message", {}).get("content", []):
                if part.get("type") == "tool_use" and self.on_progress:
                    await self.on_progress(describe_tool(part.get("name", ""), part.get("input", {})))
        elif kind == "result":
            self.session = ev.get("session_id")
            self.ok = not ev.get("is_error")
            result = ev.get("result") or ""
            denied = ev.get("permission_denials") or []
            if denied and not result:
                result = "Not allowed: " + describe_tool(denied[0]["tool_name"], denied[0]["tool_input"])
            if not result and ev.get("subtype") == "error_during_execution":
                result = "Stopped."
            self.result = result
            return True
        return False

    async def _set_text(self, text):
        if text == self.text:
            return
        self.text = text
        if self.on_text:
            await self.on_text(text)


class Brain:
    def __init__(self):
        self.session = None
        self.last_at = 0.0
        self.proc = None       # oneshot: this request's process; session: the long-lived one
        self.turn = None       # the request in progress
        self.cancelled = False  # set by cancel(): the caller shouldn't report this request
        self.mode = None       # the mode of the last request
        self.last = None       # timings of the last request (dashboard)
        self.recent = {"session": collections.deque(maxlen=20), "oneshot": collections.deque(maxlen=20)}
        self._sig = None       # what the session process was started with (restart when it changes)
        self._events = None    # session: stdout events, read by _read()
        self._stderr = collections.deque(maxlen=40)
        self._stderr_task = None
        self._fresh = False    # forget() was called: the next request starts a new process
        self._idle = None      # session: timer that ends the process when unused
        self._req = 0

    @property
    def busy(self):
        return self.turn is not None

    @property
    def alive(self):
        return self.proc is not None and self.proc.returncode is None

    @property
    def pid(self):
        """Our own Claude Code process, if running (not counted as one of the user's sessions)."""
        return self.proc.pid if self.alive else None

    def forget(self):
        """A new conversation from the next request on."""
        self.session = None
        self._fresh = True
        if not self.busy and self.alive:
            asyncio.ensure_future(self._stop())

    def status(self):
        def avg(rows, key):
            vals = [r[key] for r in rows if r.get(key) is not None]
            return int(sum(vals) / len(vals)) if vals else None
        return {"mode": (config.load().get("brain_mode") or "session"), "alive": self.alive,
                "session": bool(self.session), "last": self.last,
                "avg": {m: {"n": len(rows), "first_token_ms": avg(rows, "first_token_ms"),
                            "total_ms": avg(rows, "total_ms")} for m, rows in self.recent.items() if rows}}

    async def ask(self, text, on_progress, on_text=None):
        """Runs one request. Returns (ok, reply). on_progress(line) gets tool activity, on_text(text) the
        answer as it's written ("" = what came so far wasn't the answer)."""
        if self.busy:
            return False, "Still working on the last request."
        self.cancelled = False
        cfg = config.load()
        claude = config.claude_path(cfg)
        if not claude:
            return False, "Claude Code not found. Install it, or set its path in the dashboard."
        mode = "oneshot" if cfg.get("brain_mode") == "oneshot" else "session"
        self.turn = turn = Turn(on_progress, on_text)
        info = {"mode": mode, "warm": False, "spawn_ms": None}
        try:
            if mode == "oneshot":
                if self.alive and self._events is not None:  # switched from session mode
                    await self._stop()
                ok, reply = await self._ask_oneshot(text, cfg, claude, turn, info)
            else:
                ok, reply = await self._ask_session(text, cfg, claude, turn, info)
        finally:
            self.turn = None
            self.last_at = time.time()
            self.mode = mode
        total = time.monotonic() - turn.started
        self.last = {**info, "at": time.time(), "ok": ok, "cancelled": self.cancelled,
                     "first_token_ms": int(turn.first_token * 1000) if turn.first_token is not None else None,
                     "total_ms": int(total * 1000)}
        if not self.cancelled and ok:
            self.recent[mode].append(self.last)
        log.info("%s%s: first token %s, total %.1f s%s", mode, " (warm)" if info["warm"] else "",
                 f"{turn.first_token:.2f} s" if turn.first_token is not None else "-", total,
                 " (cancelled)" if self.cancelled else "")
        if mode == "session":
            self._arm_idle(cfg)
        return ok, reply

    def cancel(self):
        if not self.busy:
            return
        self.cancelled = True
        if self._events is None:  # oneshot: the process is the request
            if self.alive:
                self.proc.kill()
            return
        if not self._send({"type": "control_request", "request_id": self._request_id(),
                           "request": {"subtype": "interrupt"}}):
            if self.alive:
                self.proc.kill()
            return
        turn, proc = self.turn, self.proc

        def still_going():
            if self.turn is turn and self.proc is proc and self.alive:
                log.warning("interrupt not answered in %d s: stopping Claude Code", INTERRUPT_WAIT_S)
                proc.kill()  # the next request starts it again with --resume
        asyncio.get_running_loop().call_later(INTERRUPT_WAIT_S, still_going)

    # --- oneshot: a process per request ---------------------------------------

    def _common_args(self, cfg):
        allowed = list(cfg.get("safe_tools") or [])
        mcp = []
        if cfg.get("mcp_server", True):
            # The stick tools (mcp_server.py) over HTTP on our own server. The feedback ones run
            # without asking; open_url / open_app / quick_command change the Mac, so they're left to
            # Claude Code's permission prompt (our hook -> A / B on the stick). --mcp-config takes
            # several values, so an option must follow it.
            mcp = ["--mcp-config", str(mcp_setup.brain_config())]
            allowed += [t for t in mcp_server_tools() if t not in allowed]
        return ["--output-format", "stream-json", "--verbose", "--include-partial-messages",
                "--model", cfg.get("model") or "sonnet", *mcp, "--allowedTools", ",".join(allowed)]

    async def _ask_oneshot(self, text, cfg, claude, turn, info):
        args = [claude, "-p", text, *self._common_args(cfg)]
        if self.session and time.time() - self.last_at < cfg.get("followup_minutes", 10) * 60:
            args += ["--resume", self.session]
        log.info("claude %s", " ".join(a if " " not in a else repr(a) for a in args[1:]))
        t = time.monotonic()
        self.proc = proc = await asyncio.create_subprocess_exec(
            *args, cwd=str(config.WORKSPACE), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            limit=16 * 1024 * 1024)
        info["spawn_ms"] = int((time.monotonic() - t) * 1000)
        try:
            async with asyncio.timeout(TIMEOUT_S):
                async for raw in proc.stdout:
                    try:
                        ev = json.loads(raw)
                    except ValueError:
                        continue
                    if await turn.feed(ev):
                        self.session = turn.session or self.session
                await proc.wait()
        except TimeoutError:
            proc.kill()
            return False, "Took too long, stopped."
        if turn.result is None:
            err = (await proc.stderr.read()).decode(errors="replace").strip()
            log.error("claude failed: %s", err[-500:])
            return False, plain(err.splitlines()[-1] if err else "Claude Code failed.")
        return turn.ok, plain(turn.result)

    # --- session: one long-lived process ----------------------------------------

    def _signature(self, cfg, claude):
        return (claude, cfg.get("model") or "sonnet", tuple(cfg.get("safe_tools") or []), str(config.WORKSPACE),
                bool(cfg.get("mcp_server", True)))

    async def prewarm(self):
        """Starts the session process ahead of the first request (when the stick connects)."""
        cfg = config.load()
        if cfg.get("brain_mode") == "oneshot" or not cfg.get("brain_prewarm", True) or self.busy or self.alive:
            return
        claude = config.claude_path(cfg)
        if not claude:
            return
        self._expire(cfg)
        try:
            await self._start(cfg, claude)
        except OSError as e:
            log.warning("pre-warm failed: %s", e)
            return
        self._arm_idle(cfg)

    def _expire(self, cfg):
        """After the idle time, a request starts a new conversation (like today's follow-up window)."""
        if self.session and time.time() - self.last_at >= self._idle_s(cfg):
            self.session = None

    def _idle_s(self, cfg):
        return float(cfg.get("brain_idle_min", 10)) * 60

    async def _start(self, cfg, claude):
        args = [claude, "-p", "--input-format", "stream-json", *self._common_args(cfg)]
        if self.session:
            args += ["--resume", self.session]  # after a crash, a restart or a config change: same conversation
        log.info("claude session: %s", " ".join(args[1:]))
        t = time.monotonic()
        self.proc = proc = await asyncio.create_subprocess_exec(
            *args, cwd=str(config.WORKSPACE), stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE, limit=16 * 1024 * 1024)
        self._sig = self._signature(cfg, claude)
        self._events = q = asyncio.Queue()
        self._stderr.clear()
        asyncio.ensure_future(self._read(proc, q))
        self._stderr_task = asyncio.ensure_future(self._read_stderr(proc))
        return int((time.monotonic() - t) * 1000)

    async def _read(self, proc, q):
        try:
            async for raw in proc.stdout:
                try:
                    q.put_nowait(json.loads(raw))
                except ValueError:
                    continue
        except (OSError, ValueError) as e:  # ValueError: a line over the limit
            log.warning("claude session output: %s", e)
        await proc.wait()
        log.info("claude session ended (exit %s)", proc.returncode)
        q.put_nowait(None)

    async def _read_stderr(self, proc):
        async for raw in proc.stderr:  # read it all, or a chatty process would block on a full pipe
            self._stderr.append(raw.decode(errors="replace").rstrip())

    async def _stop(self):
        proc, self.proc, self._events = self.proc, None, None
        if self._idle:
            self._idle.cancel()
            self._idle = None
        if proc is None or proc.returncode is not None:
            return
        try:
            proc.stdin.close()  # end of input: Claude Code exits on its own
            await asyncio.wait_for(proc.wait(), 3)
        except (TimeoutError, OSError):
            proc.kill()
            await proc.wait()

    def _arm_idle(self, cfg):
        if self._idle:
            self._idle.cancel()
        self._idle = asyncio.get_running_loop().call_later(self._idle_s(cfg), self._on_idle)

    def _on_idle(self):
        self._idle = None
        if self.busy:
            return
        log.info("claude session idle for %s min: stopping it (next request: a new conversation)",
                 config.load().get("brain_idle_min", 10))
        self.session = None
        asyncio.ensure_future(self._stop())

    def _request_id(self):
        self._req += 1
        return f"req_{self._req}_{os.urandom(4).hex()}"

    def _send(self, obj):
        if not self.alive or self.proc.stdin is None:
            return False
        try:
            self.proc.stdin.write((json.dumps(obj) + "\n").encode())
            return True
        except (OSError, RuntimeError):  # the process is gone
            return False

    async def _ask_session(self, text, cfg, claude, turn, info):
        self._expire(cfg)
        sig = self._signature(cfg, claude)
        if self.alive and (self._fresh or self._sig != sig):
            log.info("claude session: %s, restarting it", "new chat" if self._fresh else "settings changed")
            await self._stop()
        self._fresh = False
        for attempt in (1, 2):
            warm = self.alive
            if warm and self._drain():  # it died while idle
                await self._kill()
                warm = False
            if not warm:
                info["spawn_ms"] = await self._start(cfg, claude)
            info["warm"] = warm
            q, proc = self._events, self.proc
            msg = {"type": "user", "message": {"role": "user", "content": text},
                   "parent_tool_use_id": None, "session_id": self.session or ""}
            try:
                async with asyncio.timeout(TIMEOUT_S):
                    if self._send(msg):
                        await proc.stdin.drain()
                        while (ev := await q.get()) is not None:
                            if await turn.feed(ev):
                                break
            except TimeoutError:
                log.warning("claude session: no result in %d s, stopping it", TIMEOUT_S)
                await self._kill()
                return False, "Took too long, stopped."
            except (OSError, RuntimeError):  # writing to a process that just died
                pass
            if turn.result is not None:
                self.session = turn.session or self.session
                return turn.ok, plain(turn.result)
            # The process ended without a result: killed (cancel), or it crashed
            await self._kill()
            if self.cancelled:
                return False, "Cancelled."
            if self._stderr_task:
                await asyncio.wait([self._stderr_task], timeout=1)
            err = "\n".join(self._stderr).strip()
            log.error("claude session ended mid-request: %s", err[-500:])
            if attempt == 1 and warm and turn.events == 0:
                log.info("claude session: retrying in a new process (--resume %s)", self.session)
                continue  # it had died while idle: start again (same conversation) and resend
            return False, plain(err.splitlines()[-1] if err else "Claude Code stopped unexpectedly.")
        return False, "Claude Code stopped unexpectedly."

    def _drain(self):
        """Drops events left over between requests (hook events, a late result).
        True if the process has ended meanwhile."""
        q = self._events
        while q is not None and not q.empty():
            if q.get_nowait() is None:
                return True
        return False

    async def _kill(self):
        proc, self.proc, self._events = self.proc, None, None
        if proc is not None and proc.returncode is None:
            proc.kill()
            await proc.wait()
