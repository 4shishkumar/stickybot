"""Brains that talk to an OpenAI-compatible chat API instead of Claude Code: OpenRouter (your own
key, pay per use) now; Ollama (local, http://localhost:11434/v1, no key) can reuse the same code.

OpenAICompatBrain has the shared part: the conversation in memory, one streamed
POST {base_url}/chat/completions per round (SSE: `data: {chunk}` lines, `: comment` keep-alives,
`data: [DONE]`), the stick tools as OpenAI-style functions (mcp_server.py's StickTools, the same code
the MCP server runs; open_url / open_app / quick_command ask on the stick first), and a tool loop of
at most MAX_ROUNDS rounds. A subclass says where to send it, which key, and what errors mean.

OpenRouter specifics (checked against https://openrouter.ai/docs, Nov 2026):
- Headers: Authorization: Bearer <key>; HTTP-Referer and X-Title name the app.
- Usage (tokens and `usage.cost` in credits = USD) comes in every response, for a stream in the last
  chunk before [DONE]; the old `usage: {"include": true}` request field is deprecated (no effect),
  so we don't send it.
- An error before the stream starts is an HTTP status with {"error": {"code", "message"}}: 401 bad
  key, 402 no credits, 429 rate limited, 502/503 the model is down / no provider. An error after it
  started comes as a chunk with a top-level "error" (and finish_reason "error").
- Closing the connection cancels the request (and its billing, where the provider supports it).
- A model that can't call tools: its `supported_parameters` (GET /models) lack "tools", and asking
  anyway gets a 404/400 "No endpoints found that support tool use". We check the list first, and on
  such an error retry once without tools (and remember the model).
"""
import asyncio
import collections
import json
import logging
import time

import aiohttp

from . import brain, config, keychain

log = logging.getLogger("brain")

MAX_ROUNDS = 5           # tool rounds per request; then one last round without tools for the answer
HISTORY_MESSAGES = 24    # kept between requests (user / assistant / tool messages); oldest turns go first
TOOL_RESULT_CHARS = 2000
CONNECT_S = 10
READ_S = 60              # nothing from the server this long mid-stream -> give up
MODELS_TTL_S = 600       # the model list is cached this long

SYSTEM = """You are the user's voice assistant on their Mac. Requests arrive by voice from a small \
handheld device (an M5StickS3) and were transcribed automatically, so expect transcription slips \
("get hub" = GitHub, numbers written as words). Act on what the user most likely meant.

Your reply is shown on a 135x240 pixel screen and may be read aloud:
- One or two short sentences, at most about 200 characters. Plain text only: no markdown, no lists, \
no code blocks, no emoji.
- Lead with the answer or the result ("Opened Slack.", "Paris is the capital of France.").
- If you don't know something current (news, weather, prices), say so briefly instead of guessing.

You can't run commands on the Mac or browse the web. Your tools: open_url, open_app and quick_command \
change the Mac and need the user's approval on the device (they press a button); notify, ask (an A/B \
question on the device), celebrate, progress, pin_home, speak and stick_status are feedback on the \
device. Your reply already shows on the device, so don't notify about it. Use tools only when they help."""


def system_prompt(now=None):
    t = time.localtime(now)
    return SYSTEM + "\n\nIt is " + time.strftime("%A %d %B %Y, %H:%M", t) + " for the user."


def describe(name, args):
    """A stick tool call as a progress line ("Opening github.com")."""
    import re
    a = args if isinstance(args, dict) else {}
    lines = {"open_url": lambda: "Opening " + re.sub(r"^https?://", "", str(a.get("url", "")))[:80],
             "open_app": lambda: "Opening " + str(a.get("name", ""))[:60],
             "quick_command": lambda: "Running: " + str(a.get("phrase", ""))[:100],
             "ask": lambda: "Asking: " + str(a.get("question", ""))[:100],
             "notify": lambda: "Notifying: " + str(a.get("text", ""))[:100],
             "speak": lambda: "Speaking",
             "celebrate": lambda: "Celebrating",
             "progress": lambda: "Progress: " + str(a.get("text", ""))[:100],
             "pin_home": lambda: "Pinning: " + str(a.get("text", ""))[:100],
             "stick_status": lambda: "Checking the stick"}
    return lines[name]() if name in lines else name


class ApiError(Exception):
    """The API said no (an HTTP status, or an error chunk mid-stream)."""

    def __init__(self, status, message):
        super().__init__(f"{status}: {message}")
        self.status, self.message = status, message or ""


class Round:
    """One streamed completion: its text, tool calls (merged from their deltas), usage."""

    def __init__(self):
        self.text = ""
        self.calls = {}       # index -> {"id", "name", "arguments"}
        self.finish = None
        self.usage = None

    def tool_calls(self):
        return [c for _, c in sorted(self.calls.items()) if c["name"]]

    def add_call_delta(self, d):
        c = self.calls.setdefault(d.get("index", len(self.calls)), {"id": "", "name": "", "arguments": ""})
        f = d.get("function") or {}
        c["id"] = d.get("id") or c["id"]
        c["name"] += f.get("name") or ""
        c["arguments"] += f.get("arguments") or ""


async def sse_data(content):
    """The `data:` payloads of an SSE stream, one per line (comments and other fields skipped)."""
    async for raw in content:
        line = raw.decode(errors="replace").rstrip("\r\n")
        if line.startswith("data:"):
            yield line[5:].strip()


async def error_message(resp):
    try:
        body = await resp.json(content_type=None)
        err = body.get("error") if isinstance(body, dict) else None
        if isinstance(err, dict):
            return str(err.get("message") or "")
        return str(err or body)[:300]
    except (ValueError, aiohttp.ClientError):
        return (await resp.text(errors="replace"))[:300] if not resp.closed else ""


def refuses_tools(e):
    return e.status in (400, 404) and "tool" in e.message.lower()


class OpenAICompatBrain:
    """The Brain interface (see brain.py) for an OpenAI-compatible chat API."""

    kind = "openai"          # the config / status name
    label = "the model"      # in user-facing messages ("Couldn't reach OpenRouter.")
    base_url = ""
    needs_key = False

    def __init__(self, agent=None):
        self.agent = agent
        self.history = []        # past turns (the system prompt goes in front per request)
        self.last_at = 0.0
        self.cancelled = False
        self.last = None         # the last request (dashboard)
        self.recent = collections.deque(maxlen=20)
        self.pid = None          # no process of ours
        self._task = None
        self._session = None
        self._no_tools = set()   # models that refused tools this run

    # --- what a subclass says -----------------------------------------------------------

    def model(self, cfg):
        return ""

    def api_key(self):
        """The key (blocking: runs in a worker thread), or None."""
        return None

    def headers(self, key):
        return {"Authorization": f"Bearer {key}"} if key else {}

    def max_tokens(self, cfg):
        return 400

    def tools_supported(self, model):
        """True / False if known, None if not (then we try, and fall back)."""
        return None

    def error_text(self, e):
        return f"{self.label} answered {e.status}: {brain.plain(e.message)[:120]}"

    def setup_text(self, missing):
        return f"Set up {self.label} in Settings → Brain ({missing})."

    # --- the Brain interface ----------------------------------------------------------------

    @property
    def busy(self):
        return self._task is not None

    @property
    def alive(self):
        return False

    def forget(self):
        self.history = []

    def cancel(self):
        if self._task is not None:
            self.cancelled = True
            self._task.cancel()  # closes the HTTP stream (the request stops there)

    async def prewarm(self):
        pass

    def status(self):
        cfg = config.load()
        vals = lambda k: [r[k] for r in self.recent if r.get(k) is not None]
        avg = lambda k: int(sum(vals(k)) / len(vals(k))) if vals(k) else None
        return {"provider": self.label, "model": self.model(cfg), "alive": False, "session": bool(self.history),
                "last": self.last,
                "avg": {self.kind: {"n": len(self.recent), "first_token_ms": avg("first_token_ms"),
                                    "total_ms": avg("total_ms")}} if self.recent else {}}

    async def ask(self, text, on_progress, on_text=None):
        if self.busy:
            return False, "Still working on the last request."
        self.cancelled = False
        cfg = config.load()
        self._expire(cfg)
        info = {"model": self.model(cfg), "first_token_ms": None, "cost": None, "rounds": 0, "tools": []}
        started = time.monotonic()
        self._task = task = asyncio.ensure_future(self._converse(text, cfg, on_progress, on_text, info, started))
        try:
            ok, reply = await task
        except asyncio.CancelledError:
            if not self.cancelled or asyncio.current_task().cancelling():
                raise  # we were cancelled ourselves, not by cancel()
            ok, reply = False, "Cancelled."
        finally:
            self._task = None
            self.last_at = time.time()
        self.last = {**info, "at": time.time(), "ok": ok, "cancelled": self.cancelled,
                     "error": None if ok or self.cancelled else reply,
                     "total_ms": int((time.monotonic() - started) * 1000)}
        if ok and not self.cancelled:
            self.recent.append(self.last)
        log.info("%s %s: first token %s, total %.1f s, %d round%s, cost %s%s", self.kind, info["model"],
                 f"{info['first_token_ms'] / 1000:.2f} s" if info["first_token_ms"] is not None else "-",
                 self.last["total_ms"] / 1000, info["rounds"], "" if info["rounds"] == 1 else "s",
                 f"${info['cost']:.5f}" if info["cost"] is not None else "-", " (cancelled)" if self.cancelled else
                 "" if ok else f" (error: {reply})")
        return ok, reply

    async def test(self, model=None):
        """One short request without history or tools (the dashboard's Test button)."""
        if self.busy:
            return {"ok": False, "error": "Busy with a request, try again in a moment."}
        cfg = config.load()
        model = model or self.model(cfg)
        info = {"model": model, "first_token_ms": None, "cost": None, "rounds": 0, "tools": []}
        started = time.monotonic()
        msgs = [{"role": "system", "content": system_prompt()},
                {"role": "user", "content": "Say hello and tell me which model you are, in one short sentence."}]
        try:
            key = await self._key()
            why = self._not_ready(key, model)
            if why:
                return {"ok": False, "error": why, "model": model}
            r = await self._complete(key, model, msgs, False, 60, None, info, started)
            ok, reply = True, brain.plain(r.text) or "(an empty reply)"
        except ApiError as e:
            ok, reply = False, self.error_text(e)
        except (aiohttp.ClientError, OSError, TimeoutError) as e:
            ok, reply = False, self._network_text(e)
        return {"ok": ok, "reply": reply if ok else None, "error": None if ok else reply, "model": model,
                "first_token_ms": info["first_token_ms"], "total_ms": int((time.monotonic() - started) * 1000),
                "cost": info["cost"]}

    async def close(self):
        if self._session is not None and not self._session.closed:
            await self._session.close()
        self._session = None

    # --- the conversation -------------------------------------------------------------------

    def _expire(self, cfg):
        """Unused for brain_idle_min: the next request starts a new conversation (like Claude's session)."""
        if self.history and time.time() - self.last_at >= float(cfg.get("brain_idle_min", 10)) * 60:
            log.info("%s: idle for %s min, new conversation", self.kind, cfg.get("brain_idle_min", 10))
            self.history = []

    def _remember(self, turn):
        h = self.history + turn
        while len(h) > HISTORY_MESSAGES:  # drop the oldest turn (up to the next user message) as a whole
            nxt = next((i for i, m in enumerate(h) if i > 0 and m["role"] == "user"), len(h))
            h = h[nxt:]
        self.history = h

    async def _key(self):
        if not self.needs_key:
            return None
        return await asyncio.get_running_loop().run_in_executor(None, self.api_key)

    def _not_ready(self, key, model):
        if self.needs_key and not key:
            return self.setup_text("no key")
        if not model:
            return self.setup_text("no model")
        return ""

    def _network_text(self, e):
        if isinstance(e, TimeoutError):
            return f"{self.label} stopped answering. Try again."
        return f"Couldn't reach {self.label}. Check the internet connection."

    async def _converse(self, text, cfg, on_progress, on_text, info, started):
        key = await self._key()
        model = info["model"]
        why = self._not_ready(key, model)
        if why:
            return False, why
        turn = [{"role": "user", "content": text}]
        use_tools = (bool(cfg.get("mcp_server", True)) and model not in self._no_tools
                     and self.tools_supported(model) is not False)
        shown = [""]

        async def set_text(t):  # the current message's text so far ("" = withdraw a preamble)
            if t != shown[0]:
                shown[0] = t
                if on_text:
                    await on_text(t)
        try:
            for rnd in range(MAX_ROUNDS + 1):
                tools = use_tools and rnd < MAX_ROUNDS
                if rnd and on_progress:
                    await on_progress("Thinking…")
                await set_text("")
                msgs = [{"role": "system", "content": system_prompt()}, *self.history, *turn]
                try:
                    r = await self._complete(key, model, msgs, tools, self.max_tokens(cfg), set_text, info, started)
                except ApiError as e:
                    if not (tools and refuses_tools(e)):
                        raise
                    log.info("%s: %s can't use tools (%s), going on without them", self.kind, model, e.message[:120])
                    self._no_tools.add(model)
                    use_tools = tools = False
                    r = await self._complete(key, model, msgs, False, self.max_tokens(cfg), set_text, info, started)
                calls = r.tool_calls() if tools else []
                if not calls:
                    answer = r.text.strip()
                    turn.append({"role": "assistant", "content": answer})
                    self._remember(turn)
                    return True, brain.plain(answer) or "(no answer)"
                await set_text("")  # what it wrote before the tool calls wasn't the answer
                turn.append({"role": "assistant", "content": r.text or None,
                             "tool_calls": [{"id": c["id"] or f"call_{rnd}_{i}", "type": "function",
                                             "function": {"name": c["name"], "arguments": c["arguments"] or "{}"}}
                                            for i, c in enumerate(calls)]})
                for i, c in enumerate(calls):
                    result = await self._run_tool(c, on_progress, info)
                    turn.append({"role": "tool", "tool_call_id": c["id"] or f"call_{rnd}_{i}", "content": result})
            return False, "Too many steps, stopped."  # (not reached: the last round has no tools)
        except ApiError as e:
            return False, self.error_text(e)
        except (aiohttp.ClientError, OSError, TimeoutError) as e:
            log.warning("%s: %s", self.kind, e)
            return False, self._network_text(e)

    async def _run_tool(self, call, on_progress, info):
        from . import mcp_server  # (it imports core, which imports us)
        name = call["name"]
        try:
            args = json.loads(call["arguments"] or "{}")
        except ValueError:
            return "error: the arguments weren't valid JSON"
        if name not in mcp_server.BY_NAME:
            return f"error: no tool called {name}"
        info["tools"].append(name)
        if on_progress:
            await on_progress(describe(name, args))
        if self.agent is None:
            return "error: the stick tools aren't available"
        tools = getattr(self.agent, "mcp_tools", None)
        if tools is None:
            tools = self.agent.mcp_tools = mcp_server.StickTools(self.agent)
        text, err, _ = await tools.call_approved(name, args, "Agent")
        return (("error: " if err else "") + text)[:TOOL_RESULT_CHARS]

    def _http(self):
        if self._session is None or self._session.closed:
            self._session = aiohttp.ClientSession()
        return self._session

    async def _complete(self, key, model, messages, tools, max_tokens, set_text, info, started):
        """One streamed completion. Raises ApiError, aiohttp.ClientError, TimeoutError."""
        from .mcp_server import openai_tools
        body = {"model": model, "messages": messages, "stream": True, "max_tokens": max_tokens}
        if tools:
            body["tools"] = openai_tools()
        info["rounds"] += 1
        timeout = aiohttp.ClientTimeout(total=None, connect=CONNECT_S, sock_read=READ_S)
        r = Round()
        async with self._http().post(self.base_url + "/chat/completions", json=body, headers=self.headers(key),
                                     timeout=timeout) as resp:
            if resp.status != 200:
                raise ApiError(resp.status, await error_message(resp))
            async for data in sse_data(resp.content):
                if data == "[DONE]":
                    break
                try:
                    chunk = json.loads(data)
                except ValueError:
                    continue
                err = chunk.get("error")
                if err:
                    e = err if isinstance(err, dict) else {"message": str(err)}
                    raise ApiError(e.get("code") or 500, str(e.get("message") or "error mid-stream"))
                if chunk.get("usage"):
                    r.usage = chunk["usage"]
                for ch in chunk.get("choices") or []:
                    d = ch.get("delta") or {}
                    for tc in d.get("tool_calls") or []:
                        r.add_call_delta(tc)
                    piece = d.get("content")
                    if piece:
                        if info["first_token_ms"] is None:
                            info["first_token_ms"] = int((time.monotonic() - started) * 1000)
                        r.text += piece
                        if set_text and not r.calls:
                            await set_text(r.text)
                    r.finish = ch.get("finish_reason") or r.finish
        cost = (r.usage or {}).get("cost")
        if isinstance(cost, (int, float)):
            info["cost"] = (info["cost"] or 0) + cost
        return r


# --- OpenRouter ------------------------------------------------------------------------------

# Fast, cheap models that can call tools, in order of preference: the first one in the live list is
# suggested (checked against https://openrouter.ai/api/v1/models, Nov 2026). None there -> the
# cheapest tool-capable one.
PREFERRED = ("google/gemini-3.1-flash-lite", "anthropic/claude-haiku-5.5", "openai/gpt-5.4-nano",
             "google/gemini-2.5-flash-lite", "openai/gpt-4.1-nano", "openai/gpt-4o-mini")


def per_million(price):
    try:
        v = float(price) * 1_000_000
    except (TypeError, ValueError):
        return None
    return round(v, 4) if v >= 0 else None


def simplify(m):
    """A /models entry -> what the dashboard needs (None if it can't chat)."""
    if not isinstance(m, dict) or not isinstance(m.get("id"), str):
        return None
    out_mod = (m.get("architecture") or {}).get("output_modalities")
    if out_mod and "text" not in out_mod:
        return None
    p = m.get("pricing") or {}
    return {"id": m["id"], "name": str(m.get("name") or m["id"]), "in": per_million(p.get("prompt")),
            "out": per_million(p.get("completion")), "tools": "tools" in (m.get("supported_parameters") or []),
            "context": m.get("context_length")}


def suggest(models):
    ids = {m["id"]: m for m in models}
    for mid in PREFERRED:
        if mid in ids and ids[mid]["tools"]:
            return mid
    ok = [m for m in models if m["tools"] and m["in"] and m["out"] is not None
          and not m["id"].startswith("~") and ":" not in m["id"]]
    return min(ok, key=lambda m: m["in"] + m["out"])["id"] if ok else None


class OpenRouterBrain(OpenAICompatBrain):
    kind = "openrouter"
    label = "OpenRouter"
    base_url = "https://openrouter.ai/api/v1"
    needs_key = True
    REFERER = "https://github.com/ashish-kumar-noon/stickybot"

    def __init__(self, agent=None):
        super().__init__(agent)
        self.models = None       # simplified /models list (cached)
        self.models_at = 0.0
        self._models_lock = None

    def model(self, cfg):
        return str(cfg.get("openrouter_model") or "").strip()

    def api_key(self):
        return keychain.get("openrouter")

    def headers(self, key):
        return {**super().headers(key), "HTTP-Referer": self.REFERER, "X-Title": "StickyBot"}

    def max_tokens(self, cfg):
        try:
            return max(50, min(4000, int(cfg.get("openrouter_max_tokens") or 400)))
        except (TypeError, ValueError):
            return 400

    def tools_supported(self, model):
        for m in self.models or []:
            if m["id"] == model:
                return m["tools"]
        return None

    def setup_text(self, missing):
        if missing == "no key":
            return "Add your OpenRouter key in Settings → Brain."
        return "Pick an OpenRouter model in Settings → Brain."

    def error_text(self, e):
        if e.status == 401:
            return "OpenRouter rejected the key. Check it in Settings → Brain."
        if e.status == 402:
            return "Out of OpenRouter credits. Add some at openrouter.ai."
        if e.status == 429:
            return "OpenRouter is rate limiting. Try again in a moment."
        if e.status == 403:
            return "OpenRouter refused that request (moderation or permissions)."
        if e.status == 404 or e.status == 400 and "model" in e.message.lower():
            return "OpenRouter doesn't know that model. Pick another in Settings → Brain."
        if e.status in (502, 503):
            return "The model is unavailable right now. Try again, or pick another."
        if e.status == 408:
            return "OpenRouter timed out. Try again."
        return f"OpenRouter: {brain.plain(e.message)[:120] or f'error {e.status}'}"

    def status(self):
        st = super().status()
        st["key"] = keychain.known_status("openrouter")
        return st

    async def list_models(self, force=False):
        """(models, suggested id, error). Cached MODELS_TTL_S; a stale list beats none when offline."""
        if self._models_lock is None:
            self._models_lock = asyncio.Lock()
        async with self._models_lock:
            err = None
            if force or self.models is None or time.monotonic() - self.models_at >= MODELS_TTL_S:
                try:
                    timeout = aiohttp.ClientTimeout(total=20)
                    async with self._http().get(self.base_url + "/models", timeout=timeout,
                                                headers={"HTTP-Referer": self.REFERER, "X-Title": "StickyBot"}) as r:
                        if r.status != 200:
                            raise ApiError(r.status, await error_message(r))
                        data = (await r.json(content_type=None)).get("data")
                    if not isinstance(data, list):
                        raise ApiError(502, "unexpected model list")
                    self.models = sorted((s for s in map(simplify, data) if s), key=lambda m: m["name"].lower())
                    self.models_at = time.monotonic()
                except (ApiError, aiohttp.ClientError, OSError, TimeoutError, ValueError, AttributeError) as e:
                    log.warning("openrouter models: %s", e)
                    err = "Couldn't load the model list from OpenRouter."
            models = self.models or []
            return models, suggest(models), err
