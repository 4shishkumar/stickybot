"""Speech: cutting a streamed reply into sentences, and the queue that makes and plays them."""
import asyncio

import numpy as np
import pytest

from stick_agent import clips, speech
from tests.conftest import FakeLink


def grow(text, step=3):
    """The reply as it streams in: every prefix, `step` characters at a time."""
    return [text[:i] for i in range(step, len(text) + step, step)]


def cut(text, step=3):
    s = speech.Sentences()
    out = []
    for part in grow(text, step):
        out += s.feed(part)
    return out, s.finish()


@pytest.mark.parametrize("step", [1, 3, 50])
def test_sentences_as_they_complete(step):
    done, rest = cut("It's 28°C in Bengaluru. Sunny all day! Want more? Bring a hat.", step)
    assert done == ["It's 28°C in Bengaluru.", "Sunny all day!", "Want more?"]
    assert rest == ["Bring a hat."]


def test_a_sentence_waits_for_the_next_one():
    s = speech.Sentences()
    assert s.feed("Let me check the weather.") == []   # might be a preamble before a tool call
    assert s.feed("Let me check the weather. It") == ["Let me check the weather."]


def test_no_split_inside_numbers_or_abbreviations():
    done, rest = cut("It costs 3.50 dollars, e.g. at Dr. Smith's shop. Version 2.1.3 is out. OK")
    assert done == ["It costs 3.50 dollars, e.g. at Dr. Smith's shop.", "Version 2.1.3 is out."]
    assert rest == ["OK"]


def test_restart_when_the_text_is_withdrawn():
    s = speech.Sentences()
    s.feed("Let me look. ")
    assert s.feed("") == []
    assert s.feed("It's sunny. And") == ["It's sunny."]
    assert s.finish() == ["And"]


def test_markdown_and_code_are_not_spoken():
    done, rest = cut("**Done.** Here's the command:\n```sh\nrm -rf x.\n```\n- First item\n- Second one")
    assert done == ["Done.", "Here's the command:", "First item"]
    assert rest == ["Second one"]
    assert speech.speakable("Text ```still open. more") == "Text "


def test_lines_are_boundaries():
    done, rest = cut("Three PRs need you\nPR 12 from Ana\nPR 13")
    assert done == ["Three PRs need you", "PR 12 from Ana"] and rest == ["PR 13"]


# --- the queue -----------------------------------------------------------------------------

class Rig:
    """A Speaker whose `say` and playback are fakes (no audio)."""

    def __init__(self, synth_s=0.0, play_s=0.0, output="stick"):
        self.link = FakeLink()
        self.sp = speech.Speaker(lambda: self.link, lambda: output, lambda: "")
        self.made, self.played = [], []
        self.synth_s, self.play_s = synth_s, play_s

        async def synth(sentence, procs):
            await asyncio.sleep(self.synth_s)
            self.made.append(sentence)
            return {"pcm": np.zeros(160, dtype=np.int16), "text": sentence}

        async def play(batch, procs):
            self.played.append([c["text"] for c in batch])
            await asyncio.sleep(self.play_s)
        self.sp.synth, self.sp.play = synth, play


async def test_plays_in_order_and_batches_what_is_ready():
    r = Rig(synth_s=0.01, play_s=0.15)
    u = r.sp.start()
    for s in ["One.", "Two.", "Three.", "Four."]:
        u.add(s)
    u.close()
    await asyncio.wait_for(u.wait(), 3)
    assert r.made == ["One.", "Two.", "Three.", "Four."]
    assert [s for b in r.played for s in b] == ["One.", "Two.", "Three.", "Four."]
    assert r.played[0] == ["One."]              # the first one alone: speech starts at once
    assert len(r.played) < 4                    # the rest joined while it played


async def test_word_cap_stops_at_a_sentence():
    r = Rig()
    u = r.sp.start(max_words=5)
    assert u.add("One two three.")
    assert not u.add("Four five six.")          # would make 6
    assert not u.add("Seven.") and u.capped
    u.close()
    await u.wait()
    assert r.made == ["One two three."]


async def test_cancel_stops_making_and_playing():
    r = Rig(synth_s=0.05, play_s=0.3)
    u = r.sp.start()
    u.add("One.")
    u.add("Two.")
    await asyncio.sleep(0.1)                    # "One." is playing
    r.sp.stop()
    u.add("Three.")
    await asyncio.wait_for(u.wait(), 1)
    assert u.cancelled
    assert r.played == [["One."]] and "Three." not in r.made


async def test_a_new_reply_stops_the_old_one():
    r = Rig(play_s=0.5)
    old = r.sp.start()
    old.add("Old.")
    await asyncio.sleep(0.05)
    new = r.sp.say("New one.")
    await asyncio.wait_for(new.wait(), 2)
    assert old.cancelled and r.played[-1] == ["New one."]


async def test_say_failure_skips_the_sentence():
    r = Rig()
    real = r.sp.synth

    async def flaky(sentence, procs):
        if sentence == "Bad.":
            raise OSError("no such voice")
        return await real(sentence, procs)
    r.sp.synth = flaky
    u = r.sp.start()
    for s in ["Good.", "Bad.", "Fine."]:
        u.add(s)
    u.close()
    await asyncio.wait_for(u.wait(), 2)
    assert [s for b in r.played for s in b] == ["Good.", "Fine."]


async def test_stick_playback_joins_clips_and_waits_for_them(monkeypatch):
    link = FakeLink()
    sp = speech.Speaker(lambda: link, lambda: "stick", lambda: "")
    slept = []

    async def fake_sleep(s):
        slept.append(s)
    monkeypatch.setattr(speech.asyncio, "sleep", fake_sleep)
    a = {"pcm": np.ones(1600, dtype=np.int16), "text": "A."}
    b = {"pcm": np.ones(3200, dtype=np.int16), "text": "B."}
    await sp.play([a, b], set())
    assert len(link.audio) == 1                 # one clip for both
    start = link.audio[0][0]
    samples = int.from_bytes(start[1:5], "little")
    assert samples == 1600 + 3200 + int(clips.RATE * speech.GAP_S)
    assert slept and slept[0] == pytest.approx(samples / clips.RATE + speech.PLAY_MARGIN_S)
