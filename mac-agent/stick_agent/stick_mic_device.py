"""The "<stick>-bt-mic" microphone ("stickybot-bt-mic"): a CoreAudio device that exists only while A is held.

Wispr Flow keeps a ranked list of microphones and, whenever the device list
changes, uses the highest-ranked one that is present. So the agent makes a mic
appear just before the stick presses Wispr's shortcut and removes it shortly
after the key-up: with "<stick>-bt-mic" ranked first, Wispr hears the stick
while A is held and the user's usual mic the rest of the time. Nothing is
pinned and nothing forwards the real mic.

The device is a public (non-private) CoreAudio *aggregate*; what carries the
stick's audio into its input is one of two "carriers":

- "tap" (macOS 14.2+, no driver): a Core Audio *process tap* of the agent's own
  audio output, in the aggregate's tap list, with the default output device as
  its (clock) sub-device. wispr_mic.py plays the stick into the default output;
  the tap mutes all of this process's sound and hands it to the aggregate's
  input instead. Needs the "System Audio Recording" permission: until it's
  granted a tap is silent AND doesn't mute, so the stick would play out of the
  speakers. So the tap is only used after probe() proved it works (an inaudible
  -80 dBFS tone through a test-named device), and wispr_mic.py only lets audio
  out while our tap exists (`tap_live`).
- "blackhole": the BlackHole 2ch driver as the only sub-device; wispr_mic.py
  plays into BlackHole's output, which is the aggregate's input.

Any process may create and destroy aggregates and taps without admin rights;
macOS lists the aggregate like a physical mic ("stickybot-bt-mic (Aggregate)").

Aggregates and public taps both outlive the process that created them, so every
start, disconnect and shutdown path destroys the aggregates with our UID and the
taps with our tap name (cleanup_stale()).

`CoreAudioDevices` is the real backend (ctypes on CoreAudio + CoreFoundation + the
Objective-C runtime for CATapDescription; no Swift / Xcode / pyobjc needed); tests
pass a fake with the same methods.
"""
import asyncio
import concurrent.futures
import ctypes
import logging
import math
import os
import platform
import re
import struct
import threading
import time

log = logging.getLogger("wispr")

NAME = "stickybot-bt-mic"  # the fallback: no stick picked yet, or its name has nothing usable
OLD_NAME = "stick-mic-bluetooth"  # the fixed name before 1.8.1
OLD_AUTO = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*-mic-bluetooth")  # "<slug>-mic-bluetooth", 1.8.1-1.8.6


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
    """The Bluetooth mic's name for this stick: '<stick>-bt-mic' ('stickybot-bt-mic'), so two
    sticks' mics differ; the USB mic is '<stick>-usb-mic' (the firmware names that one)."""
    s = slug(stick_name)
    return f"{s}-bt-mic" if s else NAME


def is_auto_name(name):
    """A saved wispr_mic_name that is an older automatic name ("stick-mic-bluetooth",
    "<slug>-mic-bluetooth"): it means "automatic", so the mic follows the stick's name
    ("<slug>-bt-mic" now). Anything else was typed in and wins."""
    name = str(name or "").strip()
    return name == OLD_NAME or bool(OLD_AUTO.fullmatch(name))


UID = "com.stickkey.wireless-mic"
CARRIER = "BlackHole 2ch"  # the driver underneath; Wispr never needs to be set to it
TAP, BLACKHOLE = "tap", "blackhole"  # the carriers (module docstring)
CARRIER_TITLES = {TAP: "Apple audio tap", BLACKHOLE: "BlackHole 2ch"}
CARRIER_PREFS = ("auto", TAP, BLACKHOLE)  # config wireless_mic_carrier
TAP_SUFFIX = ".tap"            # a tap's name = the aggregate's UID + this (how stale ones are found)
CHECK_SUFFIX = ".check"        # the probe's device: UID + this, name + "-check" (never the real one)
TAP_MIN_MACOS = (14, 2)
PERMISSION_HINT = ("allow System Audio Recording for StickyBot (“Python” if StickyBot.app isn't set up): "
                   "System Settings → "
                   "Privacy & Security → Screen & System Audio Recording → System Audio Recording Only")


def macos_version():
    v = platform.mac_ver()[0]
    out = [int(p) if p.isdigit() else 0 for p in (v or "").split(".")]
    return tuple(out + [0, 0])[:2] if v else None


def macos_supports_tap():
    """macOS 14.2+ (Core Audio process taps)."""
    v = macos_version()
    return bool(v) and v >= TAP_MIN_MACOS


def pick_carrier(pref, tap_supported, tap_state, blackhole):
    """-> (carrier or None, why not). pref: config wireless_mic_carrier ("auto" | "tap" | "blackhole");
    tap_state: the probe's result ("ok" | "failed" | "error" | "checking" | None = not run yet);
    blackhole: BlackHole 2ch is installed."""
    pref = pref if pref in CARRIER_PREFS else "auto"
    no_bh = "BlackHole 2ch isn't installed — run: brew install --cask blackhole-2ch"
    if pref == BLACKHOLE:
        return (BLACKHOLE, "") if blackhole else (None, no_bh)
    if tap_supported and tap_state == "ok":
        return TAP, ""
    if pref == TAP or (tap_supported and not blackhole):
        if not tap_supported:
            return None, f"the Apple audio tap needs macOS {'.'.join(map(str, TAP_MIN_MACOS))} or newer"
        if tap_state in (None, "checking"):
            return None, "checking the Apple audio tap…"
        if tap_state == "failed":
            return None, f"the Apple audio tap isn't allowed yet — {PERMISSION_HINT}, then press Check again"
        return None, "the Apple audio tap doesn't work on this Mac — use BlackHole 2ch instead"
    return (BLACKHOLE, "") if blackhole else (None, no_bh)


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
# kAudioAggregateDeviceTapListKey / TapAutoStartKey, kAudioSubTapUIDKey / DriftCompensationKey
K_TAPS, K_TAP_AUTOSTART, K_TAP_UID, K_DRIFT = "taps", "tapautostart", "uid", "drift"
CA_TAP_MUTED = 1  # CATapMuteBehavior: CATapUnmuted 0, CATapMuted 1, CATapMutedWhenTapped 2 (CATapDescription.h)


class _AudioBuffer(ctypes.Structure):
    _fields_ = [("channels", ctypes.c_uint32), ("size", ctypes.c_uint32), ("data", ctypes.c_void_p)]


class _BufferList(ctypes.Structure):
    _fields_ = [("n", ctypes.c_uint32), ("bufs", _AudioBuffer * 16)]  # mNumberBuffers is the real length


# AudioDeviceIOProc (AudioHardware.h)
_IOPROC = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_uint32, ctypes.c_void_p, ctypes.POINTER(_BufferList),
                           ctypes.c_void_p, ctypes.POINTER(_BufferList), ctypes.c_void_p, ctypes.c_void_p)


class _ObjC:
    """The few Objective-C messages a CATapDescription needs, through the runtime's C API.
    Every message is checked with respondsToSelector: first: an unknown selector would raise an
    Objective-C exception, which ctypes can't catch and which aborts the whole process."""

    def __init__(self):
        try:
            self.lib = ctypes.CDLL("/usr/lib/libobjc.A.dylib")
            ctypes.CDLL("/System/Library/Frameworks/Foundation.framework/Foundation")  # NSNumber, NSArray
        except OSError as e:
            raise Unavailable(f"the Objective-C runtime isn't available: {e}") from e
        vp = ctypes.c_void_p
        self.lib.objc_getClass.restype = vp
        self.lib.objc_getClass.argtypes = [ctypes.c_char_p]
        self.lib.sel_registerName.restype = vp
        self.lib.sel_registerName.argtypes = [ctypes.c_char_p]
        self._msg = ctypes.cast(self.lib.objc_msgSend, vp).value
        self._protos = {}

    def cls(self, name):
        c = self.lib.objc_getClass(name.encode())
        if not c:
            raise Unavailable(f"{name} isn't available on this Mac")
        return c

    def _fn(self, restype, argtypes):
        key = (restype, tuple(argtypes))
        if key not in self._protos:
            self._protos[key] = ctypes.CFUNCTYPE(restype, ctypes.c_void_p, ctypes.c_void_p, *argtypes)(self._msg)
        return self._protos[key]

    def responds(self, obj, sel):
        return bool(self._fn(ctypes.c_bool, [ctypes.c_void_p])(obj, self.lib.sel_registerName(b"respondsToSelector:"),
                                                               self.lib.sel_registerName(sel.encode())))

    def send(self, restype, obj, sel, *args):
        """objc_msgSend(obj, sel, *values) with args as (ctype, value) pairs. Raises Unavailable instead
        of sending a message the object doesn't understand."""
        if not obj:
            raise Unavailable(f"no object for {sel}")
        if not self.responds(obj, sel):
            raise Unavailable(f"this macOS doesn't understand {sel}")
        return self._fn(restype, [t for t, _v in args])(obj, self.lib.sel_registerName(sel.encode()),
                                                        *[v for _t, v in args])


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

    def _array(self, items):
        arr = self.cf.CFArrayCreate(None, (ctypes.c_void_p * len(items))(*items), len(items), self._array_values)
        for x in items:
            self.cf.CFRelease(x)
        return arr

    def create_aggregate(self, name, uid, sub_uid, tap_uid=None):
        """-> (OSStatus, new device id). A public aggregate with one sub-device, `sub_uid`, and with
        `tap_uid` the process tap whose audio is its input (drift-compensated, started with the device)."""
        subs = self._array([self._dict([(K_SUB_UID, self._str(sub_uid))])])
        pairs = [(K_NAME, self._str(name)), (K_UID, self._str(uid)), (K_PRIVATE, self._num(0)),
                 (K_STACKED, self._num(0)), (K_MAIN, self._str(sub_uid)), (K_SUBS, subs)]
        if tap_uid:
            taps = self._array([self._dict([(K_TAP_UID, self._str(tap_uid)), (K_DRIFT, self._num(1))])])
            pairs += [(K_TAPS, taps), (K_TAP_AUTOSTART, self._num(1))]
        desc = self._dict(pairs)
        dev = ctypes.c_uint32(0)
        try:
            err = self.ca.AudioHardwareCreateAggregateDevice(desc, ctypes.byref(dev))
        finally:
            self.cf.CFRelease(desc)
        return err, dev.value

    def destroy_aggregate(self, dev):
        return self.ca.AudioHardwareDestroyAggregateDevice(dev)

    # --- process taps (macOS 14.2+) -----------------------------------------------------

    def _tap_api(self):
        if not hasattr(self, "_objc"):
            if not hasattr(self.ca, "AudioHardwareCreateProcessTap"):
                raise Unavailable("this macOS has no process taps (14.2+)")
            self.ca.AudioHardwareCreateProcessTap.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)]
            self.ca.AudioHardwareCreateProcessTap.restype = ctypes.c_int32
            self.ca.AudioHardwareDestroyProcessTap.argtypes = [ctypes.c_uint32]
            self.ca.AudioHardwareDestroyProcessTap.restype = ctypes.c_int32
            self._objc = _ObjC()
        return self._objc

    def process_object(self, pid):
        """The CoreAudio process object of `pid` (kAudioHardwarePropertyTranslatePIDToProcessObject), or 0."""
        a = _Addr(_fcc("id2p"), GLOBAL, 0)
        q = ctypes.c_int32(pid)
        v = ctypes.c_uint32(0)
        n = ctypes.c_uint32(4)
        if self.ca.AudioObjectGetPropertyData(SYSTEM, ctypes.byref(a), 4, ctypes.byref(q), ctypes.byref(n),
                                              ctypes.byref(v)):
            return 0
        return v.value

    def default_output_uid(self):
        """The UID of the default output device (kAudioHardwarePropertyDefaultOutputDevice), or ""."""
        dev = self._u32(SYSTEM, "dOut")
        return self._string(dev, "uid ") if dev else ""

    def create_tap(self, name, process):
        """-> (OSStatus, tap id, tap UUID string). A public, muting, stereo-mixdown tap of one process.
        (A private tap gives a public aggregate no input channels.)"""
        oc = self._tap_api()
        # alloc/init (not the autoreleasing class methods): this runs on a worker thread without a pool
        new = lambda cls: oc.send(ctypes.c_void_p, oc.cls(cls), "alloc")  # noqa: E731
        num = oc.send(ctypes.c_void_p, new("NSNumber"), "initWithUnsignedInt:", (ctypes.c_uint32, process))
        try:
            arr = oc.send(ctypes.c_void_p, new("NSArray"), "initWithObjects:count:",
                          (ctypes.POINTER(ctypes.c_void_p), (ctypes.c_void_p * 1)(num)), (ctypes.c_ulong, 1))
        finally:
            oc.send(None, num, "release")
        try:
            desc = oc.send(ctypes.c_void_p, new("CATapDescription"), "initStereoMixdownOfProcesses:",
                           (ctypes.c_void_p, arr))
        finally:
            oc.send(None, arr, "release")
        if not desc:
            raise Unavailable("CATapDescription didn't initialise")
        try:
            oc.send(None, desc, "setMuteBehavior:", (ctypes.c_long, CA_TAP_MUTED))
            oc.send(None, desc, "setPrivate:", (ctypes.c_bool, False))
            cfname = self._str(name)
            try:
                oc.send(None, desc, "setName:", (ctypes.c_void_p, cfname))  # CFString is toll-free an NSString
            finally:
                self.cf.CFRelease(cfname)
            uuid = oc.send(ctypes.c_void_p, oc.send(ctypes.c_void_p, desc, "UUID"), "UUIDString")
            out = ctypes.create_string_buffer(64)
            self.cf.CFStringGetCString(uuid, out, 64, UTF8)
            tap = ctypes.c_uint32(0)
            err = self.ca.AudioHardwareCreateProcessTap(desc, ctypes.byref(tap))
            return err, tap.value, out.value.decode()
        finally:
            oc.send(None, desc, "release")

    def destroy_tap(self, tap):
        self._tap_api()
        return self.ca.AudioHardwareDestroyProcessTap(tap)

    def taps(self):
        """[(id, uuid, name)] for every process tap on this Mac (any process's public ones)."""
        ids = self._get(SYSTEM, "tps#")
        return [(t, self._string(t, "tuid"), self._string(t, "lnam")) for t in self._u32s(ids)]

    def measure_input(self, dev, secs, amp, freq=440.0, enough_ms=60):
        """Runs `dev` for up to `secs`: plays a sine of `amp` into its output (from this process) and
        measures what its input receives. Stops early once `enough_ms` of the tone have arrived.
        -> {"callbacks", "rms", "first_ms", "ms"}: rms of the input after the first signal (0 = silence)."""
        rate_b = self._get(dev, "nsrt")
        rate = struct.unpack("d", rate_b[:8])[0] if rate_b and len(rate_b) >= 8 else 48000.0
        st = {"callbacks": 0, "sq": 0.0, "n": 0, "first": None, "phase": 0.0, "heard": 0}
        done = threading.Event()
        step = 2 * math.pi * freq / rate
        t0 = time.monotonic()
        threshold = amp * 0.2

        def cb(_dev, _now, inp, _it, out, _ot, _cd):
            try:
                st["callbacks"] += 1
                frames = 0
                if out:
                    ol = out.contents
                    for i in range(min(ol.n, 16)):
                        b = ol.bufs[i]
                        ch = max(1, b.channels)
                        frames = b.size // 4 // ch
                        if not b.data or not frames:
                            continue
                        arr = (ctypes.c_float * (frames * ch)).from_address(b.data)
                        ph = st["phase"]
                        for f in range(frames):
                            s = amp * math.sin(ph + f * step)
                            for k in range(ch):
                                arr[f * ch + k] = s
                    st["phase"] = (st["phase"] + frames * step) % (2 * math.pi)
                if inp:
                    il = inp.contents
                    for i in range(min(il.n, 16)):
                        b = il.bufs[i]
                        count = b.size // 4
                        if not b.data or not count:
                            continue
                        arr = (ctypes.c_float * count).from_address(b.data)
                        sq = sum(x * x for x in arr)
                        if st["first"] is None and max(abs(x) for x in arr) > threshold:
                            st["first"] = time.monotonic() - t0
                        if st["first"] is not None:
                            st["sq"] += sq
                            st["n"] += count
                            st["heard"] += count / max(1, b.channels)
                    if st["heard"] >= rate * enough_ms / 1000:
                        done.set()
            except Exception:  # noqa: BLE001 — never raise into the HAL's IO thread
                pass
            return 0

        proc = _IOPROC(cb)
        pid = ctypes.c_void_p(0)
        ca = self.ca
        ca.AudioDeviceCreateIOProcID.argtypes = [ctypes.c_uint32, _IOPROC, ctypes.c_void_p,
                                                 ctypes.POINTER(ctypes.c_void_p)]
        ca.AudioDeviceStart.argtypes = ca.AudioDeviceStop.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
        ca.AudioDeviceDestroyIOProcID.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
        for f in (ca.AudioDeviceCreateIOProcID, ca.AudioDeviceStart, ca.AudioDeviceStop, ca.AudioDeviceDestroyIOProcID):
            f.restype = ctypes.c_int32
        err = ca.AudioDeviceCreateIOProcID(dev, proc, None, ctypes.byref(pid))
        if err:
            raise Unavailable(f"can't run the check device (CoreAudio error {err})")
        try:
            err = ca.AudioDeviceStart(dev, pid)
            if err:
                raise Unavailable(f"can't start the check device (CoreAudio error {err})")
            done.wait(secs)
            ca.AudioDeviceStop(dev, pid)
        finally:
            ca.AudioDeviceDestroyIOProcID(dev, pid)
        return {"callbacks": st["callbacks"], "rms": math.sqrt(st["sq"] / st["n"]) if st["n"] else 0.0,
                "first_ms": None if st["first"] is None else round(st["first"] * 1000, 1),
                "ms": round((time.monotonic() - t0) * 1000, 1)}


PROBE_S = 1.5        # the tap check: at most this long for the tone to come through
PROBE_AMP = 1e-4     # -80 dBFS: inaudible even if the tap doesn't mute it


class StickMicDevice:
    """Creates / removes the aggregate. Thread-safe (one lock); every call is a few
    CoreAudio calls, timed and logged. `backend`: a CoreAudioDevices-like object, or a
    factory for one (made on first use, so creating this never touches CoreAudio).
    `mode`: the carrier, TAP or BLACKHOLE (the agent sets it, see pick_carrier())."""

    def __init__(self, backend=None, name=NAME, uid=UID, carrier=CARRIER, own_pid=None, mode=BLACKHOLE):
        is_factory = backend is None or isinstance(backend, type) or not hasattr(backend, "devices")
        self._factory = (backend or CoreAudioDevices) if is_factory else None
        self._backend = None if is_factory else backend
        self.name, self.uid, self.carrier = name, uid, carrier
        self.mode = mode
        self.own_pid = os.getpid() if own_pid is None else own_pid
        self.dev = None           # its CoreAudio id while it exists
        self.carrier_dev = None   # its sub-device's id (BlackHole, or the output device in tap mode)
        self.tap = None           # our process tap's id while it exists (tap mode)
        self.dev_mode = None      # the carrier the device was made with
        self.baseline = set()     # processes already reading the sub-device itself when we made the device
        self.error = ""
        self.create_ms = self.destroy_ms = None  # the last ones
        self.check = {"state": None, "detail": "", "at": None}  # the last probe() (tap mode)
        self._lock = threading.Lock()

    @property
    def tap_name(self):
        return self.uid + TAP_SUFFIX

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

    @property
    def tap_live(self):
        """Our tap exists (so this process's sound is muted and goes to the device): the only time
        wispr_mic.py lets the stick's audio out in tap mode."""
        return self.mode == TAP and self.tap is not None and self.dev is not None

    def ensure_present(self):
        """The device exists: -> "created" (just now), "present" (it already was) or "" (it can't be)."""
        with self._lock:
            try:
                ca = self._ca()
                devices = ca.devices()
                ours = [(d, name) for d, name, uid in devices if uid == self.uid]
                # (a BlackHole one left by an earlier run is fine to reuse; a tapped one needs our own tap)
                same = (self.dev_mode in (None, BLACKHOLE) if self.mode == BLACKHOLE
                        else self.dev_mode == TAP and self.tap is not None)
                if ours and same and all(name == self.name for _d, name in ours):
                    self.dev = ours[0][0]
                    return "present"
                if ours:  # renamed in the settings, another carrier, or left by an earlier run: make it again
                    self._remove_all("removed the old", [d for d, _n in ours], ca)
                if self.mode == TAP:
                    return self._create_tapped(ca, devices)
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
                return self._made(dev, t, self.carrier)
            except Exception as e:  # noqa: BLE001 — ctypes land: never take the agent down
                self.dev = None
                self._set_error(f"can't create the {self.name} device: {e}")
                return ""

    def _made(self, dev, t, what):
        self.create_ms = round((time.monotonic() - t) * 1000, 1)
        self.dev = dev
        self.dev_mode = self.mode
        self._set_error("")
        log.info("stick mic: %s created in %.1f ms (id %d, on %s)", self.name, self.create_ms, dev, what)
        return "created"

    def _new_tap(self, ca, name):
        """-> (tap id, uuid, output uid) or raises Unavailable."""
        process = ca.process_object(self.own_pid)
        if not process:
            raise Unavailable("macOS has no audio process object for the agent")
        out_uid = ca.default_output_uid()
        if not out_uid:
            raise Unavailable("there's no default output device")
        err, tap, uuid = ca.create_tap(name, process)
        if err or not tap:
            raise Unavailable(f"can't create the audio tap (CoreAudio error {err})")
        return tap, uuid, out_uid

    def _create_tapped(self, ca, devices):
        t = time.monotonic()
        try:
            tap, uuid, out_uid = self._new_tap(ca, self.tap_name)
        except Unavailable as e:
            self.dev = None
            self._set_error(f"can't create the {self.name} device: {e}")
            return ""
        out = next((d for d, _n, uid in devices if uid == out_uid), None)
        self.carrier_dev = out
        self.baseline = self._readers(ca, out) if out is not None else set()
        err, dev = ca.create_aggregate(self.name, self.uid, out_uid, tap_uid=uuid)
        if err or not dev:
            ca.destroy_tap(tap)
            self.dev = None
            self._set_error(f"can't create the {self.name} device (CoreAudio error {err})")
            return ""
        self.tap = tap
        return self._made(dev, t, f"the Apple audio tap {tap}, clocked by {out_uid}")

    def remove(self):
        """Destroys every device with our UID (ours and any left over), and our tap. -> how many devices."""
        with self._lock:
            return self._remove_all("removed")

    def cleanup_stale(self):
        """remove() for start / disconnect / exit: a crashed agent leaves its device and tap behind
        (and an interrupted check its test device)."""
        with self._lock:
            n = self._remove_all("removed")
            self._remove_check_leftovers()
            self._remove_taps(lambda name: name in (self.tap_name, self.uid + CHECK_SUFFIX + TAP_SUFFIX))
            return n

    def _remove_taps(self, match):
        try:
            ca = self._ca()
            if not hasattr(ca, "taps"):
                return
            for t, _uuid, name in ca.taps():
                if match(name):
                    err = ca.destroy_tap(t)
                    log.info("stick mic: removed a left-over audio tap %d (%s)%s", t, name,
                             f": CoreAudio error {err}" if err else "")
        except Exception as e:  # noqa: BLE001 — older macOS: no taps to list
            log.debug("stick mic: can't list audio taps: %s", e)

    def _remove_check_leftovers(self):
        try:
            ca = self._ca()
            for d, _n, uid in ca.devices():
                if uid == self.uid + CHECK_SUFFIX:
                    ca.destroy_aggregate(d)
        except Exception:  # noqa: BLE001
            pass

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
        if self.tap is not None:  # after the device: it lists the tap
            try:
                err = ca.destroy_tap(self.tap)
                if err:
                    log.warning("stick mic: can't remove the audio tap %d (CoreAudio error %d)", self.tap, err)
            except Exception as e:  # noqa: BLE001
                log.warning("stick mic: can't remove the audio tap: %s", e)
            self.tap = None
        return n

    def _readers(self, ca, dev):
        pids = ca.input_users(dev)
        return None if pids is None else {p for p in pids if p != self.own_pid}

    def recording(self):
        """Someone (Wispr) is reading the device right now: True / False, or None when we can't
        tell. macOS may report an aggregate's readers on its sub-device, so a process that was
        already reading that one before the device appeared makes it ambiguous."""
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

    # --- is the tap allowed? ------------------------------------------------------------

    def probe(self, secs=PROBE_S, amp=PROBE_AMP):
        """Proves the tap path works before any stick audio goes through it: a test-named tap +
        aggregate ("…-check", never the real name / UID), a -80 dBFS tone from this process into
        it, and its input must hear the tone. Without the System Audio Recording permission (or
        while macOS still asks) a tap hears silence — and doesn't mute either. The first call is
        what makes macOS ask. -> self.check: {"state": "ok" | "failed" | "error", "detail", ...}."""
        with self._lock:
            t0 = time.monotonic()
            res = {"state": "error", "detail": "", "at": time.time(), "create_ms": None, "first_ms": None,
                   "level": None, "ms": None}
            ca = tap = dev = None
            try:
                ca = self._ca()
                if not hasattr(ca, "create_tap"):
                    raise Unavailable("this backend has no process taps")
                tap, uuid, out_uid = self._new_tap(ca, self.uid + CHECK_SUFFIX + TAP_SUFFIX)
                err, dev = ca.create_aggregate(self.name + "-check", self.uid + CHECK_SUFFIX, out_uid, tap_uid=uuid)
                if err or not dev:
                    dev = None
                    raise Unavailable(f"can't create the check device (CoreAudio error {err})")
                res["create_ms"] = round((time.monotonic() - t0) * 1000, 1)
                m = ca.measure_input(dev, secs, amp)
                res["first_ms"], res["level"] = m.get("first_ms"), m.get("rms")
                expected = amp / math.sqrt(2)
                if m.get("first_ms") is not None and (m.get("rms") or 0) >= expected * 0.25:
                    res["state"], res["detail"] = "ok", "the Apple audio tap works"
                elif not m.get("callbacks"):
                    res["detail"] = "the check device didn't run"
                else:
                    res["state"] = "failed"
                    res["detail"] = f"the tap heard nothing — {PERMISSION_HINT}"
            except Unavailable as e:
                res["detail"] = str(e)
            except Exception as e:  # noqa: BLE001 — ctypes land
                res["detail"] = f"the tap check failed: {e}"
            finally:
                for what, fn, x in (("device", "destroy_aggregate", dev), ("tap", "destroy_tap", tap)):
                    if x is not None and ca is not None:
                        try:
                            getattr(ca, fn)(x)
                        except Exception as e:  # noqa: BLE001
                            log.warning("stick mic: can't remove the check %s: %s", what, e)
            res["ms"] = round((time.monotonic() - t0) * 1000, 1)
            self.check = res
            log.info("stick mic: tap check %s in %.0f ms (created in %s ms, tone after %s ms, level %s): %s",
                     res["state"], res["ms"], res["create_ms"], res["first_ms"],
                     None if res["level"] is None else f"{res['level']:.1e}", res["detail"])
            return res


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

    def __init__(self, device, send, allowed=None, settings=None, emit=None, clock=time.monotonic, removed=None):
        self.device = device
        self._removed = removed or (lambda: None)  # called after the device was removed
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
        self.checking = False    # the tap check is running

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
            self._removed()
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
        n = await self._run(self.device.remove)
        self._removed()
        if n:
            log.info("stick mic: removed now (%s)", why)
            self._report()

    async def check(self):
        """The tap check (StickMicDevice.probe), in turn with the device calls; never during a hold
        or while the device is there (the check mutes this process for its ~0.2-1.5 s).
        -> the result, or None when it can't run now."""
        if self.holding or self.device.present or self.clock() < self.keep_until:
            return None
        self.checking = True
        self._report()
        try:
            return await self._run(self.device.probe)
        finally:
            self.checking = False
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
                "holding": self.holding, "last": self.last, "carrier": self.device.mode,
                "tap_live": self.device.tap_live, "check": {**self.device.check, "running": self.checking}}


