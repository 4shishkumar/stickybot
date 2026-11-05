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

    async def ask(self, text, on_progress, on_text=None):
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

    async def ask(text, on_progress, on_text=None):
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
    task = asyncio.create_task(agent.ask_permission("Bash", "rm -rf build/", "web", wait=5))
    await asyncio.sleep(0.05)
    prompt = next(m for m in agent.link.sent if m.get("src") == "cli" and m.get("prompt"))
    pid = prompt["prompt"]["id"]
    assert prompt["prompt"]["tool"] == "Bash" and pid in agent.prompts
    agent._on_message({"cmd": "permission", "id": pid, "decision": "once"})
    assert await task == "allow"
    assert agent.link.sent[-1] == {"src": "cli"}   # cleared on the stick
    assert agent.prompts == {}


async def test_permission_denied_from_the_dashboard(agent):
    task = asyncio.create_task(agent.ask_permission("Edit", "/etc/hosts", "web", wait=5))
    await asyncio.sleep(0.05)
    agent.decide(next(iter(agent.prompts)), allow=False)
    assert await task == "deny"


async def test_permission_nobody_answers(agent, monkeypatch):
    monkeypatch.setattr(core, "PROMPT_RESEND_S", 0.05)
    assert await agent.ask_permission("Bash", "ls", "web", wait=0.2) is None


async def test_permission_turned_off(agent):
    agent.cfg["stick_approvals"] = False
    assert await agent.ask_permission("Bash", "ls", "web") is None
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


# --- streaming the reply (both brain modes stream) -----------------------------------------

class StreamingBrain(FakeBrain):
    """Writes its answer in chunks: text is appended, None withdraws it (a preamble), "TOOL:x" is a tool line."""

    def __init__(self, chunks, pause=0.0, ok=True):
        super().__init__(ok=ok)
        self.chunks, self.pause = chunks, pause
        self.forgot = False

    async def ask(self, text, on_progress, on_text=None):
        self.asked.append(text)
        acc = ""
        for c in self.chunks:
            if c is None:
                acc = ""
                await on_text("")
            elif c.startswith("TOOL:"):
                await on_progress(c[5:])
            else:
                acc += c
                await on_text(acc)
            await asyncio.sleep(self.pause)
            if self.cancelled:
                return False, "Stopped."
        return self.ok, acc.strip()

    def forget(self):
        self.forgot = True


@pytest.fixture
def fake_voice(agent):
    """The agent's speaker with fake `say` and playback: records what's made and played."""
    log = {"made": [], "played": [], "sent_before": []}

    async def synth(sentence, procs):
        log["made"].append(sentence)
        log["sent_before"].append(len(agent.link.states()))  # how far the stick screen had got
        return {"pcm": np.zeros(160, dtype=np.int16), "text": sentence}

    async def play(batch, procs):
        log["played"] += [c["text"] for c in batch]
        await asyncio.sleep(0.01)
    agent.speaker.synth, agent.speaker.play = synth, play
    agent.cfg.update(speak_replies=True, speak_max_words=40)
    return log


async def test_reply_streams_to_the_stick_throttled(agent):
    agent.cfg["stream_interval_ms"] = 100
    agent.brain = StreamingBrain([f"w{i} " for i in range(40)], pause=0.01)   # ~0.4 s of writing
    await agent.handle_text("count")
    st = agent.link.states()
    working = [t for s, t in st if s == "working"]
    assert st[0] == ("thinking", "") and st[-1] == ("done", " ".join(f"w{i}" for i in range(40)))
    assert 2 <= len(working) <= 7                      # ~3-4 a second, not 40
    assert all(len(a) <= len(b) for a, b in zip(working, working[1:], strict=False))  # only grows
    assert agent.last_reply["text"].startswith("w0 w1")
    history = [json.loads(line) for line in config.HISTORY.read_text().splitlines()]
    assert history[-1]["reply"] == agent.last_reply["text"]


async def test_streaming_off_shows_only_tool_lines(agent):
    agent.cfg["stream_replies"] = False
    agent.brain = StreamingBrain(["TOOL:Searching: x", "It's ", "sunny."])
    await agent.handle_text("weather")
    assert agent.link.states() == [("thinking", ""), ("working", "Searching: x"), ("done", "It's sunny.")]


async def test_stick_text_limits_hold_while_streaming(agent):
    agent.cfg["stream_interval_ms"] = 100
    agent.brain = StreamingBrain(["Café ✨ **bold** " + "word " * 120], pause=0)
    await agent.handle_text("long")
    for m in agent.link.sent:
        if m.get("state") == "working":
            assert len(m["text"]) <= 300
            assert all(ch.isascii() or ch in core.STICK_EXTRA for ch in m["text"])
            assert "*" not in m["text"]


async def test_streamed_speech_sentence_by_sentence(agent, fake_voice):
    agent.brain = StreamingBrain(["It's 28°C. ", "Sunny ", "today. ", "Bring a hat."], pause=0.02)
    await agent.handle_text("weather")
    await asyncio.wait_for(agent.speaker.current.wait(), 2)
    assert fake_voice["made"] == ["It's 28°C.", "Sunny today.", "Bring a hat."]
    assert fake_voice["played"] == fake_voice["made"]
    done_at = [s for s, _ in agent.link.states()].index("done")
    assert fake_voice["sent_before"][0] <= done_at     # speaking began before "done" went to the stick
    assert any(e["kind"] == "spoken" and e["text"].startswith("It's 28") for e in agent.events)


async def test_preamble_is_not_spoken(agent, fake_voice):
    agent.brain = StreamingBrain(["Let me check. ", None, "TOOL:Searching: weather", "It's sunny. ", "Warm."])
    await agent.handle_text("weather")
    await asyncio.wait_for(agent.speaker.current.wait(), 2)
    assert fake_voice["made"] == ["It's sunny.", "Warm."]
    assert ("working", "Searching: weather") in agent.link.states()


async def test_long_replies_are_spoken_up_to_the_word_limit(agent, fake_voice):
    agent.cfg["speak_max_words"] = 6
    agent.brain = StreamingBrain(["One two three. ", "Four five. ", "Six seven eight. ", "Nine."])
    await agent.handle_text("count")
    await asyncio.wait_for(agent.speaker.current.wait(), 2)
    assert fake_voice["made"] == ["One two three.", "Four five."]


async def test_cancel_stops_the_stream_and_the_speech(agent, fake_voice):
    agent.brain = StreamingBrain([f"Sentence {i}. " for i in range(50)], pause=0.02)
    task = asyncio.create_task(agent.handle_text("long job"))
    await asyncio.sleep(0.2)
    utter = agent.speaker.current
    agent._on_message({"src": "agent", "evt": "cancel"})
    await asyncio.wait_for(task, 2)
    assert utter.cancelled and agent.speaker.current is None
    assert "done" not in [s for s, _ in agent.link.states()]
    made = len(fake_voice["made"])
    await asyncio.sleep(0.1)
    assert len(fake_voice["made"]) == made and made < 50


async def test_talking_again_stops_the_speech(agent, fake_voice):
    agent.brain = StreamingBrain(["One. ", "Two. ", "Three."])
    await agent.handle_text("count")
    utter = agent.speaker.current
    agent._on_message({"src": "agent", "evt": "talk", "state": "start"})
    assert utter.cancelled


async def test_failed_request_speaks_the_error_in_one_go(agent, fake_voice):
    agent.brain = StreamingBrain([], ok=False)

    async def fail(text, on_progress, on_text=None):
        return False, "Claude Code failed."
    agent.brain.ask = fail
    await agent.handle_text("hi")
    await asyncio.wait_for(agent.speaker.current.wait(), 2)
    assert fake_voice["made"] == ["Claude Code failed."]


@pytest.mark.parametrize("said, asked", [("Start over.", []), ("New chat. Weather?", ["Weather?"])])
async def test_new_chat_phrase(agent, said, asked):
    agent.brain = StreamingBrain(["It's sunny."])
    await agent.handle_text(said)
    assert agent.brain.forgot and agent.brain.asked == asked
    if not asked:
        assert agent.link.states() == [("done", "OK, new chat.")]


async def test_connecting_prewarms_the_brain(agent):
    warmed = []

    async def prewarm():
        warmed.append(1)
    agent.brain.prewarm = prewarm
    agent._on_link("connected")
    await asyncio.sleep(0.01)
    assert warmed == [1]


def test_status_reports_the_brain(agent):
    from stick_agent import brain
    agent.brain = brain.Brain()
    st = agent.status()["brain"]
    assert st["mode"] == "session" and st["alive"] is False and st["last"] is None
