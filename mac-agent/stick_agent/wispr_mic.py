"""Wireless Wispr mic: the stick's mic, played where the "stick-mic-bluetooth" device hears it.

While A holds the Wispr shortcut away from USB, the stick streams its mic as
the same ADPCM frames as a B talk, between {"evt":"wispr","state":"prepare"} and
{"evt":"wispr","state":"stop"} (PROTOCOL.md, "Mac link"). It becomes the input of
the "stick-mic-bluetooth" device that stick_mic_device.py shows around each hold,
which Wispr ranks first — no cable. Two carriers (stick_mic_device.py):

- "tap": we play into the default output device; the agent's own process tap
  (inside that device) mutes it and hands it to the device's input. Audio only
  goes out while the tap exists (`gate`), so the stick never plays out of the
  speakers. The stream is opened at "prepare" and closed once the device is
  removed (it's a real output device: no silence kept playing into it).
- "blackhole": we play into BlackHole 2ch's output (Wispr is never set to BlackHole itself).

Wispr starts recording on key-down, so latency matters: frames are decoded as
they arrive, put in order by seq (a lost frame becomes 20 ms of silence),
resampled to the device's rate and played from a small jitter buffer (~80 ms)
by a PortAudio output stream (sounddevice) that stays open while the feature
is on and the stick is connected.
"""
import collections
import logging
import threading
import time

import numpy as np

from .adpcm import decode_frame

log = logging.getLogger("wispr")

DEVICE = "BlackHole 2ch"
DEFAULT_OUTPUT = "the default output"  # device_name for the tap carrier
RATE = 16000          # the stick's mic
TARGET_MS = 80        # jitter buffer: start playing once this much is queued
MAX_MS = 200          # more than this queued (a burst, clock drift): drop the oldest back to TARGET_MS
REORDER = 2           # frames that may arrive past a gap before the gap counts as lost
RESCAN_S = 10         # how often to look for the device again while it's missing


def _sounddevice():
    import sounddevice  # lazy: PortAudio only loads when the wireless mic is used
    return sounddevice


def find_device(sd, name=DEVICE):
    """The output device whose name contains `name` -> {index, name, rate, channels}, or None."""
    try:
        devices = sd.query_devices()
    except Exception as e:  # noqa: BLE001 — PortAudio missing / broken
        log.warning("can't list audio devices: %s", e)
        return None
    for i, d in enumerate(devices):
        if name.lower() in str(d.get("name", "")).lower() and d.get("max_output_channels", 0) > 0:
            return {"index": d.get("index", i), "name": d["name"], "rate": int(d.get("default_samplerate") or 48000),
                    "channels": min(2, int(d["max_output_channels"]))}
    return None


def find_default_output(sd):
    """The default output device -> {index, name, rate, channels}, or None."""
    try:
        d = sd.query_devices(kind="output")
    except Exception as e:  # noqa: BLE001 — no default output / PortAudio broken
        log.warning("can't find the default output device: %s", e)
        return None
    if not d or not d.get("max_output_channels", 0):
        return None
    return {"index": d.get("index"), "name": d["name"], "rate": int(d.get("default_samplerate") or 48000),
            "channels": min(2, int(d["max_output_channels"]))}


class Sequencer:
    """Frames by seq (u16, wraps) -> samples in order. A gap is filled with silence
    once REORDER later frames have arrived (or at the end); late frames are dropped."""

    def __init__(self, reorder=REORDER):
        self.reorder = reorder
        self.next = None      # seq expected next
        self.waiting = {}     # seq -> samples, arrived ahead of a gap
        self.size = 320       # samples per frame (the last one seen), for the silence
        self.lost = self.late = self.frames = 0

    def _ahead(self, seq):
        return (seq - self.next) & 0xFFFF

    def add(self, seq, samples):
        """-> list of int16 arrays now ready to play, in order."""
        if self.next is None:
            self.next = seq
        d = self._ahead(seq)
        if d >= 0x8000 or seq in self.waiting:  # behind (already played or given up) or a duplicate
            self.late += 1
            return []
        self.frames += 1
        if len(samples):
            self.size = len(samples)
        self.waiting[seq] = samples
        out = self._release()
        if len(self.waiting) > self.reorder:  # a gap that won't fill any more: silence
            while self.waiting and self.next not in self.waiting:
                out.append(np.zeros(self.size, dtype=np.int16))
                self.lost += 1
                self.next = (self.next + 1) & 0xFFFF
            out += self._release()
        return out

    def _release(self):
        out = []
        while self.next in self.waiting:
            out.append(self.waiting.pop(self.next))
            self.next = (self.next + 1) & 0xFFFF
        return out

    def flush(self):
        """The stream ended: everything still waiting, gaps as silence."""
        out = []
        while self.waiting:
            if self.next in self.waiting:
                out.append(self.waiting.pop(self.next))
            else:
                out.append(np.zeros(self.size, dtype=np.int16))
                self.lost += 1
            self.next = (self.next + 1) & 0xFFFF
        return out


class Resampler:
    """Streaming mono resampler (soxr, "quick" quality: a few samples of delay, no
    bursts; Wispr's recogniser doesn't need more). Pass-through when the rates match."""

    def __init__(self, rate_in, rate_out):
        self.rate_in, self.rate_out = rate_in, rate_out
        self._r = None
        if rate_in != rate_out:
            import soxr
            self._r = soxr.ResampleStream(rate_in, rate_out, 1, dtype="float32", quality="QQ")

    def __call__(self, x, last=False):
        x = np.asarray(x, dtype=np.float32)
        if self._r is None:
            return x
        return self._r.resample_chunk(x, last=last)


class JitterBuffer:
    """Audio at the device's rate between the Bluetooth side (push) and the audio
    thread (pull). Holds back until TARGET_MS is queued, so the small gaps between
    Bluetooth deliveries don't become clicks; when it runs dry it waits again."""

    def __init__(self, rate, target_ms=TARGET_MS, max_ms=MAX_MS):
        self.target = int(rate * target_ms / 1000)
        self.max = int(rate * max_ms / 1000)
        self._q = collections.deque()
        self._n = 0
        self._lock = threading.Lock()
        self.playing = False
        self.draining = False   # the stream ended: play out what's left without waiting for TARGET_MS
        self.underruns = 0
        self.dropped = 0        # samples dropped to keep the latency down

    @property
    def queued(self):
        return self._n

    def push(self, x):
        if not len(x):
            return
        with self._lock:
            self._q.append(np.asarray(x, dtype=np.float32))
            self._n += len(x)
            if self._n > self.max:
                self._drop(self._n - self.target)

    def _drop(self, n):
        self.dropped += n
        while n > 0 and self._q:
            head = self._q[0]
            if len(head) <= n:
                self._q.popleft()
                self._n -= len(head)
                n -= len(head)
            else:
                self._q[0] = head[n:]
                self._n -= n
                n = 0

    def pull(self, n):
        out = np.zeros(n, dtype=np.float32)
        with self._lock:
            if not self.playing:
                if self._n < self.target and not (self.draining and self._n):
                    return out
                self.playing = True
            i = 0
            while i < n and self._q:
                head = self._q[0]
                k = min(n - i, len(head))
                out[i:i + k] = head[:k]
                if k == len(head):
                    self._q.popleft()
                else:
                    self._q[0] = head[k:]
                self._n -= k
                i += k
            if i < n:  # ran dry
                self.playing = False
                if not self.draining:
                    self.underruns += 1
                else:
                    self.draining = False  # all played out
        return out

    def drain(self):
        with self._lock:
            self.draining = True

    def clear(self):
        with self._lock:
            self._q.clear()
            self._n = 0
            self.playing = self.draining = False


class WisprMic:
    """Plays Wispr streams from the stick into the carrier. `emit(kind, **data)` reports
    to the dashboard; `sd` is the sounddevice module (tests pass a fake). `carrier`: "blackhole",
    "tap" or None (none usable: `unavailable` says why); `gate()`: may audio go out now (tap mode:
    only while our tap exists)."""

    def __init__(self, emit=None, sd=None, device_name=DEVICE):
        self._emit = emit or (lambda kind, **data: None)
        self._sd = sd
        self.device_name = device_name
        self.carrier = "blackhole"
        self.unavailable = ""        # why there's no carrier (carrier None)
        self.gate = lambda: True
        self.gated = 0               # blocks held back by the gate in the last stream
        self.enabled = False
        self.dev = None              # find_device() result
        self.stream = None           # sounddevice.OutputStream while open
        self.jb = None
        self.resample = None
        self.seqr = None
        self.active = False          # between start and stop
        self.error = ""
        self._missing_logged = False
        self._looked_at = 0.0
        self.last = None             # stats of the last stream, for the dashboard
        self._started = 0.0

    # --- device ------------------------------------------------------------------

    def _module(self):
        if self._sd is None:
            try:
                self._sd = _sounddevice()
            except Exception as e:  # noqa: BLE001 — not installed / PortAudio missing
                self._set_error(f"sounddevice isn't available: {e}")
                return None
        return self._sd

    def _set_error(self, msg):
        if msg != self.error:
            self.error = msg
            if msg:
                log.warning("wireless mic: %s", msg)
            self._emit("wispr_mic", **self.status())

    def has_blackhole(self):
        """BlackHole 2ch is there (PortAudio restarted to see one installed since, while no stream is open)."""
        if self.carrier == "blackhole" and self.dev:
            return True
        sd = self._module()
        if sd is None:
            return False
        if find_device(sd, DEVICE):
            return True
        if self.stream is None and hasattr(sd, "_terminate"):
            try:
                sd._terminate()
                sd._initialize()
            except Exception:  # noqa: BLE001
                return False
            return find_device(sd, DEVICE) is not None
        return False

    def set_carrier(self, carrier, why="", gate=None):
        """The agent picked the carrier (stick_mic_device.pick_carrier): reopen on the new device."""
        gate = gate or (lambda: True)
        if carrier == self.carrier and why == self.unavailable:
            self.gate = gate
            return
        changed = carrier != self.carrier
        self.carrier, self.unavailable, self.gate = carrier, why, gate
        if changed:
            log.info("wireless mic: carrier %s%s", carrier or "none", f" ({why})" if why else "")
            self.close()
            self.dev, self._looked_at, self._missing_logged = None, 0.0, False
            self.device_name = DEFAULT_OUTPUT if carrier == "tap" else DEVICE
        if carrier is None:
            self._set_error(why)
        elif self.error and not self.dev:
            self._set_error("")
        self._emit("wispr_mic", **self.status())

    def device(self, force=False):
        """The device we play into (BlackHole, or the default output for the tap), looked for
        again at most every RESCAN_S while it's missing.
        PortAudio only sees devices added after it started once it's restarted, which we
        only do while no stream is open."""
        if self.dev and not force:
            return self.dev
        again = bool(self._looked_at)
        if again and not force and time.monotonic() - self._looked_at < RESCAN_S:
            return None
        self._looked_at = time.monotonic()
        if self.carrier is None:
            return None
        sd = self._module()
        if sd is None:
            return None
        if again and self.stream is None and hasattr(sd, "_terminate"):
            try:  # pick up a device installed since the agent started
                sd._terminate()
                sd._initialize()
            except Exception:  # noqa: BLE001
                pass
        self.dev = find_default_output(sd) if self.carrier == "tap" else find_device(sd, self.device_name)
        if self.dev:
            self._missing_logged = False
            log.info("wireless mic: playing into %s (%d Hz, %d ch)", self.dev["name"], self.dev["rate"],
                     self.dev["channels"])
            self._set_error("")
        elif not self._missing_logged:
            self._missing_logged = True
            self._set_error("there's no output device to play into" if self.carrier == "tap" else
                            f"{self.device_name} isn't installed — run: brew install --cask blackhole-2ch")
        return self.dev

    @property
    def ready(self):
        """On, with somewhere to play: what the stick is told ("wmic" in the hello)."""
        return self.enabled and self.device() is not None

    def set_enabled(self, on):
        self.enabled = bool(on)
        if not self.enabled:
            self.close()
        self._emit("wispr_mic", **self.status())

    # --- output stream -------------------------------------------------------------

    def _callback(self, outdata, frames, _time, _status):
        data = self.jb.pull(frames) if self.jb else np.zeros(frames, dtype=np.float32)
        if self.carrier != "blackhole" and not self.gate():
            # tap mode without our tap: this is the real speakers — silence (the audio is dropped)
            if self.active and data.any():
                self.gated += 1
            outdata[:] = 0
            return
        outdata[:] = data[:, None]  # mono to every channel

    def open(self):
        """Opens the output stream (silence until a stream starts). Returns True if open."""
        if self.stream is not None and getattr(self.stream, "active", True) is False and not self.active:
            self.close()  # stopped by itself (its device went away): open it again
            self.dev = None
        if self.stream is not None:
            return True
        dev = self.device()
        sd = self._module()
        if not dev or sd is None:
            return False
        self.jb = JitterBuffer(dev["rate"])
        try:
            self.stream = sd.OutputStream(device=dev["index"], samplerate=dev["rate"], channels=dev["channels"],
                                          dtype="float32", latency="low", callback=self._callback)
            self.stream.start()
        except Exception as e:  # noqa: BLE001 — device gone, busy...
            self.stream = None
            self.dev = None  # look for it again
            self._set_error(f"can't play into {dev['name']}: {e}")
            return False
        self._set_error("")
        return True

    def close(self):
        self.active = False
        if self.stream is not None:
            try:
                self.stream.stop()
                self.stream.close()
            except Exception as e:  # noqa: BLE001
                log.info("wireless mic: closing: %s", e)
            self.stream = None

    def warm(self):
        """The stick connected: have the stream ready before the first key-down. Not in tap mode:
        there it's the real output device (e.g. AirPods), so it's only open around a hold."""
        if self.enabled and self.carrier != "tap":
            self.open()

    def release(self):
        """The mic device is gone: in tap mode, let go of the output device until the next hold."""
        if self.carrier == "tap" and not self.active:
            self.close()

    # --- a stream from the stick -------------------------------------------------

    def start(self, rate=RATE):
        if not self.enabled or not self.open():
            self.active = False
            return False
        self.jb.clear()
        self.jb.underruns = self.jb.dropped = 0
        self.gated = 0
        self.resample = Resampler(int(rate or RATE), self.dev["rate"])
        self.seqr = Sequencer()
        self.active = True
        self._started = time.time()
        self._emit("wispr_mic", **self.status())
        return True

    def add(self, frame):
        if not self.active or len(frame) <= 5:
            return
        seq, samples = decode_frame(frame)
        for chunk in self.seqr.add(seq, np.asarray(samples, dtype=np.int16)):
            self.jb.push(self.resample(chunk.astype(np.float32) / 32768.0))

    def stop(self):
        if not self.active:
            return
        self.active = False
        for chunk in self.seqr.flush():
            self.jb.push(self.resample(chunk.astype(np.float32) / 32768.0))
        self.jb.push(self.resample(np.zeros(0, dtype=np.float32), last=True))
        self.jb.drain()  # play out the rest, then silence
        self.last = {"t": time.time(), "secs": round(time.time() - self._started, 1), "frames": self.seqr.frames,
                     "lost": self.seqr.lost, "late": self.seqr.late, "underruns": self.jb.underruns,
                     "dropped_ms": int(self.jb.dropped * 1000 / self.dev["rate"]) if self.dev else 0,
                     "gated": self.gated}
        log.info("wispr stream: %.1fs, %d frames, %d lost, %d late, %d underruns, %d ms dropped", self.last["secs"],
                 self.last["frames"], self.last["lost"], self.last["late"], self.last["underruns"],
                 self.last["dropped_ms"])
        self._emit("wispr_mic", **self.status())

    def status(self):
        return {"enabled": self.enabled, "device": self.dev["name"] if self.dev else None, "carrier": self.carrier,
                "open": self.stream is not None, "streaming": self.active, "error": self.error, "last": self.last}
