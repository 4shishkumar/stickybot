"""The agent's talk flow, end to end, with a fake stick and fake speech-to-text."""
import asyncio
import json

import numpy as np
import pytest

from stick_agent import adpcm, commands, config, core


def frames(seconds, loud=True):
    n = int(16000 * seconds)
    pcm = (8000 * np.sin(np.arange(n) / 3)).astype(np.int16) if loud else np.zeros(n, dtype=np.int16)
    return adpcm.encode(pcm.tolist())


async def talk(agent, seconds):
    """Simulates holding B for `seconds`: talk start, audio frames, talk stop."""
    agent._on_message({"src": "agent", "evt": "talk", "state": "start"})
    for f in frames(seconds):
        agent._on_audio(f)
    agent._on_message({"src": "agent", "evt": "talk", "state": "stop"})
    for _ in range(200):  # until the agent has told the stick how it ended
        st = agent.link.states()
        if st and st[-1][0] in ("done", "error", "cancelled"):
            return st
        await asyncio.sleep(0.01)
    return agent.link.states()


@pytest.fixture
def pr_command(home, monkeypatch):
    config.COMMANDS.write_text(json.dumps({"commands": [
        {"name": "GitHub PR", "phrases": ["pr {n}", "open pr {n}"], "open_url": "https://github.com/acme/app/pull/{n}"}]}))
    opened = []
    monkeypatch.setattr(commands.subprocess, "run", lambda args, **kw: opened.append(args))
    return opened


class FakeBrain:
    def __init__(self, ok=True, reply="It's 28°C.", progress=()):
        self.ok, self.reply, self.prog = ok, reply, list(progress)
        self.busy = False
        self.cancelled = False
        self.asked = []
        self.proc = None

    async def ask(self, text, on_progress):
        self.asked.append(text)
        for p in self.prog:
            await on_progress(p)
        return self.ok, self.reply

    def cancel(self):
        self.cancelled = True

    def forget(self):
        pass


async def test_quick_press_without_words_closes_quietly(agent):
    agent.stt.text = ""
    assert await talk(agent, 1.0) == [("cancelled", "")]
    assert agent.state == "ready"


async def test_too_short(agent):
    assert await talk(agent, 0.3) == [("error", "Too short - hold B while you speak.")]
    assert agent.stt.calls == 0


async def test_long_silence_says_didnt_catch_that(agent):
    agent.stt.text = ""
    assert await talk(agent, 3.0) == [("error", "Didn't catch that.")]


async def test_transcription_crash_still_answers_the_stick(agent):
    agent.stt.text = RuntimeError("There is no Stream(cpu, 1) in current thread.")
    st = await talk(agent, 1.5)
    assert st[-1][0] == "error" and "Something went wrong" in st[-1][1]
    assert agent.state == "ready"


async def test_model_not_loaded_yet(agent):
    agent.stt_ready = False
    st = await talk(agent, 1.5)
    assert st == [("error", "Speech model still loading, try again in a moment.")]


async def test_quick_command(agent, pr_command):
    agent.stt.text = "Open PR twelve thirty four."
    st = await talk(agent, 1.5)
    assert st == [("done", "Opened GitHub PR #1234")]
    assert pr_command == [["open", "https://github.com/acme/app/pull/1234"]]
    history = [json.loads(line) for line in config.HISTORY.read_text().splitlines()]
    assert history[-1]["via"] == "quick" and history[-1]["command"] == "GitHub PR"
    assert agent.stats["quick"] == 1


async def test_claude_request(agent):
    agent.brain = FakeBrain(progress=["Searching: weather"])
    agent.stt.text = "what's the weather"
    st = await talk(agent, 2.0)
    assert st == [("thinking", ""), ("working", "Searching: weather"), ("done", "It's 28°C.")]
    assert agent.brain.asked == ["what's the weather"]
    assert agent.last_reply["text"] == "It's 28°C."


async def test_claude_failure_is_an_error(agent):
    agent.brain = FakeBrain(ok=False, reply="Claude Code failed.")
    await agent.handle_text("do the thing")
    assert agent.link.states()[-1] == ("error", "Claude Code failed.")
    assert agent.last_reply is None


async def test_busy_brain(agent):
    agent.brain = FakeBrain()
    agent.brain.busy = True
    await agent.handle_text("another one")
    assert agent.link.states() == [("error", "Still working on the last request.")]


async def test_cancel_from_the_stick(agent):
    b = FakeBrain()

    async def ask(text, on_progress):
        await asyncio.sleep(0.2)
        b.cancelled = True  # what the real Brain does after cancel()
        return False, "killed"
    b.ask = ask
    agent.brain = b
    task = asyncio.create_task(agent.handle_text("long job"))
    await asyncio.sleep(0.05)
    agent._on_message({"src": "agent", "evt": "cancel"})
    await task
    assert [s for s, _ in agent.link.states()] == ["thinking"]  # nothing after the cancel
    assert agent.state == "ready"


async def test_permission_answered_on_the_stick(agent):
    task = asyncio.create_task(agent.ask_permission("Bash", "rm -rf build/", "loom", wait=5))
    await asyncio.sleep(0.05)
    prompt = next(m for m in agent.link.sent if m.get("src") == "cli" and m.get("prompt"))
    pid = prompt["prompt"]["id"]
    assert prompt["prompt"]["tool"] == "Bash" and pid in agent.prompts
    agent._on_message({"cmd": "permission", "id": pid, "decision": "once"})
    assert await task == "allow"
    assert agent.link.sent[-1] == {"src": "cli"}   # cleared on the stick
    assert agent.prompts == {}


async def test_permission_denied_from_the_dashboard(agent):
    task = asyncio.create_task(agent.ask_permission("Edit", "/etc/hosts", "loom", wait=5))
    await asyncio.sleep(0.05)
    agent.decide(next(iter(agent.prompts)), allow=False)
    assert await task == "deny"


async def test_permission_nobody_answers(agent, monkeypatch):
    monkeypatch.setattr(core, "PROMPT_RESEND_S", 0.05)
    assert await agent.ask_permission("Bash", "ls", "loom", wait=0.2) is None


async def test_permission_turned_off(agent):
    agent.cfg["stick_approvals"] = False
    assert await agent.ask_permission("Bash", "ls", "loom") is None
    assert agent.link.sent == []


def test_hello_extra_has_a_safe_home_card(agent):
    agent.last_reply = {"t": __import__("time").time(), "text": "Café — ✨ naïve"}
    extra = agent._hello_extra()
    assert extra["home"]["k"] == "reply" and extra["home"]["b"] == "Cafe —  naive"
    assert all(ch.isascii() or ch in core.STICK_EXTRA for ch in extra["home"]["b"])


@pytest.mark.parametrize("raw, safe", [
    ("plain", "plain"),
    ("28°C — “quoted” …", "28°C — “quoted” …"),
    ("café naïve", "cafe naive"),
    ("emoji 😀 gone", "emoji  gone"),
    ("", ""),
    (None, ""),
])
def test_stick_text(raw, safe):
    assert core.stick_text(raw) == safe


def test_should_speak(agent):
    agent.cfg.update(speak_replies=True, speak_max_words=5, speak_quick=False)
    assert agent.should_speak("one two three")
    assert not agent.should_speak("one two three four five six")
    assert not agent.should_speak("Opened Slack.", quick=True)
    agent.cfg["speak_replies"] = False
    assert not agent.should_speak("hi")
