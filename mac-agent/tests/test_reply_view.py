"""The stick's reply view (firmware 2.6.0): the whole answer sent in parts after "done", the
"listen" shortcut ({"evt":"speak"}) reading the last reply aloud with "speech" on / off back to
the stick, and the agent's "bye" when it pauses or quits."""
import asyncio

import numpy as np

from stick_agent import clips, core, link, speech
from tests.conftest import FakeLink

LONG = ("## Release notes\n\nHere's what changed this week:\n\n"
        "- **Login**: the redirect bug is fixed, the session cookie survives the round trip.\n"
        "- Dark mode landed behind the `theme` toggle.\n\n"
        "```sh\nnpm run build\n```\n\n"
        + "Still open: the billing refactor needs a review, and the CI cache keeps missing. " * 8
        + "It's 28°C — nice.")


def fake_voice(agent, secs=0.0):
    """No `say`, no audio: what was made and played."""
    made, played = [], []

    async def synth(sentence, procs):
        made.append(sentence)
        return {"pcm": np.zeros(int(clips.RATE * secs) or 160, dtype=np.int16), "text": sentence}

    async def play(batch, procs):
        played.append([c["text"] for c in batch])
    agent.speaker.synth, agent.speaker.play = synth, play
    return made, played


# --- the text ------------------------------------------------------------------------------------

def test_reader_text_keeps_paragraphs_and_drops_markdown():
    t = core.reader_text(LONG)
    assert t.startswith("Release notes\n\nHere's what changed this week:\n\n• Login: the redirect")
    assert "• Dark mode landed behind the theme toggle." in t
    assert "npm run build" in t and "```" not in t and "**" not in t and "##" not in t
    assert "\n\n\n" not in t
    assert "28°C — nice." in t  # the stick's own symbols stay


def test_reader_text_is_capped_without_breaking_a_character():
    t = core.reader_text("°" * 5000)
    assert len(t.encode()) <= core.READER_MAX
    assert t.endswith("…") and t.encode().decode() == t
    assert core.reader_text("short") == "short"


def test_reply_parts_fit_and_join_back():
    t = core.reader_text(LONG * 3)
    parts = core.reply_parts(t)
    assert "".join(parts) == t
    assert all(len(p.encode()) <= core.PART_BYTES for p in parts)
    assert core.reply_parts("—" * 200, size=10) == ["———"] * 66 + ["——"]  # 3 bytes each, never split
    assert core.reply_parts("") == []


# --- "done" + the parts ------------------------------------------------------------------------------

async def test_a_long_answer_follows_done_in_parts(agent):
    await agent.tell_stick("done", "what changed", LONG)
    done, *rest = agent.link.sent
    assert done["state"] == "done" and len(done["text"]) == 300
    assert done["parts"] == len(rest) >= 2 and done["rid"] == 1
    assert [m["reply"]["i"] for m in rest] == list(range(len(rest)))
    assert all(m["src"] == "agent" and m["reply"]["rid"] == 1 for m in rest)
    assert "".join(m["reply"]["t"] for m in rest) == core.reader_text(LONG)
    assert agent.last_done == LONG
    agent.link.sent.clear()
    await agent.tell_stick("done", "again", LONG)
    assert agent.link.sent[0]["rid"] == 2  # a new id each time


async def test_a_short_answer_is_just_done(agent):
    await agent.tell_stick("done", "weather", "It's 28°C.")
    assert agent.link.sent == [{"src": "agent", "state": "done", "heard": "weather", "text": "It's 28°C."}]
    await agent.tell_stick("working", "x", LONG)  # only "done" carries the full reply
    assert "parts" not in agent.link.sent[-1] and len(agent.link.sent) == 2


def test_the_hello_says_we_read_replies_aloud(agent):
    assert agent._hello_extra()["speak"] is True


# --- listen ---------------------------------------------------------------------------------------

async def until(cond, s=2.0):
    for _ in range(int(s / 0.01)):
        if cond():
            return True
        await asyncio.sleep(0.01)
    return False


def speech_msgs(agent):
    return [m["speech"] for m in agent.link.sent if "speech" in m]


async def test_listen_reads_the_last_reply_and_says_when_it_ends(agent):
    made, played = fake_voice(agent)
    await agent.tell_stick("done", "weather", "It's 28°C in Bangalore. Rain later. Take an umbrella.")
    agent._on_message({"src": "agent", "evt": "speak", "what": "last"})
    assert await until(lambda: speech_msgs(agent) == ["on", "off"])
    assert made == ["It's 28°C in Bangalore.", "Rain later.", "Take an umbrella."]  # sentence by sentence
    assert [s for b in played for s in b] == made


async def test_listen_again_stops_it(agent):
    made, played = fake_voice(agent)
    real = agent.speaker.play

    async def slow(batch, procs):
        await real(batch, procs)
        await asyncio.sleep(5)
    agent.speaker.play = slow
    await agent.tell_stick("done", "x", "One. Two.")
    agent._on_message({"src": "agent", "evt": "speak", "what": "last"})
    assert await until(lambda: played)
    agent._on_message({"src": "agent", "evt": "speak", "what": "stop"})
    assert await until(lambda: speech_msgs(agent) == ["on", "off"])
    assert agent.speaker.current is None


async def test_listen_without_a_reply_says_off(agent):
    fake_voice(agent)
    agent._on_message({"src": "agent", "evt": "speak", "what": "last"})
    assert await until(lambda: speech_msgs(agent) == ["off"])


async def test_listen_reads_a_long_reply_whole(agent):
    """speak_max_words caps the automatic reading, not the one asked for."""
    made, _ = fake_voice(agent)
    agent.cfg["speak_max_words"] = 5
    await agent.tell_stick("done", "x", LONG)
    agent._on_message({"src": "agent", "evt": "speak", "what": "last"})
    assert await until(lambda: speech_msgs(agent) == ["on", "off"], 5)
    assert sum(len(s.split()) for s in made) > 100
    assert not any("npm run build" in s for s in made)  # no code read out


# --- the speaker -------------------------------------------------------------------------------------

async def test_a_new_utterance_doesnt_say_off_in_between():
    seen = []
    sp = speech.Speaker(lambda: FakeLink(), lambda: "stick", lambda: "", on_speech=seen.append)

    async def synth(sentence, procs):
        return {"pcm": np.zeros(160, dtype=np.int16), "text": sentence}

    async def play(batch, procs):
        await asyncio.sleep(0.05)
    sp.synth, sp.play = synth, play
    sp.say("Old one.")
    new = sp.say("New one.")
    await asyncio.wait_for(new.wait(), 2)
    await asyncio.sleep(0.01)
    assert seen == [True, False]
    sp.stop()  # nothing playing: nothing more to say
    assert seen == [True, False]


async def test_clips_are_joined_only_up_to_the_sticks_buffer():
    played = []
    sp = speech.Speaker(lambda: FakeLink(), lambda: "stick", lambda: "")

    async def synth(sentence, procs):
        return {"pcm": np.zeros(clips.RATE * 12, dtype=np.int16), "text": sentence}  # 12 s each

    async def play(batch, procs):
        played.append([c["text"] for c in batch])
        await asyncio.sleep(0.05)
    sp.synth, sp.play = synth, play
    u = sp.start()
    for s in "ABCDEF":
        u.add(s)
    u.close()
    await asyncio.wait_for(u.wait(), 3)
    assert [s for b in played for s in b] == list("ABCDEF")  # nothing lost or cut
    assert all(len(b) <= 2 for b in played)  # 2 x 12 s fit in 30 s, 3 don't


# --- bye -------------------------------------------------------------------------------------

class Client:
    def __init__(self):
        self.parts, self.disconnected = [], False

    async def write_gatt_char(self, char, data, response):
        self.parts.append(bytes(data))

    async def disconnect(self):
        self.disconnected = True

    @property
    def is_connected(self):
        return not self.disconnected


async def test_pause_says_bye_before_disconnecting():
    lk = link.Link(lambda m: None, lambda a: None, lambda s: None)
    c = lk.client = Client()
    lk.pause(True)
    assert await until(lambda: c.disconnected)
    assert b"".join(c.parts) == b'{"src":"agent","bye":true}\n'


async def test_a_new_stick_gets_the_bye_too():
    lk = link.Link(lambda m: None, lambda a: None, lambda s: None)
    c = lk.client = Client()
    lk.set_target({"address": "OTHER", "name": "y"})
    assert await until(lambda: c.disconnected)
    assert b"bye" in b"".join(c.parts)


async def test_bye_on_a_stuck_link_gives_up_quickly():
    lk = link.Link(lambda m: None, lambda a: None, lambda s: None)

    class Stuck(Client):
        async def write_gatt_char(self, char, data, response):
            await asyncio.sleep(10)
    lk.client = Stuck()
    assert await lk.bye(timeout=0.05) is False


async def test_quitting_says_bye(agent):
    agent.loop = asyncio.get_running_loop()
    agent.shutdown()
    assert await until(lambda: {"src": "agent", "bye": True} in agent.link.sent)


def test_old_sticks_ignore_the_new_keys():
    """Firmware before 2.6.0 takes {"bye"} / {"reply"} / {"speech"} for a hello (no "state"): they
    change nothing on its screen; the "done" before the parts carries the first 300 characters as before."""
    for m in (link.BYE, {"src": "agent", "reply": {"rid": 1, "i": 0, "t": "x"}}, {"src": "agent", "speech": "on"}):
        assert "state" not in m and m["src"] == "agent"
