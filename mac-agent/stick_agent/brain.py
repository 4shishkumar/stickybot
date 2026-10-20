"""Hands a request to Claude Code (headless, the user's own login).

Safe tools (web search, reading) run without asking; anything else triggers
Claude Code's permission prompt, which our hook answers from the stick or the
dashboard. Requests within a few minutes continue the same conversation.
"""
import asyncio
import json
import logging
import re
import time

from . import config

log = logging.getLogger("claude")
TIMEOUT_S = 240


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
    t = re.sub(r"[*_`#>]+", "", t)
    t = re.sub(r"\s+", " ", t).strip()
    return t if len(t) <= 280 else t[:277].rsplit(" ", 1)[0] + "…"


class Brain:
    def __init__(self):
        self.session = None
        self.last_at = 0.0
        self.proc = None
        self.cancelled = False  # set by cancel(): the caller shouldn't report this request

    @property
    def busy(self):
        return self.proc is not None and self.proc.returncode is None

    def forget(self):
        self.session = None

    async def ask(self, text, on_progress):
        """Runs one request. Returns (ok, reply). on_progress(line) gets tool activity."""
        self.cancelled = False
        cfg = config.load()
        claude = config.claude_path(cfg)
        if not claude:
            return False, "Claude Code not found. Install it, or set its path in the dashboard."
        args = [claude, "-p", text, "--output-format", "stream-json", "--verbose",
                "--model", cfg.get("model") or "sonnet",
                "--allowedTools", ",".join(cfg.get("safe_tools") or [])]
        if self.session and time.time() - self.last_at < cfg.get("followup_minutes", 10) * 60:
            args += ["--resume", self.session]
        log.info("claude %s", " ".join(a if " " not in a else repr(a) for a in args[1:]))
        self.proc = await asyncio.create_subprocess_exec(
            *args, cwd=str(config.WORKSPACE), stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            limit=16 * 1024 * 1024)
        result, ok = None, False
        try:
            async with asyncio.timeout(TIMEOUT_S):
                async for raw in self.proc.stdout:
                    try:
                        ev = json.loads(raw)
                    except ValueError:
                        continue
                    if ev.get("type") == "assistant":
                        for part in ev.get("message", {}).get("content", []):
                            if part.get("type") == "tool_use":
                                await on_progress(describe_tool(part.get("name", ""), part.get("input", {})))
                    elif ev.get("type") == "result":
                        self.session = ev.get("session_id") or self.session
                        ok = not ev.get("is_error")
                        result = ev.get("result") or ""
                        denied = ev.get("permission_denials") or []
                        if denied and not result:
                            result = "Not allowed: " + describe_tool(denied[0]["tool_name"], denied[0]["tool_input"])
                await self.proc.wait()
        except TimeoutError:
            self.proc.kill()
            return False, "Took too long, stopped."
        finally:
            self.last_at = time.time()
        if result is None:
            err = (await self.proc.stderr.read()).decode(errors="replace").strip()
            log.error("claude failed: %s", err[-500:])
            return False, plain(err.splitlines()[-1] if err else "Claude Code failed.")
        return ok, plain(result)

    def cancel(self):
        if self.busy:
            self.cancelled = True
            self.proc.kill()
