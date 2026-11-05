"""Who is recording from BlackHole 2ch right now? (CoreAudio, read-only)

macOS 14.2+ has an audio object per process using audio
(kAudioHardwarePropertyProcessObjectList, 'prs#'), each with its PID ('ppid'),
whether it is running input ('piri') and on which devices ('pdv#', input
scope). Wispr Flow's helper shows up running input on "BlackHole 2ch" for
exactly as long as a dictation lasts, so watching this tells the smart mic when
Wispr starts and stops listening, whoever pressed the shortcut.

The device-level "running somewhere" flag is no use: our own output stream into
BlackHole keeps it running all the time. We never change anything here — only
property reads (and change listeners) — so no permission is needed.

How it's watched: a poll every 50 ms (each one is ~40 small CoreAudio calls,
~4 ms). We also register property listeners on every process's 'piri'; once one
has fired (so we know they work on this Mac), the poll slows to 4 Hz and a
listener call wakes it straight away instead.

`CoreAudio` is the real backend (ctypes); tests use a fake with the same methods.
On older macOS (no 'prs#') it raises Unsupported and the feature says so.
"""
import ctypes
import logging
import os
import struct
import threading
import time

log = logging.getLogger("wispr")

DEVICE = "BlackHole 2ch"
INTERVAL_S = 0.05      # 20 Hz: Wispr's start is seen within ~50 ms
SLOW_S = 0.25          # once listeners are known to work: they wake us, this is the safety net
DEVICES_EVERY_S = 1.0  # how often to check whether devices came or went (AirPods...)


class Unsupported(Exception):
    pass


def _fcc(s):
    return struct.unpack(">I", s.encode())[0]


class _Addr(ctypes.Structure):
    _fields_ = [("sel", ctypes.c_uint32), ("scope", ctypes.c_uint32), ("elem", ctypes.c_uint32)]


SYSTEM = 1                  # kAudioObjectSystemObject
GLOBAL = _fcc("glob")
INPUT = _fcc("inpt")
BUILT_IN = _fcc("bltn")     # kAudioDeviceTransportTypeBuiltIn
RUNNING_INPUT = _fcc("piri")
_LISTENER = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_uint32, ctypes.c_uint32, ctypes.POINTER(_Addr), ctypes.c_void_p)


class CoreAudio:
    """Read-only CoreAudio queries via ctypes."""

    def __init__(self):
        try:
            self.ca = ctypes.CDLL("/System/Library/Frameworks/CoreAudio.framework/CoreAudio")
            self.cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
            self.libc = ctypes.CDLL(None)
        except OSError as e:
            raise Unsupported(f"CoreAudio isn't available: {e}") from e
        u32p = ctypes.POINTER(ctypes.c_uint32)
        self.ca.AudioObjectGetPropertyDataSize.argtypes = [ctypes.c_uint32, ctypes.POINTER(_Addr), ctypes.c_uint32,
                                                           ctypes.c_void_p, u32p]
        self.ca.AudioObjectGetPropertyData.argtypes = [ctypes.c_uint32, ctypes.POINTER(_Addr), ctypes.c_uint32,
                                                       ctypes.c_void_p, u32p, ctypes.c_void_p]
        self.ca.AudioObjectAddPropertyListener.argtypes = [ctypes.c_uint32, ctypes.POINTER(_Addr), _LISTENER,
                                                           ctypes.c_void_p]
        self.cf.CFStringGetCString.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_long, ctypes.c_uint32]
        self.cf.CFRelease.argtypes = [ctypes.c_void_p]
        self.libc.proc_name.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
        if self._get(SYSTEM, "prs#") is None:
            raise Unsupported("needs macOS 14.2 or later (per-process audio objects)")
        self._names = {}      # pid -> process name
        self._wake = None     # listener target: wake(selector)
        self._watched = set()  # process objects with a listener
        self._listener = _LISTENER(self._on_property)  # kept alive as long as we are

    def _get(self, obj, sel, scope=GLOBAL):
        a = _Addr(_fcc(sel), scope, 0)
        n = ctypes.c_uint32(0)
        if self.ca.AudioObjectGetPropertyDataSize(obj, ctypes.byref(a), 0, None, ctypes.byref(n)):
            return None
        buf = ctypes.create_string_buffer(n.value)
        if self.ca.AudioObjectGetPropertyData(obj, ctypes.byref(a), 0, None, ctypes.byref(n), buf):
            return None
        return buf.raw[:n.value]

    def _u32(self, obj, sel, scope=GLOBAL):
        """A fixed-size UInt32 property in one call (no size query)."""
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
            self.cf.CFStringGetCString(ref, out, 512, 0x08000100)  # UTF-8
        finally:
            self.cf.CFRelease(ref)  # the getter hands us a retained CFString
        return out.value.decode(errors="replace")

    # --- listeners ------------------------------------------------------------------------

    def _on_property(self, _obj, n, addrs, _data):  # on a CoreAudio thread: only wake the monitor
        try:
            wake = self._wake
            if wake:
                for i in range(n):
                    wake(addrs[i].sel)
        except Exception:  # noqa: BLE001 — never raise into CoreAudio
            pass
        return 0

    def watch(self, wake):
        """Call wake(selector) when a process starts / stops input (or one comes / goes)."""
        first = self._wake is None
        self._wake = wake
        if first:
            self.ca.AudioObjectAddPropertyListener(SYSTEM, ctypes.byref(_Addr(_fcc("prs#"), GLOBAL, 0)),
                                                   self._listener, None)

    def _watch_processes(self, procs):
        if not self._wake:
            return
        for p in procs:
            if p not in self._watched:
                self._watched.add(p)
                self.ca.AudioObjectAddPropertyListener(p, ctypes.byref(_Addr(RUNNING_INPUT, GLOBAL, 0)),
                                                       self._listener, None)
        if len(self._watched) > 4 * len(procs) + 100:  # objects of processes that quit go away on their own
            self._watched &= set(procs)

    # --- devices ----------------------------------------------------------------------

    def device_ids(self):
        return tuple(self._u32s(self._get(SYSTEM, "dev#")))

    def device_name(self, dev):
        return self._string(dev, "lnam")

    def devices(self):
        """{device id: name} for every audio device."""
        return {d: self.device_name(d) for d in self.device_ids()}

    def default_input_name(self):
        dev = self._u32(SYSTEM, "dIn ")  # kAudioHardwarePropertyDefaultInputDevice
        return self.device_name(dev) if dev else ""

    def builtin_input_names(self):
        """Names of the built-in input devices (the MacBook's own mic)."""
        return [self.device_name(d) for d in self.device_ids()
                if self._u32(d, "tran") == BUILT_IN and self._get(d, "stm#", INPUT)]

    # --- processes ----------------------------------------------------------------------

    def process_name(self, pid):
        if pid not in self._names:
            if len(self._names) > 500:
                self._names.clear()
            buf = ctypes.create_string_buffer(256)
            n = self.libc.proc_name(pid, buf, 256)
            self._names[pid] = buf.value.decode(errors="replace") if n > 0 else f"pid {pid}"
        return self._names[pid]

    def input_users(self, dev):
        """[(pid, name)] of the processes running input on device `dev` right now."""
        procs = self._u32s(self._get(SYSTEM, "prs#"))
        self._watch_processes(procs)
        users = []
        for p in procs:
            if not self._u32(p, "piri"):  # kAudioProcessPropertyIsRunningInput
                continue
            if dev not in self._u32s(self._get(p, "pdv#", INPUT)):
                continue
            pid = self._u32(p, "ppid")
            if pid is not None:
                users.append((pid, self.process_name(pid)))
        return users


class RecordingMonitor:
    """Watches `backend` and calls on_change(recording, who) when some process other
    than ours starts / stops running input on the device, and on_devices() when the
    set of audio devices changes. `poll()` does one round (tests call it directly)."""

    def __init__(self, on_change=None, on_devices=None, backend=None, device_name=DEVICE,
                 interval=INTERVAL_S, own_pid=None, clock=time.monotonic):
        self.on_change = on_change or (lambda recording, who: None)
        self.on_devices = on_devices or (lambda: None)
        self.device_name = device_name
        self.interval = interval
        self.own_pid = os.getpid() if own_pid is None else own_pid
        self.clock = clock
        self.error = ""
        self.backend = None
        try:
            self.backend = backend() if callable(backend) else (backend or CoreAudio())
        except Unsupported as e:
            self.error = str(e)
        except Exception as e:  # noqa: BLE001 — anything odd in ctypes land: just don't monitor
            self.error = f"can't watch CoreAudio: {e}"
        self.recording = False
        self.who = ""
        self.dev = None          # the device's CoreAudio id
        self.listeners_work = False
        self._ids = None
        self._ids_at = -1e9
        self._thread = None
        self._stop = threading.Event()
        self._woken = threading.Event()

    @property
    def supported(self):
        return self.backend is not None

    def _find(self):
        devices = self.backend.devices()
        for d, name in devices.items():
            if name == self.device_name:
                return d
        for d, name in devices.items():
            if self.device_name.lower() in name.lower():
                return d
        return None

    def poll(self):
        if not self.supported:
            return
        now = self.clock()
        if self.dev is None or now - self._ids_at >= DEVICES_EVERY_S:
            self._ids_at = now
            ids = self.backend.device_ids()
            if ids != self._ids:
                first = self._ids is None
                self._ids = ids
                self.dev = self._find()
                if not first:
                    self.on_devices()
        users = []
        if self.dev is not None:
            users = [(pid, name) for pid, name in self.backend.input_users(self.dev) if pid != self.own_pid]
        recording = bool(users)
        who = ", ".join(sorted({name for _, name in users}))
        if recording != self.recording or (recording and who != self.who):
            self.recording, self.who = recording, who
            self.on_change(recording, who)

    def _wake(self, sel):
        if sel == RUNNING_INPUT and not self.listeners_work:
            self.listeners_work = True
            log.info("smart mic: CoreAudio change listeners work: polling slowly, woken on changes")
        self._woken.set()

    def _run(self, stop):
        failures = 0
        if hasattr(self.backend, "watch"):
            try:
                self.backend.watch(self._wake)
            except Exception as e:  # noqa: BLE001 — polling alone still works
                log.info("smart mic: no CoreAudio listeners (%s): polling", e)
        while not stop.is_set():
            self._woken.wait(SLOW_S if self.listeners_work else self.interval)
            self._woken.clear()
            if stop.is_set():
                break
            try:
                self.poll()
                failures = 0
            except Exception as e:  # noqa: BLE001 — keep watching; say so once in a while
                failures += 1
                if failures in (1, 100):
                    log.warning("smart mic: CoreAudio poll failed: %s", e)

    def start(self):
        if not self.supported or (self._thread and self._thread.is_alive()):
            return
        self._stop = threading.Event()  # each thread has its own: a stopped one can't come back
        self._thread = threading.Thread(target=self._run, args=(self._stop,), name="wispr-monitor", daemon=True)
        self._thread.start()

    def stop(self):
        self._stop.set()
        self._woken.set()
        self._thread = None
        if self.recording:
            self.recording, self.who = False, ""
            self.on_change(False, "")
