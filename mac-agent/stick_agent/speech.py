"""Reading replies aloud, sentence by sentence while Claude is still writing.

Each sentence is made with macOS `say` (offline voices) as soon as it's complete; the
next one is made while the current one plays, so speech is continuous. Output is the
stick's speaker (clips over Bluetooth) or the Mac's speakers.

The stick plays one clip at a time and a new clip's start packet stops the one playing,
so the next clip is only sent once the current one has finished; everything made by
then goes in that one clip (fewer gaps).
"""
import asyncio
import logging
import os
import re
import tempfile
import time

import numpy as np

from . import clips

log = logging.getLogger("speech")

GAP_S = 0.12          # silence between sentences joined into one clip
PLAY_MARGIN_S = 0.15  # the stick starts playing a little after the clip's last packet
MAX_CLIP_S = 30       # the stick's clip buffer

# A sentence ends at . ! ? … (plus closing quotes / brackets) followed by space, or at a line break
_BOUNDARY = re.compile(r"(?<=[.!?…])[\"'”’)\]]*\s+|\n+")
_ABBREV = re.compile(r"(?:\b(?:Mr|Mrs|Ms|Dr|St|vs|approx|e\.g|i\.e)|(?:^|\s)[A-Z])\.$")  # "Dr.", "J. Smith"


def speakable(text):
    """Text for the voice: no code blocks (finished or still open), no markdown symbols."""
    t = re.sub(r"```.*?```", "\n", text or "", flags=re.S)
    t = t.split("```", 1)[0]
    t = re.sub(r"[*_`#>|]+", "", t)
    t = re.sub(r"^[ \t]*[-•][ \t]+", "", t, flags=re.M)  # list bullets
    return t


class Sentences:
    """Cuts a reply that grows (or restarts) into complete sentences, each one once.

    feed(text) takes the whole reply so far and returns the sentences completed since the last
    call; a sentence is complete once the next one has started (so a short preamble that turns
    into a tool call is never spoken). finish() returns the rest."""

    def __init__(self):
        self.text = ""
        self.said = ""  # the part of speakable(text) already handed out

    def feed(self, text):
        text = text or ""
        if not text.startswith(self.text):  # a new message ("" = the text so far was a preamble)
            self.said = ""
        self.text = text
        return self._cut(final=False)

    def finish(self):
        return self._cut(final=True)

    def _cut(self, final):
        s = speakable(self.text)
        out = []
        # normally what was handed out is still the start; if markdown clean-up shifted it, go by length
        pos = len(self.said) if s.startswith(self.said) else min(len(self.said), len(s))
        for m in _BOUNDARY.finditer(s, pos):
            piece = s[pos:m.start()]
            if _ABBREV.search(piece) and not m.group(0).startswith("\n"):
                continue  # "Dr. Smith", "e.g. this": not the end
            if m.end() >= len(s):  # nothing after the boundary yet: the next sentence hasn't started
                break
            out.append(piece)
            pos = m.end()
        if final:
            out.append(s[pos:])
            pos = len(s)
        self.said = s[:pos]
        return [" ".join(p.split()) for p in out if p.strip() and re.search(r"\w", p)]


class Utterance:
    """One reply being spoken: add() sentences, close() when there are no more."""

    def __init__(self, speaker, max_words=None):
        self.speaker = speaker
        self.max_words = max_words
        self.words = 0
        self.capped = False         # stopped at max_words: the rest stays on the screen
        self.spoken = []            # sentences queued for speech
        self._in = asyncio.Queue()  # sentences to make
        self._ready = asyncio.Queue()  # made clips, in order
        self._procs = set()
        self.cancelled = False
        self._tasks = [asyncio.ensure_future(self._make()), asyncio.ensure_future(self._play())]

    def add(self, sentence):
        if self.cancelled or self.capped or not sentence.strip():
            return False
        n = len(sentence.split())
        if self.max_words is not None and self.words + n > self.max_words:
            self.capped = True
            return False
        self.words += n
        self.spoken.append(sentence)
        self._in.put_nowait(sentence)
        return True

    def close(self):
        self._in.put_nowait(None)

    async def wait(self):
        await asyncio.gather(*self._tasks, return_exceptions=True)

    def cancel(self):
        if self.cancelled:
            return
        self.cancelled = True
        for p in list(self._procs):
            if p.returncode is None:
                try:
                    p.terminate()
                except ProcessLookupError:
                    pass
        for t in self._tasks:
            t.cancel()

    async def _make(self):
        try:
            while (sentence := await self._in.get()) is not None:
                t = time.monotonic()
                try:
                    clip = await self.speaker.synth(sentence, self._procs)
                except Exception as e:  # noqa: BLE001 — `say` missing, a voice that's gone...
                    log.warning("speech failed: %s", e)
                    continue
                log.debug("made %r in %.2f s", sentence[:40], time.monotonic() - t)
                self._ready.put_nowait(clip)
        finally:
            self._ready.put_nowait(None)

    async def _play(self):
        done = False
        carry = None  # made, but it didn't fit in the last clip
        while not done:
            clip = carry if carry is not None else await self._ready.get()
            carry = None
            if clip is None:
                return
            batch, secs = [clip], clip_seconds(clip)
            while not self._ready.empty():  # everything already made goes in one go...
                more = self._ready.get_nowait()
                if more is None:
                    done = True
                    break
                if secs + GAP_S + clip_seconds(more) > MAX_CLIP_S:  # ...that fits the stick's clip buffer
                    carry = more
                    break
                batch.append(more)
                secs += GAP_S + clip_seconds(more)
            await self.speaker.play(batch, self._procs)


def clip_seconds(clip):
    """How long a clip made for the stick plays (0 for a file for the Mac's speakers)."""
    return len(clip["pcm"]) / clips.RATE if "pcm" in clip else 0.0


class Speaker:
    """Makes and plays speech. `output()` says where it goes right now: "stick" or "mac".
    `on_speech(bool)` hears when speech starts and when it ends (stopped, or all of it played):
    the stick shows "Reading aloud" (2.6.0)."""

    def __init__(self, link, output, voice, on_speech=None):
        self._link = link      # () -> the Bluetooth link
        self.output = output   # () -> "stick" | "mac"
        self.voice = voice     # () -> macOS voice name or ""
        self.on_speech = on_speech  # (bool) -> None: speech started / ended
        self.current = None    # the Utterance being spoken
        self.speaking = False

    def start(self, max_words=None):
        """A new reply to speak (stops the one before)."""
        if self.current:
            self.current.cancel()  # replaced: no "ended" in between
        u = self.current = Utterance(self, max_words)
        self._changed(True)
        asyncio.ensure_future(self._ended(u))
        return u

    async def _ended(self, u):
        await u.wait()
        if self.current is u:
            self.current = None
            self._changed(False)

    def _changed(self, on):
        if on == self.speaking:
            return
        self.speaking = on
        if self.on_speech:
            try:
                self.on_speech(on)
            except Exception as e:  # noqa: BLE001 — never break speech for it
                log.warning("on_speech: %s", e)

    def stop(self):
        if self.current:
            self.current.cancel()
        self.current = None
        self._changed(False)

    def say(self, text):
        """Speaks a whole text (returns at once; `.wait()` on the result to wait). Sentence by
        sentence, so a long one plays in clips the stick can hold, the first one soon."""
        u = self.start()
        cut = Sentences()
        for sentence in cut.feed(text) + cut.finish():
            u.add(sentence)
        u.close()
        return u

    # --- making and playing (tests replace these) ---------------------------------

    async def synth(self, sentence, procs):
        """`say` -> AIFF -> a clip: 16 kHz PCM for the stick, or the file for the Mac's speakers."""
        voice = self.voice()
        args = ["say", *(["-v", voice] if voice else [])]
        with tempfile.NamedTemporaryFile(suffix=".aiff", delete=False) as f:
            path = f.name
        try:
            p = await asyncio.create_subprocess_exec(*args, "-o", path, sentence)
            procs.add(p)
            try:
                await p.wait()
            finally:
                procs.discard(p)
            if self.output() == "mac":
                return {"path": path, "text": sentence}
            pcm = await asyncio.get_running_loop().run_in_executor(None, clips.load, path, MAX_CLIP_S, True)
        except BaseException:
            os.unlink(path)
            raise
        os.unlink(path)
        return {"pcm": pcm, "text": sentence}

    async def play(self, batch, procs):
        files = [c["path"] for c in batch if "path" in c]
        try:
            link = self._link()
            if files or not link.connected:
                for c in batch:
                    await self._play_mac(c, procs)
                return
            gap = np.zeros(int(clips.RATE * GAP_S), dtype=np.int16)
            parts = []
            for c in batch:
                parts += [c["pcm"], gap]
            pcm = np.concatenate(parts[:-1])[: clips.RATE * MAX_CLIP_S]
            t = time.monotonic()
            await link.send_audio(clips.packets(pcm))
            secs = len(pcm) / clips.RATE
            log.info("spoke %.1f s on the stick (%d sentence%s, sent in %.2f s)", secs, len(batch),
                     "s" if len(batch) != 1 else "", time.monotonic() - t)
            await asyncio.sleep(secs + PLAY_MARGIN_S)  # don't cut it off with the next clip
        finally:
            for c in files:
                try:
                    os.unlink(c)
                except OSError:
                    pass

    async def _play_mac(self, clip, procs):
        if "path" in clip:
            p = await asyncio.create_subprocess_exec("afplay", clip["path"])
        else:  # made for the stick, but it's gone: say it here
            voice = self.voice()
            p = await asyncio.create_subprocess_exec("say", *(["-v", voice] if voice else []), clip["text"])
        procs.add(p)
        try:
            await p.wait()
        finally:
            procs.discard(p)
