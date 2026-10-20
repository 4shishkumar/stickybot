#!/usr/bin/env python3
"""A stand-in for `claude -p ... --output-format stream-json`.

FAKE_CLAUDE_MODE picks the behaviour; every run appends its argv to FAKE_CLAUDE_LOG."""
import json
import os
import sys
import time

mode = os.environ.get("FAKE_CLAUDE_MODE", "answer")
with open(os.environ["FAKE_CLAUDE_LOG"], "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")


def say(obj):
    print(json.dumps(obj), flush=True)


if mode == "answer":
    say({"type": "system", "subtype": "init"})
    say({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "name": "Bash", "input": {"command": "df -h /"}},
        {"type": "tool_use", "name": "WebSearch", "input": {"query": "weather bengaluru"}}]}})
    say({"type": "result", "session_id": "sess-1", "is_error": False,
         "result": "**It's 28°C** in Bengaluru.\n```\nignored\n```"})
elif mode == "denied":
    say({"type": "result", "session_id": "sess-2", "is_error": False, "result": "",
         "permission_denials": [{"tool_name": "Bash", "tool_input": {"command": "rm -rf build/"}}]})
elif mode == "error":
    say({"type": "result", "session_id": "sess-3", "is_error": True, "result": "Rate limited"})
elif mode == "crash":
    print("Error: not logged in", file=sys.stderr)
    sys.exit(1)
elif mode == "slow":
    time.sleep(30)
