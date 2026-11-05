"""Smart Wispr mic: choose what Wispr hears through BlackHole 2ch.

Wispr Flow's microphone stays on "BlackHole 2ch" for good; the agent decides
what BlackHole carries:

- the stick is streaming (A held) -> the stick (wispr_mic.WisprMic), straight away,
  also in the middle of a dictation;
- Wispr is recording (the ⌥⌘ shortcut on the Mac keyboard...) and the stick isn't
  -> a real mic (the "fallback": the Mac's default input, or a device by name),
  opened when Wispr starts and closed as soon as it stops;
- otherwise silence.

"Wispr is recording" comes from audio_procs.RecordingMonitor (CoreAudio: some
process other than ours running input on BlackHole). Two small waits keep the
real mic out of stick dictations: after Wispr starts we give the stick
STICK_FIRST_S to say it's streaming (its key and its "wispr start" arrive
together), and after a stick stream ends we wait STICK_GRACE_S (Wispr stops a
moment after the key-up) before falling back.

The real mic needs macOS's Microphone permission for the agent's process. Without
it CoreAudio delivers silence: all-zero buffers for SILENCE_S -> we say how to
allow it.

Threads: the monitor thread reports recording changes, the asyncio loop reports
stick streams, and our worker thread opens / closes the real mic (that can take
~100 ms). `reconcile()` is the whole decision; tests call it directly.
"""
import logging
import threading
import time

import numpy as np

from . import audio_procs
from .wispr_mic import JitterBuffer, Resampler

log = logging.getLogger("wispr")

DEFAULT = "default"   # wispr_fallback_mic: the Mac's default input when Wispr starts
OFF = "off"           # no fallback: Wispr hears only the stick
STICK_FIRST_S = 0.15  # Wispr started: give a stick stream this long to show up first
STICK_GRACE_S = 0.6   # a stick stream ended: Wispr usually stops within this
SILENCE_S = 1.0       # all zeros for this long from a real mic: no permission
RETRY_S = 2.0         # a mic that failed to open: try again after this
TARGET_MS = 30        # the real mic's jitter buffer (its callbacks are regular)
MAX_MS = 150
BLOCK = 256           # input frames per callback (~5 ms at 48 kHz)
PERMISSION = "Allow Microphone access for stick-agent: System Settings → Privacy & Security → Microphone"


def is_blackhole(name):
    return "blackhole" in str(name or "").lower()


def input_devices(sd):
    """Real input devices sounddevice can open -> [{index, name, rate, channels}] (BlackHole left out)."""
    try:
        devices = sd.query_devices()
    except Exception as e:  # noqa: BLE001
        log.warning("can't list audio devices: %s", e)
        return []
    out = []
    for i, d in enumerate(devices):
        name = str(d.get("name", ""))
        if d.get("max_input_channels", 0) > 0 and not is_blackhole(name):
            out.append({"index": d.get("index", i), "name": name, "rate": int(d.get("default_samplerate") or 48000),
                        "channels": int(d["max_input_channels"])})
    return out


def choose(choice, inputs, default_name="", builtin_names=()):
    """The fallback mic for `choice` -> (device from `inputs` or None, the name we looked for).
    "default": the Mac's default input, or the built-in mic when the default is BlackHole;
    a name: that device, or the default one when it isn't connected; "off": None."""
    if choice == OFF:
        return None, ""
    by_name = {d["name"]: d for d in inputs}
    if choice and choice != DEFAULT and not is_blackhole(choice):
        if choice in by_name:
            return by_name[choice], choice
        log.info("smart mic: %s isn't connected: using the default input", choice)
    want = default_name if default_name and not is_blackhole(default_name) else ""
    if not want:
        builtin = [n for n in builtin_names if n in by_name]
        guess = [d["name"] for d in inputs if "macbook" in d["name"].lower() or "built-in" in d["name"].lower()]
        want = (builtin or guess or [""])[0]
    return by_name.get(want), want


class FallbackMic:
    """A real mic: sounddevice InputStream -> mono -> resampled to BlackHole's rate ->
    a small jitter buffer that WisprMic's output callback pulls from while forwarding."""

    def __init__(self, sd, dev, rate_out, clock=time.monotonic):
        self.sd, self.dev, self.clock = sd, dev, clock
        self.name = dev["name"]
        self.jb = JitterBuffer(rate_out, TARGET_MS, MAX_MS)
        self.resample = Resampler(dev["rate"], rate_out)
        self.stream = None
        self.forward = False
        self.opened_at = None
        self.open_ms = None        # InputStream() + start()
        self.first_ms = None       # open -> first audio callback
        self.peak = 0.0            # loudest sample since open (0 = no permission, or a dead mic)
        self.frames = 0
        self.status_errors = 0

    def open(self):
        t = self.clock()
        self.opened_at = t
        self.stream = self.sd.InputStream(device=self.dev["index"], samplerate=self.dev["rate"], channels=1,
                                          dtype="float32", latency="low", blocksize=BLOCK, callback=self._callback)
        self.stream.start()
        self.open_ms = int((self.clock() - t) * 1000)

    def _callback(self, indata, frames, _time, status):
        if status:
            self.status_errors += 1
        if self.first_ms is None:
            self.first_ms = int((self.clock() - self.opened_at) * 1000)
        x = np.asarray(indata, dtype=np.float32)
        x = x[:, 0] if x.ndim == 2 else x
        self.frames += frames
        if len(x):
            self.peak = max(self.peak, float(np.abs(x).max()))
        if self.forward:
            self.jb.push(self.resample(x))

    def set_forward(self, on):
        if on and not self.forward:
            self.jb.clear()
        self.forward = on

    def pull(self, n):
        return self.jb.pull(n)

    def silent_for(self):
        """Seconds of nothing but zeros since open (0 once any sound has come)."""
        if self.peak > 0 or self.opened_at is None:
            return 0.0
        return self.clock() - self.opened_at

    def close(self):
        self.forward = False
        if self.stream is not None:
            try:
                self.stream.stop()
                self.stream.close()
            except Exception as e:  # noqa: BLE001
                log.info("smart mic: closing %s: %s", self.name, e)
            self.stream = None


class SmartMic:
    """The router. `wispr` is the WisprMic (owns the BlackHole output); `backend` the
    CoreAudio backend (factory) for the monitor; `stick_ready()` says whether an A press
    would stream from the stick right now (connected, wireless mic ready)."""

    def __init__(self, wispr, emit=None, backend=None, clock=time.monotonic, stick_ready=None):
        self.wispr = wispr
        wispr.on_stream = self.on_stick
        wispr.smart = self
        self._emit = emit or (lambda kind, **data: None)
        self.backend = backend
        self.clock = clock
        self.stick_ready = stick_ready or (lambda: False)
        self.enabled = False
        self.choice = DEFAULT
        self.monitor = None
        self.threaded = False
        self.recording = False
        self.who = ""
        self.rec_since = 0.0
        self.stick_ended = -1e9
        self.mic = None              # FallbackMic while open
        self.mic_error = ""
        self._permission_logged = False
        self._failed_at = -1e9
        self.devices_dirty = False
        self.hears = None            # "stick" | mic name | "" (Wispr records, nothing to give it) | None (idle)
        self.session = None
        self.last_session = None
        self._lock = threading.RLock()
        self._wake = threading.Event()
        self._worker = None
        self._running = False

    # --- setup ----------------------------------------------------------------------------

    @property
    def on(self):
        """Choosing for Wispr: wireless mic on, a fallback chosen and the monitor working."""
        return self.enabled and self.choice != OFF and self.monitor is not None and self.monitor.supported

    def configure(self, enabled, choice):
        with self._lock:
            self.enabled = bool(enabled)
            self.choice = choice if isinstance(choice, str) and choice else DEFAULT
        if self._running:
            if self.enabled and self.choice != OFF:
                self._start_monitor()
            elif self.monitor:
                self.monitor.stop()
        self._kick()
        self._emit_status()

    def start(self, threaded=True):
        """From Agent.run(): start watching (threads), if the settings say so."""
        self._running = True
        self.threaded = threaded
        if threaded and self._worker is None:
            self._worker = threading.Thread(target=self._work, name="wispr-router", daemon=True)
            self._worker.start()
        if self.enabled and self.choice != OFF:
            self._start_monitor()
        self._kick()

    def _start_monitor(self):
        if self.monitor is None:
            self.monitor = audio_procs.RecordingMonitor(on_change=self.on_recording, on_devices=self.on_devices,
                                                        backend=self.backend or audio_procs.CoreAudio)
            if self.monitor.supported:
                log.info("smart mic: watching who records from %s", self.wispr.device_name)
            else:
                log.warning("smart mic: unsupported here: %s", self.monitor.error)
        if self.threaded:
            self.monitor.start()

    # --- inputs ---------------------------------------------------------------------------

    def on_recording(self, recording, who):
        """From the monitor (its thread): Wispr (or whoever) started / stopped recording BlackHole."""
        with self._lock:
            if recording and not self.recording:
                self.rec_since = self.clock()
                log.info("smart mic: %s started recording", who)
            elif not recording and self.recording:
                log.info("smart mic: %s stopped recording", self.who)
            self.recording, self.who = recording, who
        self._kick()

    def on_stick(self, active):
        """From WisprMic.start() / stop() (the asyncio loop). The output follows the stick at
        once by itself; this just stops / restarts the real mic's share. No lock: the worker
        may be holding it while a mic opens, and the stick's frames mustn't wait for that."""
        if not active:
            self.stick_ended = self.clock()
        mic = self.mic
        if active and mic:
            mic.set_forward(False)
            self.wispr.fallback = None
        self._kick()

    def on_devices(self):
        """Audio devices came or went: PortAudio needs a restart to see new ones."""
        self.devices_dirty = True
        self._kick()

    def _kick(self):
        if self.threaded and self._running:
            self._wake.set()
        else:
            self.reconcile()

    def stop(self):
        """Stop watching and routing (closes the real mic)."""
        self._running = False
        if self.monitor:
            self.monitor.stop()
        self._wake.set()
        with self._lock:
            self._close_mic()
            self._end_session(self.clock())

    def _work(self):
        while self._running:
            try:
                wait = self.reconcile()
            except Exception:  # noqa: BLE001 — keep routing
                log.exception("smart mic: routing failed")
                wait = 1.0
            self._wake.wait(1.0 if wait is None else max(0.01, wait))
            self._wake.clear()

    # --- the decision -----------------------------------------------------------------------

    def reconcile(self):
        """Opens / closes / forwards the real mic for the current state. -> seconds until it
        should run again (a wait running out, the silence check), or None."""
        with self._lock:
            now = self.clock()
            if not (self.on and self.recording):
                self._close_mic()
                self._end_session(now)
                self._set_hears(None if not self.recording else "")
                if self.on and self.devices_dirty and not self.wispr.active:
                    self._refresh_devices()
                if self.on:
                    self.wispr.open()  # keep BlackHole's output ready for the next dictation
                return None
            if self.session is None:
                self._begin_session(now)
            if self.wispr.active:
                if self.mic:
                    self.mic.set_forward(False)
                self.wispr.fallback = None
                self._set_hears("stick")
                return self._silence_check(now)
            first = STICK_FIRST_S if self.stick_ready() else 0.0
            hold = max(self.rec_since + first, self.stick_ended + STICK_GRACE_S)
            if now < hold:
                if self.hears != "stick":
                    self._set_hears("")
                return hold - now
            if self.mic is None and now - self._failed_at >= RETRY_S:
                self._open_mic(now)
            if self.mic is None:
                self._set_hears("")
                return RETRY_S
            self.mic.set_forward(True)
            self.wispr.fallback = self.mic
            self._set_hears(self.mic.name)
            return self._silence_check(now)

    def _silence_check(self, now):
        if not self.mic:
            return None
        silent = self.mic.silent_for()
        if silent >= SILENCE_S:
            self._set_mic_error(f"No sound from {self.mic.name}. {PERMISSION}", permission=True)
            return None
        if self.mic.peak > 0 and self.mic_error:
            self._set_mic_error("")
        return 0.1 if self.mic.peak == 0 else None

    def _set_mic_error(self, msg, permission=False):
        if msg == self.mic_error:
            return
        self.mic_error = msg
        if msg and (not permission or not self._permission_logged):
            log.warning("smart mic: %s", msg)
            self._permission_logged = self._permission_logged or permission
        self._emit_status()

    def _set_hears(self, hears):
        if hears == self.hears:
            return
        if self.session is not None:
            self._account(self.clock())
        self.hears = hears
        self._emit_status()

    # --- the real mic --------------------------------------------------------------------------

    def _resolve(self):
        sd = self.wispr._module()
        if sd is None:
            return None, ""
        default, builtin = "", []
        backend = self.monitor.backend if self.monitor else None
        try:
            default = backend.default_input_name() if backend else ""
            builtin = backend.builtin_input_names() if backend else []
        except Exception as e:  # noqa: BLE001
            log.info("smart mic: can't read the default input: %s", e)
        dev, want = choose(self.choice, input_devices(sd), default, builtin)
        if dev is None and want and not self.wispr.active:  # connected since PortAudio started (AirPods...)
            log.info("smart mic: %s isn't known to PortAudio yet: restarting it", want)
            self._refresh_devices()
            dev, want = choose(self.choice, input_devices(sd), default, builtin)
        return dev, want

    def _open_mic(self, now):
        t = self.clock()
        if not self.wispr.open():
            self._failed_at = now
            self._set_mic_error(self.wispr.error or "can't play into BlackHole 2ch")
            return
        dev, want = self._resolve()
        if dev is None:
            self._failed_at = now
            self._set_mic_error(f"no microphone to use{f' ({want} not found)' if want else ''}")
            return
        mic = FallbackMic(self.wispr._module(), dev, self.wispr.dev["rate"], clock=self.clock)
        try:
            mic.open()
        except Exception as e:  # noqa: BLE001 — PortAudio error: often the permission
            mic.close()
            self._failed_at = now
            self._set_mic_error(f"can't open {dev['name']}: {e}. {PERMISSION}", permission=True)
            return
        self.mic = mic
        total = int((self.clock() - t) * 1000)
        if self.session is not None:
            self.session.update(mic=mic.name, open_ms=mic.open_ms, ready_ms=int((self.clock() - self.rec_since) * 1000))
        log.info("smart mic: opened %s (%d Hz) in %d ms (%d ms after Wispr started)", mic.name, dev["rate"], total,
                 int((self.clock() - self.rec_since) * 1000))
        if self.mic_error and "Microphone access" not in self.mic_error:
            self._set_mic_error("")

    def _close_mic(self):
        if self.mic is None:
            return
        mic, self.mic = self.mic, None
        self.wispr.fallback = None
        mic.close()
        if self.session is not None and mic.first_ms is not None:
            self.session["first_audio_ms"] = mic.first_ms
        log.info("smart mic: closed %s", mic.name)

    def _refresh_devices(self):
        self.devices_dirty = False
        if self.mic is None:
            self.wispr.refresh_devices()

    # --- stats ------------------------------------------------------------------------------------

    def _begin_session(self, now):
        self.session = {"who": self.who, "t0": now, "start": time.time(), "sources": {}, "mic": None,
                        "open_ms": None, "ready_ms": None, "first_audio_ms": None, "_at": now}

    def _account(self, now):
        s = self.session
        key = self.hears if self.hears else "nothing"
        if self.hears is not None:
            s["sources"][key] = s["sources"].get(key, 0.0) + now - s["_at"]
        s["_at"] = now

    def _end_session(self, now):
        s = self.session
        if s is None:
            return
        if self.mic is not None and self.mic.first_ms is not None:
            s["first_audio_ms"] = self.mic.first_ms
        self._account(now)
        self.session = None
        secs = now - s["t0"]
        sources = {k: round(v, 1) for k, v in s["sources"].items() if v >= 0.05}
        self.last_session = {"t": s["start"], "secs": round(secs, 1), "who": s["who"], "sources": sources,
                             "mic": s["mic"], "open_ms": s["open_ms"], "ready_ms": s["ready_ms"],
                             "first_audio_ms": s["first_audio_ms"]}
        extra = ""
        if s["mic"]:
            extra = f"; {s['mic']} opened in {s['open_ms']} ms, ready {s['ready_ms']} ms after Wispr started"
            if s["first_audio_ms"] is not None:
                extra += f", first audio after {s['first_audio_ms']} ms"
        log.info("smart mic session: %.1fs (%s), Wispr heard %s%s", secs, s["who"] or "?",
                 ", ".join(f"{k} {v}s" for k, v in sources.items()) or "nothing", extra)

    # --- status -----------------------------------------------------------------------------------

    def _emit_status(self):
        self._emit("wispr_mic", **self.wispr.status())

    def status(self):
        if not (self.enabled and self.choice != OFF):
            smart = "off"
        elif self.monitor is None:
            smart = "starting"
        else:
            smart = "on" if self.monitor.supported else "unsupported"
        return {"fallback": self.choice, "smart": smart,
                "smart_error": self.monitor.error if self.monitor else "", "wispr_recording": self.recording,
                "recorder": self.who, "hears": self.hears, "mic": self.mic.name if self.mic else None,
                "mic_error": self.mic_error, "session": self.last_session}

    def inputs(self):
        """For the dashboard: the input devices to choose from + the current default."""
        sd = self.wispr._module()
        with self._lock:
            if self.devices_dirty and self.mic is None and not self.wispr.active:
                self._refresh_devices()
            names = [d["name"] for d in input_devices(sd)] if sd else []
        default = ""
        try:
            backend = self.monitor.backend if self.monitor and self.monitor.backend else None
            if backend is None:  # not watching (off): a one-off look
                make = self.backend or audio_procs.CoreAudio
                backend = make() if callable(make) else make
            default = backend.default_input_name()
        except Exception:  # noqa: BLE001 — older macOS, no CoreAudio: no default shown
            pass
        return {"devices": names, "default": default}
