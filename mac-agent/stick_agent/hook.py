#!/usr/bin/env python3
"""Claude Code PermissionRequest hook -> StickyBot (the agent).

Claude Code runs this when a tool needs permission. It asks the agent
(http://localhost:8660), which shows the prompt on the stick and the
dashboard and waits for A (approve) / B (deny).

If the agent isn't running or nobody answers in time, it prints nothing and
Claude Code shows its normal prompt — it never blocks you.

Register in ~/.claude/settings.json (install.sh offers to do it):
  "PermissionRequest": [{"hooks": [{"type": "command", "timeout": 45,
     "command": "<venv python> <this file>"}]}]
"""
import json
import os
import sys
import urllib.request
from pathlib import Path

HOME = Path(os.environ.get("STICK_AGENT_HOME", Path.home() / ".stick-agent"))
PORT = int(os.environ.get("STICK_AGENT_PORT", 8660))
WAIT_S = 30  # then Claude Code asks in the terminal (the hook's own timeout is 45 s)


def describe(tool, inp):
    if tool == "Bash":
        return inp.get("command", "")
    for key in ("file_path", "notebook_path", "path", "url", "pattern", "query"):
        if key in inp:
            return str(inp[key])
    return json.dumps(inp, separators=(",", ":"))


def main():
    try:
        req = json.load(sys.stdin)
        token = (HOME / "token").read_text().strip()
    except (ValueError, OSError):
        return
    cwd = Path(req.get("cwd", ""))
    origin = "Agent" if cwd == HOME / "workspace" else (cwd.name or "terminal")
    tool = req.get("tool_name", "?")
    body = json.dumps({"tool": tool, "hint": describe(tool, req.get("tool_input", {}))[:300],
                       "from": origin, "wait": WAIT_S}).encode()
    r = urllib.request.Request(f"http://127.0.0.1:{PORT}/api/permission", data=body, method="POST",
                               headers={"Content-Type": "application/json", "X-Stick-Token": token})
    try:
        with urllib.request.urlopen(r, timeout=WAIT_S + 5) as resp:
            answer = json.load(resp).get("decision")
    except OSError:
        return  # agent not running
    if answer not in ("allow", "deny"):
        return  # nobody answered: Claude Code asks in the terminal
    decision = {"behavior": answer}
    if answer == "deny":
        decision["message"] = "Denied on the stick"
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}))


if __name__ == "__main__":
    main()
