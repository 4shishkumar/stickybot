"""The Claude Code hook scripts, run as Claude Code runs them (stdin JSON -> stdout)."""
import asyncio
import json
import os
import sys
import time
from pathlib import Path

import pytest
from aiohttp.test_utils import TestServer

from stick_agent import config, server

PKG = Path(__file__).parent.parent / "stick_agent"


def env_for(home):
    return {**os.environ, "STICK_AGENT_HOME": str(home), "STICK_AGENT_PORT": str(config.PORT)}


async def run_hook(script, payload, home):
    p = await asyncio.create_subprocess_exec(
        sys.executable, str(PKG / script), stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE, env=env_for(home))
    out, err = await asyncio.wait_for(p.communicate(json.dumps(payload).encode()), 20)
    return p.returncode, out.decode(), err.decode()


PERMISSION = {"tool_name": "Bash", "tool_input": {"command": "rm -rf build/"}, "cwd": "/Users/me/web"}


async def test_permission_hook_agent_not_running(home):
    t = time.time()
    code, out, _ = await run_hook("hook.py", PERMISSION, home)
    assert code == 0 and out == "" and time.time() - t < 5


async def test_permission_hook_bad_input(home):
    p = await asyncio.create_subprocess_exec(sys.executable, str(PKG / "hook.py"), stdin=asyncio.subprocess.PIPE,
                                             stdout=asyncio.subprocess.PIPE, env=env_for(home))
    out, _ = await p.communicate(b"not json")
    assert p.returncode == 0 and out == b""


@pytest.fixture
async def live_agent(agent):
    """The agent's API on its real port, as the hooks expect."""
    srv = TestServer(server.make_app(agent), host="127.0.0.1", port=config.PORT)
    await srv.start_server()
    yield agent
    await srv.close()


@pytest.mark.parametrize("allow, behavior", [(True, "allow"), (False, "deny")])
async def test_permission_hook_round_trip(home, live_agent, allow, behavior):
    task = asyncio.create_task(run_hook("hook.py", PERMISSION, home))
    for _ in range(300):
        if live_agent.prompts:
            break
        await asyncio.sleep(0.02)
    pid, p = next(iter(live_agent.prompts.items()))
    assert p["tool"] == "Bash" and p["hint"] == "rm -rf build/" and p["from"] == "web"
    live_agent.decide(pid, allow)
    code, out, _ = await task
    decision = json.loads(out)["hookSpecificOutput"]["decision"]
    assert code == 0 and decision["behavior"] == behavior


async def test_claude_hooks_record_and_report(home, live_agent):
    live_agent.cfg["claude_done_min_s"] = 0
    start = {"hook_event_name": "UserPromptSubmit", "session_id": "s1", "cwd": "/Users/me/web", "prompt": "fix CI"}
    code, out, _ = await run_hook("claude_hooks.py", start, home)
    assert code == 0 and out == ""
    sessions = json.loads((home / "claude_sessions.json").read_text())
    assert sessions["s1"]["prompt"] == "fix CI"
    code, out, _ = await run_hook("claude_hooks.py", {"hook_event_name": "Stop", "session_id": "s1",
                                                      "cwd": "/Users/me/web"}, home)
    assert code == 0 and out == ""
    assert "s1" not in json.loads((home / "claude_sessions.json").read_text())
    titles = [m["event"]["title"] for m in live_agent.link.sent if "event" in m]
    assert titles == ["Claude finished in web"]


async def test_claude_hooks_ignore_the_agents_own_claude(home):
    req = {"hook_event_name": "UserPromptSubmit", "session_id": "own", "cwd": str(home / "workspace")}
    code, out, _ = await run_hook("claude_hooks.py", req, home)
    assert code == 0 and out == "" and not (home / "claude_sessions.json").exists()


async def test_claude_hooks_agent_not_running_is_fine(home):
    for ev in ("UserPromptSubmit", "Stop"):
        code, out, err = await run_hook("claude_hooks.py", {"hook_event_name": ev, "session_id": "x", "cwd": "/tmp"},
                                        home)
        assert code == 0 and out == "" and err == ""


async def test_claude_hooks_garbage_input(home):
    p = await asyncio.create_subprocess_exec(sys.executable, str(PKG / "claude_hooks.py"),
                                             stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                                             env=env_for(home))
    out, _ = await p.communicate(b"\x00\x01garbage")
    assert p.returncode == 0 and out == b""
