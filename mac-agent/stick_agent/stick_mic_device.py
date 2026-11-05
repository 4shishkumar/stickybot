"""The "stick-mic-bluetooth" microphone: a CoreAudio device that exists only while A is held.

Wispr Flow keeps a ranked list of microphones and, whenever the device list
changes, uses the highest-ranked one that is present. So the agent makes a mic
appear just before the stick presses Wispr's shortcut and removes it shortly
after the key-up: with "stick-mic-bluetooth" ranked first, Wispr hears the stick
while A is held and the user's usual mic the rest of the time. Nothing is
pinned and nothing forwards the real mic.

The device is a public (non-private) CoreAudio *aggregate* wrapping the
BlackHole 2ch driver (its only sub-device): whatever wispr_mic.py plays into
BlackHole's output is that aggregate's input. Any process may create and
destroy aggregates without admin rights; macOS lists them like a physical mic
("stick-mic-bluetooth (Aggregate)" in Wispr's list).

An aggregate outlives the process that created it, so every start, disconnect
and shutdown path destroys the ones with our UID (cleanup_stale()).

`CoreAudioDevices` is the real backend (ctypes on CoreAudio + CoreFoundation;
no Swift / Xcode needed); tests pass a fake with the same methods.
"""
import asyncio
import concurrent.futures
import ctypes
import logging
import os
import struct
import threading
import time

log = logging.getLogger("wispr")

NAME = "stick-mic-bluetooth"  # the old fixed name (and the fallback before a stick is picked)


def slug(stick_name):
    """'Arbaaz StickS3' -> 'arbaaz-sticks3' (same rule as the firmware's USB mic name)."""
    out, dash = [], False
    for c in str(stick_name or "").lower():
        if c.isascii() and c.isalnum():
            out.append(c)
            dash = False
        elif out and not dash:
            out.append("-")
            dash = True
    return "".join(out).strip("-")


def name_for(stick_name):
    """The Bluetooth mic's name for this stick: '<stick>-mic-bluetooth' ('stick-key-mic-bluetooth'),
    so two sticks' mics differ; the USB mic is '<stick>-mic-usb'."""
    s = slug(stick_name)
    return f"{s}-mic-bluetooth" if s else NAME
UID = "com.stickkey.wireless-mic"
CARRIER = "BlackHole 2ch"  # the driver underneath; Wispr never needs to be set to it


class Unavailable(Exception):
    pass


def _fcc(s):
    return struct.unpack(">I", s.encode())[0]


class _Addr(ctypes.Structure):
    _fields_ = [("sel", ctypes.c_uint32), ("scope", ctypes.c_uint32), ("elem", ctypes.c_uint32)]


SYSTEM = 1                 # kAudioObjectSystemObject
GLOBAL = _fcc("glob")
INPUT = _fcc("inpt")
UTF8 = 0x08000100          # kCFStringEncodingUTF8
CF_SINT32 = 3              # kCFNumberSInt32Type
# kAudioAggregateDevice*Key / kAudioSubDeviceUIDKey (AudioHardware.h)
K_NAME, K_UID, K_PRIVATE, K_STACKED, K_MAIN, K_SUBS, K_SUB_UID = (
    "name", "uid", "private", "stacked", "master", "subdevices", "uid")


class CoreAudioDevices:
    """The few CoreAudio calls the stick mic needs, via ctypes."""

    def __init__(self):
        try:
            self.ca = ctypes.CDLL("/System/Library/Frameworks/CoreAudio.framework/CoreAudio")
            self.cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
            self.libc = ctypes.CDLL(None)
        except OSError as e:
            raise Unavailable(f"CoreAudio isn't available: {e}") from e
        vp, u32, u32p = ctypes.c_void_p, ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32)
        ca, cf = self.ca, self.cf
        ca.AudioObjectGetPropertyDataSize.argtypes = [u32, ctypes.POINTER(_Addr), u32, vp, u32p]
        ca.AudioObjectGetPropertyData.argtypes = [u32, ctypes.POINTER(_Addr), u32, vp, u32p, vp]
        ca.AudioObjectSetPropertyData.argtypes = [u32, ctypes.POINTER(_Addr), u32, vp, u32, vp]
        ca.AudioHardwareCreateAggregateDevice.argtypes = [vp, u32p]
        ca.AudioHardwareDestroyAggregateDevice.argtypes = [u32]
        for f in (ca.AudioObjectGetPropertyDataSize, ca.AudioObjectGetPropertyData, ca.AudioObjectSetPropertyData,
                  ca.AudioHardwareCreateAggregateDevice, ca.AudioHardwareDestroyAggregateDevice):
            f.restype = ctypes.c_int32
        cf.CFStringCreateWithCString.argtypes = [vp, ctypes.c_char_p, u32]
        cf.CFStringCreateWithCString.restype = vp
        cf.CFStringGetCString.argtypes = [vp, ctypes.c_char_p, ctypes.c_long, u32]
        cf.CFNumberCreate.argtypes = [vp, ctypes.c_long, vp]
        cf.CFNumberCreate.restype = vp
        cf.CFDictionaryCreate.argtypes = [vp, ctypes.POINTER(vp), ctypes.POINTER(vp), ctypes.c_long, vp, vp]
        cf.CFDictionaryCreate.restype = vp
        cf.CFArrayCreate.argtypes = [vp, ctypes.POINTER(vp), ctypes.c_long, vp]
        cf.CFArrayCreate.restype = vp
        cf.CFRelease.argtypes = [vp]
        self.libc.proc_name.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
        self._dict_keys = ctypes.addressof(ctypes.c_char.in_dll(cf, "kCFTypeDictionaryKeyCallBacks"))
        self._dict_values = ctypes.addressof(ctypes.c_char.in_dll(cf, "kCFTypeDictionaryValueCallBacks"))
        self._array_values = ctypes.addressof(ctypes.c_char.in_dll(cf, "kCFTypeArrayCallBacks"))
        # Let the HAL run its notifications on its own thread (kAudioHardwarePropertyRunLoop = NULL), so
        # creating a device never waits for a run loop on the calling thread.
        none = ctypes.c_void_p(0)
        ca.AudioObjectSetPropertyData(SYSTEM, ctypes.byref(_Addr(_fcc("rnlp"), GLOBAL, 0)), 0, None,
                                      ctypes.sizeof(none), ctypes.byref(none))

    # --- reading ------------------------------------------------------------------------

    def _get(self, obj, sel, scope=GLOBAL):
        a = _Addr(_fcc(sel), scope, 0)
        n = ctypes.c_uint32(0)
        if self.ca.AudioObjectGetPropertyDataSize(obj, ctypes.byref(a), 0, None, ctypes.byref(n)):
            return None
        buf = ctypes.create_string_buffer(max(n.value, 1))
        if self.ca.AudioObjectGetPropertyData(obj, ctypes.byref(a), 0, None, ctypes.byref(n), buf):
            return None
        return buf.raw[:n.value]

    def _u32(self, obj, sel, scope=GLOBAL):
        a = _Addr(_fcc(sel), scope, 0)
        n = ctypes.c_uint32(4)
        v = ctypes.c_uint32(0)
        if self.ca.AudioObjectGetPropertyData(obj, ctypes.byref(a), 0, None, ctypes.byref(n), ctypes.byref(v)):
            return None
        return v.value

    @staticmethod
    def _u32s(b):
        return list(struct.unpack(f"{len(b) // 4}I", b[:len(b) // 4 * 4])) if b else []

    def _string(self, obj, sel):
        b = self._get(obj, sel)
        if not b or len(b) < 8:
            return ""
        ref = struct.unpack("Q", b[:8])[0]
        if not ref:
            return ""
        out = ctypes.create_string_buffer(512)
        try:
            self.cf.CFStringGetCString(ref, out, 512, UTF8)
        finally:
            self.cf.CFRelease(ref)  # the getter hands us a retained CFString
        return out.value.decode(errors="replace")

    def device_ids(self):
        return self._u32s(self._get(SYSTEM, "dev#"))

    def devices(self):
        """[(id, name, uid)] for every audio device."""
        return [(d, self._string(d, "lnam"), self._string(d, "uid ")) for d in self.device_ids()]

    def input_users(self, dev):
        """PIDs of the processes running input on `dev` right now, or None when macOS can't
        tell (per-process audio objects are macOS 14.2+)."""
        procs = self._get(SYSTEM, "prs#")
        if procs is None:
            return None
        pids = []
        for p in self._u32s(procs):
            if self._u32(p, "piri") and dev in self._u32s(self._get(p, "pdv#", INPUT)):
                pid = self._u32(p, "ppid")
                if pid is not None:
                    pids.append(pid)
        return pids

    # --- creating -----------------------------------------------------------------------

    def _str(self, s):
        return self.cf.CFStringCreateWithCString(None, s.encode(), UTF8)

    def _num(self, n):
        v = ctypes.c_int32(n)
        return self.cf.CFNumberCreate(None, CF_SINT32, ctypes.byref(v))

    def _dict(self, pairs):
        keys = [self._str(k) for k, _ in pairs]
        vals = [v for _, v in pairs]
        n = len(pairs)
        d = self.cf.CFDictionaryCreate(None, (ctypes.c_void_p * n)(*keys), (ctypes.c_void_p * n)(*vals), n,
                                       self._dict_keys, self._dict_values)
        for x in keys + vals:  # the dictionary retains its own references
            self.cf.CFRelease(x)
        return d

    def create_aggregate(self, name, uid, sub_uid):
        """-> (OSStatus, new device id). A public aggregate with one sub-device, `sub_uid`."""
        sub = self._dict([(K_SUB_UID, self._str(sub_uid))])
        subs = self.cf.CFArrayCreate(None, (ctypes.c_void_p * 1)(sub), 1, self._array_values)
        self.cf.CFRelease(sub)
        desc = self._dict([(K_NAME, self._str(name)), (K_UID, self._str(uid)), (K_PRIVATE, self._num(0)),
                           (K_STACKED, self._num(0)), (K_MAIN, self._str(sub_uid)), (K_SUBS, subs)])
        dev = ctypes.c_uint32(0)
        try:
            err = self.ca.AudioHardwareCreateAggregateDevice(desc, ctypes.byref(dev))
        finally:
            self.cf.CFRelease(desc)
        return err, dev.value

    def destroy_aggregate(self, dev):
        return self.ca.AudioHardwareDestroyAggregateDevice(dev)


class StickMicDevice:
    """Creates / removes the aggregate. Thread-safe (one lock); every call is a few
    CoreAudio calls, timed and logged. `backend`: a CoreAudioDevices-like object, or a
    factory for one (made on first use, so creating this never touches CoreAudio)."""

    def __init__(self, backend=None, name=NAME, uid=UID, carrier=CARRIER, own_pid=None):
        is_factory = backend is None or isinstance(backend, type) or not hasattr(backend, "devices")
        self._factory = (backend or CoreAudioDevices) if is_factory else None
        self._backend = None if is_factory else backend
        self.name, self.uid, self.carrier = name, uid, carrier
        self.own_pid = os.getpid() if own_pid is None else own_pid
        self.dev = None           # its CoreAudio id while it exists
        self.carrier_dev = None   # BlackHole's id
        self.baseline = set()     # processes already reading BlackHole itself when we made the device
        self.error = ""
        self.create_ms = self.destroy_ms = None  # the last ones
        self._lock = threading.Lock()

    def _ca(self):
        if self._backend is None:
            self._backend = self._factory()
        return self._backend

    def _set_error(self, msg):
        if msg and msg != self.error:
            log.warning("stick mic: %s", msg)
        self.error = msg

    @property
    def present(self):
        return self.dev is not None

    def ensure_present(self):
        """The device exists: -> "created" (just now), "present" (it already was) or "" (it can't be)."""
        with self._lock:
            try:
                ca = self._ca()
                devices = ca.devices()
                ours = [(d, name) for d, name, uid in devices if uid == self.uid]
                if ours and all(name == self.name for _d, name in ours):
                    self.dev = ours[0][0]
                    return "present"
                if ours:  # renamed in the settings: make it again under the new name
                    self._remove_all("removed the old-named", [d for d, _n in ours], ca)
                carrier = next(((d, uid) for d, name, uid in devices if name == self.carrier), None) or next(
                    ((d, uid) for d, name, uid in devices if self.carrier.lower() in name.lower()), None)
                if not carrier:
                    self.dev = None
                    self._set_error(f"{self.carrier} isn't installed — run: brew install --cask blackhole-2ch")
                    return ""
                self.carrier_dev = carrier[0]
                self.baseline = self._readers(ca, carrier[0])
                t = time.monotonic()
                err, dev = ca.create_aggregate(self.name, self.uid, carrier[1])
                if err or not dev:
                    self.dev = None
                    self._set_error(f"can't create the {self.name} device (CoreAudio error {err})")
                    return ""
                self.create_ms = round((time.monotonic() - t) * 1000, 1)
                self.dev = dev
                self._set_error("")
                log.info("stick mic: %s created in %.1f ms (id %d, on %s)", self.name, self.create_ms, dev,
                         self.carrier)
                return "created"
            except Exception as e:  # noqa: BLE001 — ctypes land: never take the agent down
                self.dev = None
                self._set_error(f"can't create the {self.name} device: {e}")
                return ""

    def remove(self):
        """Destroys every device with our UID (ours and any left over). -> how many."""
        with self._lock:
            return self._remove_all("removed")

    def cleanup_stale(self):
        """remove() for start / disconnect / exit: a crashed agent leaves its device behind."""
        with self._lock:
            return self._remove_all("removed")

    def _remove_all(self, what, ours=None, ca=None):
        try:
            ca = ca or self._ca()
            if ours is None:
                ours = [d for d, _name, uid in ca.devices() if uid == self.uid]
        except Exception as e:  # noqa: BLE001
            self._set_error(f"can't list audio devices: {e}")
            return 0
        n = 0
        for d in ours:
            t = time.monotonic()
            err = ca.destroy_aggregate(d)
            ms = round((time.monotonic() - t) * 1000, 1)
            if err:
                self._set_error(f"can't remove the {self.name} device (CoreAudio error {err})")
                continue
            n += 1
            self.destroy_ms = ms
            log.info("stick mic: %s %s%s in %.1f ms (id %d)", what, self.name,
                     "" if d == self.dev else " (left over)", ms, d)
        self.dev = None
        return n

    def _readers(self, ca, dev):
        pids = ca.input_users(dev)
        return None if pids is None else {p for p in pids if p != self.own_pid}

    def recording(self):
        """Someone (Wispr) is reading the device right now: True / False, or None when we can't
        tell. macOS reports an aggregate's readers on its sub-device (BlackHole), so a process that
        was already reading BlackHole before the device appeared makes it ambiguous."""
        if self.dev is None:
            return False
        if self.baseline is None or self.baseline:
            return None
        try:
            ca = self._ca()
            readers = set()
            for d in (self.dev, self.carrier_dev):
                if d is not None:
                    r = self._readers(ca, d)
                    if r is None:
                        return None
                    readers |= r
        except Exception:  # noqa: BLE001
            return None
        return bool(readers)


# --- the hold flow ------------------------------------------------------------------------

LEAD_MS = 250        # after creating it: time for Wispr to notice the new device before the key goes down
TAIL_MS = 800        # after the stream ends: keep it while Wispr finishes reading
SHOW_S = 60          # "create it now" from the dashboard, for ranking it in Wispr's settings
WAIT_READER_S = 4.0  # at most this much longer while Wispr is still reading it
POLL_S = 0.05


class Holds:
    """prepare -> device + lead time -> "ready" -> stream -> stop -> tail -> remove.

    Runs on the agent's asyncio loop; the CoreAudio calls go to one worker thread, in order.
    `send(msg)` is a coroutine (the Bluetooth link), `allowed()` -> "" or why not (stick on
    USB, not connected, off), `settings()` -> (lead ms, tail ms)."""

    def __init__(self, device, send, allowed=None, settings=None, emit=None, clock=time.monotonic):
        self.device = device
        self._send = send
        self._allowed = allowed or (lambda: "")
        self._settings = settings or (lambda: (LEAD_MS, TAIL_MS))
        self._emit = emit or (lambda kind, **data: None)
        self.clock = clock
        self._pool = concurrent.futures.ThreadPoolExecutor(max_workers=1, thread_name_prefix="stick-mic")
        self.holding = False
        self.hold_id = None
        self.keep_until = 0.0
        self.showing_until = 0.0
        self._life = None        # the task that watches the device and removes it
        self.last = None         # timings of the last hold, for the dashboard
        self._cur = None

    async def _run(self, fn):
        return await asyncio.get_running_loop().run_in_executor(self._pool, fn)

    def _report(self):
        self._emit("stick_mic", **self.status())

    async def prepare(self, hold_id=None):
        """A went down for Wispr: make the device, give Wispr the lead time, then say "ready"."""
        t0 = self.clock()
        self.holding, self.hold_id = True, hold_id
        why = self._allowed()
        if why:
            log.info("stick mic: not made (%s)", why)
            await self._ready(hold_id, False)
            return False
        made = await self._run(self.device.ensure_present)
        create_ms = round((self.clock() - t0) * 1000)
        lead = 0
        if made == "created":
            lead = max(0, int(self._settings()[0]))
            await asyncio.sleep(lead / 1000)
        if made:
            self._watch()
        current = self.hold_id == hold_id and self.holding
        if current:
            await self._ready(hold_id, bool(made))
        ready_ms = round((self.clock() - t0) * 1000)
        self._cur = {"t": time.time(), "made": made or "failed", "create_ms": create_ms if made == "created" else 0,
                     "lead_ms": lead, "ready_ms": ready_ms, "released_early": not current, "wispr_open_ms": None,
                     "wispr_closed_ms": None, "kept_ms": None, "_ready_at": self.clock(),
                     "_stop_at": None if current else self.clock()}
        log.info("stick mic: %s %d ms after prepare (device %s, create %d ms, lead %d ms)",
                 "ready" if current else "released before ready,", ready_ms, made or "failed",
                 self._cur["create_ms"], lead)
        if not made:
            self.last = {k: v for k, v in self._cur.items() if not k.startswith("_")}
        self._report()
        return bool(made)

    async def _ready(self, hold_id, ok):
        msg = {"src": "agent", "wispr": "ready"}
        if hold_id is not None:
            msg["id"] = hold_id
        if not ok:
            msg["device"] = False
        await self._send(msg)

    def stop(self):
        """The stream ended (key up): keep the device for the tail, then remove it."""
        if not self.holding:
            return
        self.holding = False
        self.keep_until = max(self.keep_until, self.clock() + max(0, int(self._settings()[1])) / 1000)
        if self._cur and self._cur["_stop_at"] is None:
            self._cur["_stop_at"] = self.clock()

    async def show(self, secs=SHOW_S):
        """From the dashboard: the device for `secs`, to rank it in Wispr once. -> "" or why not."""
        why = self._allowed()
        if why:
            return why
        if not await self._run(self.device.ensure_present):
            return self.device.error or "couldn't create it"
        self.showing_until = self.clock() + secs
        self.keep_until = max(self.keep_until, self.showing_until)
        self._watch()
        self._report()
        return ""

    def _watch(self):
        if self._life is None or self._life.done():
            self._life = asyncio.ensure_future(self._lifetime())

    async def _lifetime(self):
        reader_since = None
        was_reading = False
        while True:
            await asyncio.sleep(POLL_S)
            if not self.device.present:
                break
            reading = await self._run(self.device.recording)
            now = self.clock()
            cur = self._cur
            if cur and reading and not was_reading and cur["wispr_open_ms"] is None:
                cur["wispr_open_ms"] = round((now - cur["_ready_at"]) * 1000)
                log.info("stick mic: Wispr opened it %d ms after ready", cur["wispr_open_ms"])
            if cur and was_reading and not reading and cur["_stop_at"] is not None:
                cur["wispr_closed_ms"] = round((now - cur["_stop_at"]) * 1000)
                log.info("stick mic: Wispr closed it %d ms after the stream ended", cur["wispr_closed_ms"])
            was_reading = bool(reading)
            if self.holding or now < self.keep_until:
                reader_since = None
                continue
            if reading:  # still dictating: removing it now would stop Wispr with "MicDisconnected"
                reader_since = reader_since or now
                if now - reader_since < WAIT_READER_S:
                    continue
                log.warning("stick mic: Wispr still reads it %.1f s after the tail: removing it anyway",
                            WAIT_READER_S)
            await self._run(self.device.remove)
            if cur and cur["_stop_at"] is not None:
                cur["kept_ms"] = round((self.clock() - cur["_stop_at"]) * 1000)
                log.info("stick mic: removed %d ms after the stream ended", cur["kept_ms"])
            break
        self.showing_until = 0.0
        if self._cur:
            self.last = {k: v for k, v in self._cur.items() if not k.startswith("_")}
        self._report()

    async def drop(self, why):
        """Remove it now (the stick went to USB / disconnected, the feature was turned off)."""
        self.holding = False
        self.keep_until = self.showing_until = 0.0
        if self._life and not self._life.done():
            self._life.cancel()
        if await self._run(self.device.remove):
            log.info("stick mic: removed now (%s)", why)
            self._report()

    def shutdown(self):
        """Synchronous, from any thread (exit / SIGTERM): no device left behind."""
        self.holding = False
        try:
            self.device.cleanup_stale()
        except Exception:  # noqa: BLE001
            pass

    def status(self):
        now = self.clock()
        return {"name": self.device.name, "present": self.device.present, "error": self.device.error,
                "create_ms": self.device.create_ms, "destroy_ms": self.device.destroy_ms,
                "showing_s": round(self.showing_until - now) if self.showing_until > now else 0,
                "holding": self.holding, "last": self.last}


