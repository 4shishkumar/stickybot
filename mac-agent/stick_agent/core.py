"""The agent: stick talk -> speech to text -> quick command or Claude -> reply."""
import asyncio
import collections
import json
import logging
import os
import time
import uuid

from . import brain, clips, commands, config, events, feed, github, home, link, mic_router, stt, weather, wispr_mic
from .adpcm import Recording
from .speech import Sentences, Speaker

log = logging.getLogger("agent")

# Characters the stick's fonts have beyond ASCII (see m5-stick-s3/tools/make_fonts.py)
STICK_EXTRA = set("°’‘“”–—…•·×")


def stick_text(s):
    """Text the stick can draw: keeps ASCII + its extra symbols, transliterates the rest."""
    import unicodedata
    out = []
    for ch in s or "":
        if ch.isascii() or ch in STICK_EXTRA:
            out.append(ch)
        else:
            out.append(unicodedata.normalize("NFKD", ch).encode("ascii", "ignore").decode())
    return "".join(out)
PROMPT_RESEND_S = 3    # the stick drops a prompt it hasn't heard about for 30 s
PROMPT_WAIT_S = 120


class StickStream:
    """The reply on the stick while Claude writes it: "working" updates, the latest text only,
    at most one per `interval` seconds and never two at once (Bluetooth writes queue up)."""

    def __init__(self, send, interval):
        self.send = send          # async (text) -> None
        self.interval = interval
        self.pending = None
        self.shown = None
        self.sent = 0             # updates sent (for the log / tests)
        self.closed = False
        self._last = 0.0
        self._wake = asyncio.Event()
        self._task = None

    def push(self, text):
        if self.closed or text == (self.pending if self.pending is not None else self.shown):
            return
        self.pending = text
        if self._task is None or self._task.done():
            self._task = asyncio.ensure_future(self._run())

    async def _run(self):
        while self.pending is not None and not self.closed:
            wait = self._last + self.interval - time.monotonic()
            if wait > 0:
                self._wake.clear()
                try:
                    await asyncio.wait_for(self._wake.wait(), wait)
                except TimeoutError:
                    pass
                continue
            text, self.pending = self.pending, None
            self._last = time.monotonic()
            self.shown = text
            self.sent += 1
            await self.send(text)

    async def send_now(self, text):
        """Sends this one straight away (a tool line: every one is shown), after one in flight."""
        self.pending = None
        self._wake.set()
        if self._task:
            await self._task
        if self.closed:
            return
        self._last = time.monotonic()
        self.shown = text
        self.sent += 1
        await self.send(text)

    async def close(self):
        """No more updates; returns once one being sent has gone (so "done" comes after it)."""
        self.closed = True
        self._wake.set()
        if self._task:
            await self._task


class Agent:
    def __init__(self):
        config.ensure_home()
        self.cfg = config.load()
        self.stt = stt.STT()
        self.brain = brain.Brain()
        self.link = link.Link(self._on_message, self._on_audio, self._on_link, self._hello_extra)
        self.last_reply = None  # {"t", "text"} for the home screen
        self.weather = weather.Weather()
        self.sounds = None          # the stick's sound settings, as it last reported them
        self._sounds_seen = None    # asyncio.Event, set when a report arrives
        self.feed = feed.Feed()
        self.notifier = events.Events(self)  # (self.events is the dashboard feed)
        self.github = github.GitHub(self)
        self.home = home.Home()
        self.link_state = "starting"
        self.state = "starting"          # ready | listening | transcribing | thinking | working | error | ...
        self.detail = ""
        self.stt_ready = False
        self.rec = None
        self.events = collections.deque(maxlen=300)   # for the dashboard
        self.subscribers = set()                       # SSE queues
        self.prompts = {}                              # id -> {"tool","hint","from","future"}
        self.loop = None
        self.stats = {"requests": 0, "quick": 0, "claude": 0, "started": time.time()}
        self.listeners = []  # extra callbacks for every message from the stick
        self.speaker = Speaker(lambda: self.link, self._speech_output,
                                      lambda: self.cfg.get("speak_voice") or "")
        self.wispr = wispr_mic.WisprMic(emit=self.emit_soon)  # A held away from USB: the stick's mic into BlackHole
        self.wispr.set_enabled(self.cfg.get("wireless_mic"))
        # Smart Wispr mic: no stick stream but Wispr records (its own shortcut) -> a real mic into BlackHole
        self.smart = mic_router.SmartMic(self.wispr, emit=self.emit_soon, stick_ready=lambda: bool(
            self.link.connected and self.wispr.enabled and self.wispr.dev))
        self.smart.configure(self.cfg.get("wireless_mic"), self.cfg.get("wispr_fallback_mic"))

    # --- events for the dashboard / menu bar --------------------------------

    def emit(self, kind, **data):
        ev = {"t": time.time(), "kind": kind, **data}
        self.events.append(ev)
        for q in list(self.subscribers):
            q.put_nowait(ev)
        return ev

    def emit_soon(self, kind, **data):
        """emit() from any thread (the smart mic reports from its own threads)."""
        try:
            running = asyncio.get_running_loop()
        except RuntimeError:
            running = None
        if self.loop is not None and running is not self.loop and self.loop.is_running():
            self.loop.call_soon_threadsafe(lambda: self.emit(kind, **data))
        else:
            self.emit(kind, **data)

    def set_state(self, state, detail=""):
        self.state, self.detail = state, detail
        self.emit("state", state=state, detail=detail)

    def status(self):
        return {"state": self.state, "detail": self.detail, "link": self.link_state, "stick": self.cfg.get("stick"),
                "stt_ready": self.stt_ready, "busy": self.brain.busy, "mtu": self.link.mtu, "stats": self.stats,
                "prompts": [{"id": k, **{x: v[x] for x in ("tool", "hint", "from")}} for k, v in self.prompts.items()],
                "model": self.cfg.get("model"), "weather": self.weather.now, "github": self.github.status(),
                "stick_approvals": self.cfg.get("stick_approvals", True), "wireless_mic": self.wispr.status(),
                "brain": self.brain.status() if getattr(self.brain, "status", None) else None}

    def _history(self, entry):
        with config.HISTORY.open("a") as f:
            f.write(json.dumps(entry) + "\n")

    # --- stick -> agent -----------------------------------------------------

    def _on_link(self, state):
        self.link_state = state
        self.emit("link", state=state)
        if state == "connected":
            asyncio.ensure_future(self.sound({"get": 1}))
            if getattr(self.brain, "prewarm", None):
                asyncio.ensure_future(self.brain.prewarm())  # Claude Code ready before the first request
            self.wispr.warm()  # ready before the first key-down
        else:
            self.wispr.stop()
            if not self.smart.on:  # the smart mic keeps BlackHole's output for keyboard dictation
                self.wispr.close()
        if self.state in ("starting", "ready", "no-stick", "searching", "paused") and self.stt_ready:
            self.set_state("ready" if state == "connected" else state)

    def _on_message(self, msg):
        for listen in list(self.listeners):  # e.g. firmware_api waiting for a "proto" reply
            listen(msg)
        if msg.get("src") == "agent" and msg.get("evt") == "event_shown":
            self.notifier.acked(msg.get("id"))
            return
        if msg.get("src") == "agent" and msg.get("evt") == "sounds":
            self.sounds = msg.get("sounds")
            if self._sounds_seen:
                self._sounds_seen.set()
            self.emit("sounds", sounds=self.sounds)
            return
        if msg.get("src") == "agent" and msg.get("evt") == "talk":
            if msg.get("state") == "start":
                self.stop_speaking()  # talking again interrupts a reply being read out
                self.rec = Recording()
                self.set_state("listening")
            elif msg.get("state") == "stop" and self.rec is not None:
                rec, self.rec = self.rec, None
                asyncio.ensure_future(self._handle_talk(rec))
        elif msg.get("src") == "agent" and msg.get("evt") == "wispr":  # A held: the mic for Wispr, not a request
            if msg.get("state") == "start":
                self.wispr.start(msg.get("rate") or wispr_mic.RATE)
            elif msg.get("state") == "stop":
                self.wispr.stop()
        elif msg.get("src") == "agent" and msg.get("evt") == "cancel":
            log.info("cancelled on the stick")
            self.brain.cancel()
            self.stop_speaking()
            self.set_state("ready" if self.link_state == "connected" else self.link_state)
        elif msg.get("cmd") == "permission" and msg.get("id") in self.prompts:
            self._decide(msg["id"], msg.get("decision"), "stick")

    def _on_audio(self, frame):
        if self.wispr.active:
            self.wispr.add(frame)
        elif self.rec is not None:
            self.rec.add(frame)

    def _hello_extra(self):
        card = dict(self.home.card(self))
        card["a"] = stick_text(card.get("a", ""))[:60]
        card["b"] = stick_text(card.get("b", ""))[:140]
        extra = {"home": card, "wmic": self.wispr.ready}
        if self.weather.now:
            extra["wx"] = {**self.weather.now, "at": stick_text(self.weather.now.get("at", ""))}
        return extra

    async def tell_stick(self, state, heard="", text=""):
        await self.link.send({"src": "agent", "state": state, "heard": stick_text(heard)[:200],
                              "text": stick_text(text)[:300]})

    # --- handling a request ---------------------------------------------------

    async def _handle_talk(self, rec):
        """Always ends by telling the stick something (it waits on "Thinking...")."""
        try:
            await self._handle_talk_inner(rec)
        except Exception as e:  # noqa: BLE001
            log.exception("talk failed")
            await self.tell_stick("error", "", f"Something went wrong: {e}"[:120])
            self.set_state("ready" if self.link_state == "connected" else self.link_state)

    async def _handle_talk_inner(self, rec):
        audio = rec.audio()
        secs = len(audio) / 16000
        if secs < 0.6:
            await self.tell_stick("error", "", "Too short - hold B while you speak.")
            self.set_state("ready")
            return
        self.set_state("transcribing", f"{secs:.1f}s of audio")
        if not self.stt_ready:
            await self.tell_stick("error", "", "Speech model still loading, try again in a moment.")
            self.set_state("ready")
            return
        t = time.time()
        text = await asyncio.get_running_loop().run_in_executor(self.stt.thread, self.stt.transcribe, audio)
        log.info("heard (%.1fs audio, %.2fs, %d lost): %s", secs, time.time() - t, rec.lost, text)
        if not text and secs < 2.5:
            # a quick press with no words: just close the screen quietly
            await self.tell_stick("cancelled")
            self.set_state("ready")
            return
        if not text:
            await self.tell_stick("error", "", "Didn't catch that.")
            self.set_state("ready")
            return
        await self.handle_text(text, source="voice")

    async def handle_text(self, text, source="typed"):
        self.stats["requests"] += 1
        started = time.time()
        self.emit("heard", text=text, source=source)
        entry = {"t": started, "heard": text, "source": source}
        reset, rest = brain.split_new_chat(text)
        if reset:  # "new chat", "start over" (+ maybe the request itself)
            self.brain.forget()
            self.emit("info", text="Next request starts a new conversation")
            if not rest:
                reply = "OK, new chat."
                self.emit("reply", text=reply, via="quick", ms=int((time.time() - started) * 1000))
                await self.tell_stick("done", text, reply)
                self._history({**entry, "via": "new_chat", "reply": reply})
                self.set_state("ready")
                return
            text = rest
        hit = commands.match(text)
        if hit:
            cmd, slots = hit
            result = await asyncio.get_running_loop().run_in_executor(None, commands.run, cmd, slots)
            if result is not None:
                self.stats["quick"] += 1
                self.emit("reply", text=result, via="quick", ms=int((time.time() - started) * 1000))
                await self.tell_stick("done", text, result)
                self._history({**entry, "via": "quick", "command": cmd.get("name"), "reply": result})
                self.set_state("ready")
                await self.speak(result, quick=True)
                return
        if self.brain.busy:
            await self.tell_stick("error", text, "Still working on the last request.")
            return
        self.stats["claude"] += 1
        self.set_state("thinking", text)
        await self.tell_stick("thinking", text, "")

        async def to_stick(line):
            await self.tell_stick("working", text, line)
        stream = StickStream(to_stick, max(0.1, float(self.cfg.get("stream_interval_ms") or 300) / 1000))
        live = self.cfg.get("stream_replies", True)
        # Speech sentence by sentence as the answer is written (see speech.py)
        utter = self.speaker.start(int(self.cfg.get("speak_max_words") or 40)) if self._stream_speech() else None
        sentences = Sentences()

        async def progress(line):
            self.set_state("working", line)
            self.emit("progress", text=line)
            await stream.send_now(line)

        async def on_text(answer):
            if utter:
                for sentence in sentences.feed(answer):
                    utter.add(sentence)
            shown = brain.plain(answer)
            if live and shown:
                stream.push(shown)

        try:
            ok, reply = await self.brain.ask(text, progress, on_text)
        except BaseException:
            if utter:
                utter.cancel()
            raise
        finally:
            await stream.close()
        if self.brain.cancelled:  # dismissed on the stick: say nothing more
            if utter:
                utter.cancel()
            self._history({**entry, "via": "claude", "ok": False, "reply": "(cancelled)"})
            return
        if stream.sent:
            log.info("reply streamed to the stick in %d update%s", stream.sent, "s" if stream.sent != 1 else "")
        self.emit("reply", text=reply, via="claude", ok=ok, ms=int((time.time() - started) * 1000))
        if ok:
            self.last_reply = {"t": time.time(), "text": reply}
        await self.tell_stick("done" if ok else "error", text, reply)
        self._history({**entry, "via": "claude", "ok": ok, "reply": reply})
        self.set_state("ready" if self.link_state == "connected" else self.link_state)
        if utter and ok and sentences.text and not utter.cancelled:
            for sentence in sentences.finish():
                utter.add(sentence)
            utter.close()
            if utter.spoken:
                self.emit("spoken", text=" ".join(utter.spoken))
        else:
            if utter:
                utter.cancel()
            await self.speak(reply)

    def _stream_speech(self):
        return bool(self.cfg.get("speak_replies") and self.cfg.get("speak_stream", True))

    def _speech_output(self):
        return "stick" if self.cfg.get("speak_output", "stick") == "stick" and self.link.connected else "mac"

    def should_speak(self, text, quick=False):
        if not text or not self.cfg.get("speak_replies"):
            return False
        if quick and not self.cfg.get("speak_quick"):
            return False
        return len(text.split()) <= int(self.cfg.get("speak_max_words") or 40)

    async def speak(self, text, quick=False, force=False):
        """Reads a reply aloud if it's short enough (longer ones stay on the screen). Returns at once;
        the speech goes on in the background (self.speaker.current)."""
        if not force and not self.should_speak(text, quick):
            return False
        self.speaker.say(text)
        self.emit("spoken", text=text)
        return True

    def stop_speaking(self):
        self.speaker.stop()

    # --- permission prompts (from the Claude Code hook) -----------------------

    async def ask_permission(self, tool, hint, origin, wait=PROMPT_WAIT_S):
        """Shows a Claude Code permission prompt on the stick + dashboard; returns
        'allow' / 'deny', or None if nobody answered (Claude Code then asks in the terminal)."""
        if not self.cfg.get("stick_approvals", True):
            return None  # turned off: Claude Code asks in the terminal straight away
        pid = "cli-" + uuid.uuid4().hex[:8]
        fut = asyncio.get_running_loop().create_future()
        self.prompts[pid] = {"tool": tool, "hint": hint, "from": origin, "future": fut}
        self.emit("prompt", id=pid, tool=tool, hint=hint, origin=origin)
        prompt = {"src": "cli", "prompt": {"id": pid, "tool": tool, "hint": hint[:160], "from": origin}}
        try:
            deadline = time.time() + min(wait, PROMPT_WAIT_S)
            while time.time() < deadline and not fut.done():
                await self.link.send(prompt)
                try:
                    await asyncio.wait_for(asyncio.shield(fut), PROMPT_RESEND_S)
                except TimeoutError:
                    pass
            return fut.result() if fut.done() else None
        finally:
            self.prompts.pop(pid, None)
            await self.link.send({"src": "cli"})  # clear it on the stick
            self.emit("prompt_done", id=pid, decision=fut.result() if fut.done() else None)

    def _decide(self, pid, decision, by):
        p = self.prompts.get(pid)
        if p and not p["future"].done():
            allow = decision in ("once", "allow", "always")
            p["future"].set_result("allow" if allow else "deny")
            log.info("prompt %s %s on %s", pid, "approved" if allow else "denied", by)

    def decide(self, pid, allow):
        self._decide(pid, "allow" if allow else "deny", "dashboard")

    # --- sounds on the stick -------------------------------------------------------

    async def sound(self, payload):
        """Sends {"get":1} / {"set":{event: tune}} / {"play": name} / {"on"} / {"volume"} to the
        stick and returns its sound settings (None if the stick didn't answer)."""
        self._sounds_seen = asyncio.Event()
        if not await self.link.send({"src": "agent", "sound": payload}):
            return None
        try:
            await asyncio.wait_for(self._sounds_seen.wait(), 3)
        except TimeoutError:
            return None
        return self.sounds

    async def play_clip(self, pack, name):
        """Plays a sound-pack clip on the stick's speaker. Returns an error string or None."""
        f = clips.find(pack, name)
        if not f:
            return "no such sound"
        pcm = await asyncio.get_running_loop().run_in_executor(None, clips.load, f)
        t = time.time()
        ok = await self.link.send_audio(clips.packets(pcm))
        log.info("clip %s/%s: %.2fs of audio sent in %.2fs", pack, name, len(pcm) / clips.RATE, time.time() - t)
        return None if ok else "stick not connected"

    # --- events -----------------------------------------------------------------

    async def claude_finished(self, elapsed_s, cwd, prompt):
        """From the Claude Code Stop hook: "Claude finished in <project>" for long tasks."""
        if elapsed_s < int(self.cfg.get("claude_done_min_s") or 0):
            return None
        project = os.path.basename(cwd.rstrip("/")) or "a session"
        took = f"{elapsed_s // 60} min" if elapsed_s >= 60 else f"{elapsed_s} s"
        sub = f"took {took}" + (f" - {prompt[:60]}" if prompt else "")
        return await self.notifier.notify("claude_done", f"Claude finished in {project}", sub)

    # --- settings ---------------------------------------------------------------

    def update_config(self, changes):
        self.cfg.update(changes)
        config.save(self.cfg)
        if "stick" in changes:
            self.link.set_target(self.cfg["stick"])
        if "wireless_mic" in changes or "wispr_fallback_mic" in changes:
            self.wispr.set_enabled(self.cfg.get("wireless_mic"))
            self.smart.configure(self.cfg.get("wireless_mic"), self.cfg.get("wispr_fallback_mic"))
            if self.link.connected:
                self.wispr.warm()
                asyncio.ensure_future(self.link.send({"src": "agent", "wmic": self.wispr.ready}))
        if "idle_content" in changes:
            self.feed._next = 0
        if "weather" in changes or "weather_location" in changes:
            asyncio.ensure_future(self.weather.refresh(self.cfg, force=True))
        self.emit("config", config=self.cfg)

    # --- run ----------------------------------------------------------------------

    async def run(self):
        self.loop = asyncio.get_running_loop()
        self.link.set_target(self.cfg.get("stick"))
        link_task = asyncio.create_task(self.link.run())

        async def weather_loop():
            while True:
                await self.weather.refresh(self.cfg)
                await self.feed.refresh(self.cfg)
                await asyncio.sleep(60)
        asyncio.create_task(weather_loop())
        asyncio.create_task(self.github.run())
        self.smart.start()  # watch who records from BlackHole (threads)
        self.set_state("starting", "loading speech model")
        try:
            await self.loop.run_in_executor(self.stt.thread, self.stt.load,
                                            self.cfg.get("stt_model") or stt.MODEL)
            self.stt_ready = True
        except Exception as e:  # noqa: BLE001
            log.exception("speech model failed")
            self.set_state("error", f"speech model failed: {e}")
        if self.stt_ready:
            self.set_state("ready" if self.link_state == "connected" else self.link_state)
        await link_task
