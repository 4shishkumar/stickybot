"""Claude Code runner: stream-json parsing, follow-ups, timeouts, cancel."""
import asyncio
import json
import sys
from pathlib import Path

import pytest

from stick_agent import brain, config

FAKE = Path(__file__).parent / "fake_claude.py"


@pytest.fixture
def fake_claude(home, tmp_path, monkeypatch):
    """Points the agent at fake_claude.py (via a tiny wrapper so it runs with this Python)."""
    wrapper = tmp_path / "claude"
    wrapper.write_text(f"#!/bin/sh\nexec {sys.executable} {FAKE} \"$@\"\n")
    wrapper.chmod(0o755)
    log = tmp_path / "claude_calls.jsonl"
    monkeypatch.setenv("FAKE_CLAUDE_LOG", str(log))
    cfg = config.load()
    cfg["claude_path"] = str(wrapper)
    config.save(cfg)

    def calls():
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
    return calls


async def run(mode, monkeypatch, b=None, text="weather?"):
    monkeypatch.setenv("FAKE_CLAUDE_MODE", mode)
    b = b or brain.Brain()
    progress = []

    async def on_progress(line):
        progress.append(line)
    ok, reply = await b.ask(text, on_progress)
    return b, ok, reply, progress


async def test_answer_progress_and_plain_text(fake_claude, monkeypatch):
    b, ok, reply, progress = await run("answer", monkeypatch)
    assert ok
    assert reply == "It's 28°C in Bengaluru."          # markdown and code blocks stripped
    assert progress == ["Running: df -h /", "Searching: weather bengaluru"]
    assert b.session == "sess-1"
    args = fake_claude()[0]
    assert args[:2] == ["-p", "weather?"] and "--resume" not in args
    assert args[args.index("--model") + 1] == "sonnet"
    assert "WebSearch" in args[args.index("--allowedTools") + 1]


async def test_follow_up_resumes_the_session(fake_claude, monkeypatch):
    b, *_ = await run("answer", monkeypatch)
    await run("answer", monkeypatch, b, "and tomorrow?")
    second = fake_claude()[1]
    assert second[second.index("--resume") + 1] == "sess-1"
    b.forget()
    await run("answer", monkeypatch, b)
    assert "--resume" not in fake_claude()[2]


async def test_permission_denied_is_reported(fake_claude, monkeypatch):
    _, ok, reply, _ = await run("denied", monkeypatch)
    assert reply == "Not allowed: Running: rm -rf build/"


async def test_claude_error(fake_claude, monkeypatch):
    _, ok, reply, _ = await run("error", monkeypatch)
    assert not ok and reply == "Rate limited"


async def test_claude_crash_reports_stderr(fake_claude, monkeypatch):
    _, ok, reply, _ = await run("crash", monkeypatch)
    assert not ok and "not logged in" in reply


async def test_timeout(fake_claude, monkeypatch):
    monkeypatch.setattr(brain, "TIMEOUT_S", 0.5)
    _, ok, reply, _ = await run("slow", monkeypatch)
    assert not ok and reply == "Took too long, stopped."


async def test_cancel_sets_the_flag(fake_claude, monkeypatch):
    monkeypatch.setenv("FAKE_CLAUDE_MODE", "slow")
    b = brain.Brain()

    async def progress(_):
        pass
    task = asyncio.create_task(b.ask("long job", progress))
    for _ in range(100):
        if b.busy:
            break
        await asyncio.sleep(0.02)
    assert b.busy
    b.cancel()
    await asyncio.wait_for(task, 5)
    assert b.cancelled and not b.busy


async def test_missing_claude(home, monkeypatch):
    monkeypatch.setattr(config, "claude_path", lambda cfg: None)
    ok, reply = await brain.Brain().ask("hi", None)
    assert not ok and "not found" in reply


def test_describe_and_plain():
    assert brain.describe_tool("WebFetch", {"url": "https://example.com/a"}) == "Reading example.com/a"
    assert brain.describe_tool("Read", {"file_path": "/x/y"}) == "Read: /x/y"
    assert brain.describe_tool("Mystery", {}) == "Mystery"
    long = "word " * 100
    out = brain.plain(long)
    assert len(out) <= 280 and out.endswith("…")
