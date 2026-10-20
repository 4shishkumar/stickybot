#!/usr/bin/env python3
"""Claude Code hooks for "Claude finished" on the stick.

Registered for two hook events in ~/.claude/settings.json:
  UserPromptSubmit — remembers when you sent the prompt (per session)
  Stop             — Claude finished answering: tells the agent how long it took;
                     the agent shows "Claude finished in <project>" if it took at
                     least claude_done_min_s (dashboard → Events)
It never blocks or changes Claude Code: no output, short timeouts, always exit 0.
Sessions of the agent's own Claude (cwd ~/.stick-agent/workspace) are ignored.
"""
import json
import os
import sys
import time
import urllib.request
from pathlib import Path

HOME = Path(os.environ.get("STICK_AGENT_HOME", Path.home() / ".stick-agent"))
PORT = int(os.environ.get("STICK_AGENT_PORT", 8660))
SESSIONS = HOME / "claude_sessions.json"
KEEP_S = 24 * 3600


def load():
    try:
        return json.loads(SESSIONS.read_text())
    except (OSError, ValueError):
        return {}


def save(d):
    tmp = SESSIONS.with_suffix(".tmp")
    tmp.write_text(json.dumps(d))
    tmp.replace(SESSIONS)  # atomic: several sessions may run hooks at once


def main():
    try:
        req = json.load(sys.stdin)
    except ValueError:
        return
    event = req.get("hook_event_name", "")
    sid = req.get("session_id") or "?"
    cwd = req.get("cwd", "")
    if Path(cwd or "/").resolve() == (HOME / "workspace").resolve():
        return  # the agent's own Claude
    now = time.time()
    sessions = {k: v for k, v in load().items() if now - v.get("t", 0) < KEEP_S}

    if event == "UserPromptSubmit":
        sessions[sid] = {"t": now, "prompt": (req.get("prompt") or "")[:200], "cwd": cwd}
        save(sessions)
        return

    if event == "Stop":
        start = sessions.pop(sid, None)
        save(sessions)
        if not start:
            return
        body = json.dumps({"elapsed_s": round(now - start["t"]), "cwd": cwd or start.get("cwd", ""),
                           "prompt": start.get("prompt", "")}).encode()
        try:
            token = (HOME / "token").read_text().strip()
            r = urllib.request.Request(f"http://127.0.0.1:{PORT}/api/claude-event", data=body, method="POST",
                                       headers={"Content-Type": "application/json", "X-Stick-Token": token})
            urllib.request.urlopen(r, timeout=2).close()
        except OSError:
            pass  # agent not running: nothing to show


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 — never disturb Claude Code
        pass
    sys.exit(0)
