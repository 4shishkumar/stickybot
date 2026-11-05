"""Claude Code runner: the persistent session, the one-shot fallback, stream-json parsing,
follow-ups, idle restart, crash recovery, timeouts, cancel. A fake CLI stands in for claude."""
import asyncio
import json
import os
import sys
import time
from pathlib import Path

import pytest

from stick_agent import brain, config

FAKE = Path(__file__).parent / "fake_claude.py"


@pytest.fixture
def fake_claude(home, tmp_path, monkeypatch):
    """Points the agent at fake_claude.py (via a tiny wrapper so it runs with this Python).
    Returns calls(): the argv of every start."""
    wrapper = tmp_path / "claude"
    wrapper.write_text(f"#!/bin/sh\nexec {sys.executable} {FAKE} \"$@\"\n")
    wrapper.chmod(0o755)
    log = tmp_path / "claude_calls.jsonl"
    monkeypatch.setenv("FAKE_CLAUDE_LOG", str(log))
    monkeypatch.delenv("FAKE_IGNORE_INTERRUPT", raising=False)
    set_config(claude_path=str(wrapper))

    def calls():
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    calls.messages = lambda: ([json.loads(line) for line in Path(str(log) + ".msgs").read_text().splitlines()]
                              if Path(str(log) + ".msgs").exists() else [])
    return calls


@pytest.fixture
async def brains():
    """Brains made by a test; their Claude Code processes are stopped afterwards."""
    made = []

    def make():
        made.append(brain.Brain())
        return made[-1]
    yield make
    for b in made:
        await b._kill()


def set_config(**kw):
    cfg = config.load()
    cfg.update(kw)
    config.save(cfg)


class Collect:
    def __init__(self):
        self.progress, self.texts = [], []

    async def on_progress(self, line):
        self.progress.append(line)

    async def on_text(self, text):
        self.texts.append(text)


async def ask(b, text="weather?"):
    c = Collect()
    ok, reply = await b.ask(text, c.on_progress, c.on_text)
    return ok, reply, c


async def until(cond, timeout=5.0):
    end = time.monotonic() + timeout
    while not cond():
        assert time.monotonic() < end, "timed out"
        await asyncio.sleep(0.02)


# --- one-shot mode (the fallback) ------------------------------------------------------

@pytest.fixture
def oneshot(fake_claude):
    set_config(brain_mode="oneshot")
    return fake_claude


async def run(mode, monkeypatch, b=None, text="weather?"):
    monkeypatch.setenv("FAKE_CLAUDE_MODE", mode)
    b = b or brain.Brain()
    ok, reply, c = await ask(b, text)
    return b, ok, reply, c.progress


async def test_answer_progress_and_plain_text(oneshot, monkeypatch):
    b, ok, reply, progress = await run("answer", monkeypatch)
    assert ok
    assert reply == "It's 28°C in Bengaluru."          # markdown and code blocks stripped
    assert progress == ["Running: df -h /", "Searching: weather bengaluru"]
    assert b.session == "sess-1"
    args = oneshot()[0]
    assert args[:2] == ["-p", "weather?"] and "--resume" not in args
    assert args[args.index("--model") + 1] == "sonnet"
    assert "WebSearch" in args[args.index("--allowedTools") + 1]
    assert "--include-partial-messages" in args
    assert b.last["mode"] == "oneshot" and b.last["first_token_ms"] is not None


async def test_follow_up_resumes_the_session(oneshot, monkeypatch):
    b, *_ = await run("answer", monkeypatch)
    await run("answer", monkeypatch, b, "and tomorrow?")
    second = oneshot()[1]
    assert second[second.index("--resume") + 1] == "sess-1"
    b.forget()
    await run("answer", monkeypatch, b)
    assert "--resume" not in oneshot()[2]


async def test_permission_denied_is_reported(oneshot, monkeypatch):
    _, ok, reply, _ = await run("denied", monkeypatch)
    assert reply == "Not allowed: Running: rm -rf build/"


async def test_claude_error(oneshot, monkeypatch):
    _, ok, reply, _ = await run("error", monkeypatch)
    assert not ok and reply == "Rate limited"


async def test_claude_crash_reports_stderr(oneshot, monkeypatch):
    _, ok, reply, _ = await run("crash", monkeypatch)
    assert not ok and "not logged in" in reply


async def test_timeout(oneshot, monkeypatch):
    monkeypatch.setattr(brain, "TIMEOUT_S", 0.5)
    _, ok, reply, _ = await run("slow", monkeypatch)
    assert not ok and reply == "Took too long, stopped."


async def test_cancel_sets_the_flag(oneshot, monkeypatch):
    monkeypatch.setenv("FAKE_CLAUDE_MODE", "slow")
    b = brain.Brain()
    task = asyncio.create_task(ask(b, "long job"))
    await until(lambda: b.alive)
    assert b.busy
    b.cancel()
    await asyncio.wait_for(task, 5)
    assert b.cancelled and not b.busy


async def test_switching_to_oneshot_stops_the_session(fake_claude, brains):
    b = brains()
    await ask(b, "hi")
    session_proc = b.proc
    set_config(brain_mode="oneshot")
    ok, _, _ = await ask(b, "hi")
    assert ok and session_proc.returncode is not None
    assert fake_claude()[1][:2] == ["-p", "hi"]


async def test_missing_claude(home, monkeypatch):
    monkeypatch.setattr(config, "claude_path", lambda cfg: None)
    ok, reply = await brain.Brain().ask("hi", None)
    assert not ok and "not found" in reply


# --- session mode (the default) ---------------------------------------------------------

async def test_session_starts_once_and_is_reused(fake_claude, brains):
    b = brains()
    ok, reply, c = await ask(b, "hello")
    assert ok and reply == "You said: hello."
    assert c.texts[-1] == "You said: hello."       # streamed as it was written
    assert len(c.texts) > 2
    ok, reply, _ = await ask(b, "again")
    assert ok and reply == "You said: again."
    starts = fake_claude()
    assert len(starts) == 1                          # one process for both requests
    args = starts[0]
    assert args[:3] == ["-p", "--input-format", "stream-json"]
    for flag in ("--output-format", "--verbose", "--include-partial-messages", "--model", "--allowedTools"):
        assert flag in args
    assert "--resume" not in args
    msgs = fake_claude.messages()
    assert [m["message"] for m in msgs] == [{"role": "user", "content": "hello"}, {"role": "user", "content": "again"}]
    assert msgs[0]["type"] == "user" and msgs[0]["parent_tool_use_id"] is None
    assert b.last["warm"] and b.last["mode"] == "session" and b.last["first_token_ms"] is not None
    assert b.status()["avg"]["session"]["n"] == 2
    assert b.session.startswith("sess-") and b.pid


async def test_preamble_before_a_tool_is_withdrawn(fake_claude, brains):
    b = brains()
    ok, reply, c = await ask(b, "chatter")
    assert ok and reply == "It's 28°C in Bengaluru. Sunny all day."
    assert c.progress == ["Searching: weather bengaluru"]
    i = c.texts.index("Let me check the weather.")
    assert c.texts[i + 1] == ""                      # withdrawn when the tool call started
    assert c.texts[-1] == reply


async def test_busy_rejects_a_second_request(fake_claude, brains):
    b = brains()
    first = asyncio.create_task(ask(b, "slow"))
    await until(lambda: b.busy and b.turn.text)
    ok, reply, _ = await ask(b, "another")
    assert not ok and "Still working" in reply
    b.cancel()
    await asyncio.wait_for(first, 5)


async def test_cancel_interrupts_and_keeps_the_process(fake_claude, brains):
    b = brains()
    task = asyncio.create_task(ask(b, "slow"))
    await until(lambda: b.busy and b.turn.text == "One moment")
    pid = b.pid
    t = time.monotonic()
    b.cancel()
    ok, _, _ = await asyncio.wait_for(task, 5)
    assert not ok and b.cancelled and not b.busy
    assert time.monotonic() - t < 1.5                # the interrupt ended the turn, no kill needed
    assert b.pid == pid                              # same process, still warm
    ok, reply, _ = await ask(b, "next")
    assert ok and reply == "You said: next." and b.last["warm"]
    assert len(fake_claude()) == 1


async def test_cancel_kills_when_the_interrupt_is_ignored(fake_claude, brains, monkeypatch):
    monkeypatch.setenv("FAKE_IGNORE_INTERRUPT", "1")
    monkeypatch.setattr(brain, "INTERRUPT_WAIT_S", 0.3)
    b = brains()
    await ask(b, "hi")
    sid = b.session
    task = asyncio.create_task(ask(b, "slow"))
    await until(lambda: b.busy and b.turn.text)
    b.cancel()
    ok, _, _ = await asyncio.wait_for(task, 5)
    assert not ok and b.cancelled and not b.alive
    ok, _, _ = await ask(b, "after")                 # a new process, same conversation
    assert ok
    second = fake_claude()[1]
    assert second[second.index("--resume") + 1] == sid


async def test_crash_mid_request_then_resume(fake_claude, brains):
    b = brains()
    await ask(b, "hi")
    sid = b.session
    ok, reply, _ = await ask(b, "die")
    assert not ok and "something broke" in reply
    assert not b.alive
    ok, reply, _ = await ask(b, "still there?")
    assert ok
    assert fake_claude()[1][fake_claude()[1].index("--resume") + 1] == sid
    assert len(fake_claude()) == 2                   # no retry of a request that crashed it


async def test_died_while_idle_is_restarted_with_resume(fake_claude, brains):
    b = brains()
    await ask(b, "hi")
    sid = b.session
    b.proc.kill()                                    # dies between requests (not noticed yet)
    ok, reply, _ = await ask(b, "you ok?")
    assert ok and reply == "You said: you ok?."
    starts = fake_claude()
    assert len(starts) == 2 and starts[1][starts[1].index("--resume") + 1] == sid


async def test_crash_at_start_is_reported(fake_claude, brains, monkeypatch):
    monkeypatch.setenv("FAKE_CLAUDE_MODE", "crash")
    b = brains()
    ok, reply, _ = await ask(b, "hi")
    assert not ok and "not logged in" in reply
    assert len(fake_claude()) == 1


async def test_idle_restart_starts_a_new_conversation(fake_claude, brains):
    set_config(brain_idle_min=0.005)                 # 0.3 s
    b = brains()
    await ask(b, "hi")
    assert b.alive and b.session
    await until(lambda: not b.alive, 3)
    assert b.session is None
    await ask(b, "hi again")
    assert len(fake_claude()) == 2 and "--resume" not in fake_claude()[1]


async def test_settings_change_restarts_with_resume(fake_claude, brains):
    b = brains()
    await ask(b, "hi")
    sid = b.session
    set_config(model="opus")
    ok, _, _ = await ask(b, "hi")
    assert ok
    second = fake_claude()[1]
    assert second[second.index("--model") + 1] == "opus"
    assert second[second.index("--resume") + 1] == sid


async def test_forget_starts_a_new_process(fake_claude, brains):
    b = brains()
    await ask(b, "hi")
    b.forget()
    await until(lambda: not b.alive)
    await ask(b, "hi")
    assert len(fake_claude()) == 2 and "--resume" not in fake_claude()[1]


async def test_prewarm(fake_claude, brains):
    b = brains()
    await b.prewarm()
    assert b.alive
    await until(lambda: len(fake_claude()) == 1)
    await b.prewarm()                                # once
    ok, _, _ = await ask(b, "hi")
    assert ok and b.last["warm"] and len(fake_claude()) == 1


async def test_prewarm_off_or_oneshot(fake_claude, brains):
    b = brains()
    set_config(brain_prewarm=False)
    await b.prewarm()
    set_config(brain_prewarm=True, brain_mode="oneshot")
    await b.prewarm()
    assert not b.alive and fake_claude() == []


async def test_session_timeout(fake_claude, brains, monkeypatch):
    monkeypatch.setenv("FAKE_IGNORE_INTERRUPT", "1")
    monkeypatch.setattr(brain, "TIMEOUT_S", 0.5)
    b = brains()
    ok, reply, _ = await ask(b, "slow")
    assert not ok and reply == "Took too long, stopped." and not b.alive


# --- the stick tools (MCP) -------------------------------------------------------------

async def test_session_gets_the_stick_tools(fake_claude, brains):
    b = brains()
    await ask(b, "hello")
    args = fake_claude()[0]
    path = Path(args[args.index("--mcp-config") + 1])
    assert path == config.HOME / "mcp-voice.json" and oct(path.stat().st_mode & 0o777) == "0o600"
    server = json.loads(path.read_text())["mcpServers"]["stick"]
    assert server["type"] == "http" and server["url"] == f"http://127.0.0.1:{config.PORT}/mcp"
    assert server["headers"] == {"Authorization": f"Bearer {config.token()}", "X-Stick-Client": "voice"}
    allowed = args[args.index("--allowedTools") + 1].split(",")
    assert {"WebSearch", "mcp__stick__notify", "mcp__stick__ask", "mcp__stick__celebrate", "mcp__stick__progress",
            "mcp__stick__pin_home", "mcp__stick__stick_status", "mcp__stick__speak"} <= set(allowed)
    for changes_the_mac in ("open_url", "open_app", "quick_command"):  # left to the permission prompt
        assert f"mcp__stick__{changes_the_mac}" not in allowed
    assert args.index("--mcp-config") < args.index("--allowedTools")  # (both take several values)


async def test_stick_tools_off_restarts_without_them(fake_claude, brains):
    b = brains()
    await ask(b, "hello")
    set_config(mcp_server=False)
    await ask(b, "again")
    starts = fake_claude()
    assert len(starts) == 2 and "--mcp-config" not in starts[1]
    assert "mcp__stick" not in starts[1][starts[1].index("--allowedTools") + 1]


# --- helpers ---------------------------------------------------------------------------

def test_describe_and_plain():
    assert brain.describe_tool("WebFetch", {"url": "https://example.com/a"}) == "Reading example.com/a"
    assert brain.describe_tool("Read", {"file_path": "/x/y"}) == "Read: /x/y"
    assert brain.describe_tool("Mystery", {}) == "Mystery"
    long = "word " * 100
    out = brain.plain(long)
    assert len(out) <= 280 and out.endswith("…")
    assert brain.plain("Here:\n```py\nprint(1)") == "Here:"   # a code block still being written


@pytest.mark.parametrize("text, reset, rest", [
    ("New chat.", True, ""),
    ("new conversation", True, ""),
    ("Start over.", True, ""),
    ("Let's start over, what's the weather in Paris?", True, "what's the weather in Paris?"),
    ("Start a new chat please", True, ""),
    ("Forget that.", True, ""),
    ("new chat in Slack", False, "new chat in Slack"),
    ("What's new?", False, "What's new?"),
    ("how do I start over in git", False, "how do I start over in git"),
])
def test_split_new_chat(text, reset, rest):
    assert brain.split_new_chat(text) == (reset, rest)


# --- the real thing (opt-in) ------------------------------------------------------------

@pytest.mark.slow
@pytest.mark.skipif(not os.environ.get("STICK_AGENT_REAL_CLAUDE"), reason="STICK_AGENT_REAL_CLAUDE=1 (your login)")
async def test_real_claude_session(home, brains):
    """Two tiny requests through the real Claude Code: the second one is warm."""
    set_config(claude_path="")
    b = brains()
    ok, reply, c = await ask(b, "say hi in 3 words")
    assert ok and reply and c.texts
    ok, reply, _ = await ask(b, "say bye in 3 words")
    assert ok and b.last["warm"]
    print("timings:", b.status())
