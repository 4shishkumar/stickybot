"""Updates the stick's firmware over the Bluetooth link the agent already has
(no USB cable, no Wi-Fi). See PROTOCOL.md, "Firmware update over Bluetooth".

Only the app image is sent (app.bin of the bundle from flasher.bundled(); the
bootloader, partition table and settings are never touched). The stick writes
it to its other OTA slot, checks its SHA-256 against the manifest, reboots into
it and confirms it after ~25 s of running — a firmware that doesn't get that far
rolls back by itself. Firmware 1.8.0+ ("ota":1 in its @state); older sticks need
one USB update first.

Transfer: `begin` (JSON) -> "ready" with the offset to start at (0, or where an
interrupted transfer got to) -> packets [offset u32 LE][bytes] written without
response, paced by Core Bluetooth's canSendWriteWithoutResponse, at most `window`
bytes past the stick's last ack -> `end` -> "done" -> the stick restarts.
A dropped packet makes the stick ask for a resend ("gap"); a dropped link is
resumed when it comes back.

Security: the link is the bonded, encrypted one (the characteristic needs it);
the image must match the manifest's sha256 (checked here and on the stick);
older / same versions need `force`. Signing: see verify_signature().
"""
import asyncio
import hashlib
import logging
import struct
import time
from pathlib import Path

from . import flasher

log = logging.getLogger("ota")

APP_OFFSET = "0x10000"
HEADER = 4                 # [offset u32] in front of each packet
ACK_WAIT_S = 1.0           # no ack this long with the window full: resend from the last ack
STALL_S = 20.0             # no progress at all this long (link up): give up
RECONNECT_S = 120.0        # link down this long: give up (the stick keeps the transfer ~3 min)
REPLY_S = 6.0              # begin / cancel replies
END_REPLY_S = 20.0         # end: the stick reads the image back for the hash (~1-2 s)
GAP_DEBOUNCE_S = 0.5       # a repeated "gap" for the offset we just rewound to
REBOOT_S = 90.0            # after "done": wait this long for the stick to come back

REASONS = {  # the stick's refusal / error codes -> words for the dashboard
    "busy": "the stick is busy ({why}) — try again when it's idle",
    "battery": "the stick's battery is below 30 % — plug it in to charge, then try again",
    "pending": "the stick has just been updated and is still checking the new firmware — try again in half a minute",
    "unsupported": "the stick can't take a compressed image",
    "size": "the image doesn't fit the stick's update slot",
    "sha": "the image's checksum didn't match (nothing was changed on the stick)",
    "downgrade": "that's an older firmware than the stick's — use force to install it anyway",
    "same": "the stick already runs this version — use force to install it again",
    "image": "the stick refused the image (not a valid M5StickS3 firmware)",
    "open": "the stick couldn't start writing its update slot",
    "write": "writing the stick's flash failed",
    "read": "reading the image back on the stick failed",
    "timeout": "the stick gave up waiting for data",
    "not_started": "the stick had no transfer running",
}


class OtaError(Exception):
    pass


class OtaCancelled(OtaError):
    pass


def reason(msg):
    code = msg.get("error") or "?"
    return REASONS.get(code, f"the stick said {code}").format(why=msg.get("why") or "busy")


def verify_signature(data, manifest):
    """Hook for signed firmware (later): return False to refuse an image. Today the
    sha256 from the bundled manifest (and the bonded link) is all there is."""
    return True


def bundled_app():
    """The app image from the bundle: {"data", "sha256", "size", "version"}. Raises OtaError."""
    fw = flasher.bundled()
    if not fw:
        raise OtaError("no bundled firmware — run `make -C m5-stick-s3 release`")
    part = next((p for p in fw.get("parts") or [] if p.get("offset") == APP_OFFSET or p.get("file") == "app.bin"),
                None)
    path = dict(fw.get("files") or []).get(part["offset"]) if part else None
    if not part or not path:
        raise OtaError("the bundled firmware has no app image")
    data = Path(path).read_bytes()
    sha = hashlib.sha256(data).hexdigest()
    if part.get("sha256") and sha != part["sha256"]:
        raise OtaError("the bundled app image doesn't match its manifest (sha256) — refusing")
    if part.get("size") and len(data) != part["size"]:
        raise OtaError("the bundled app image doesn't match its manifest (size) — refusing")
    if data[:1] != b"\xe9":
        raise OtaError("the bundled app.bin isn't an ESP32 app image — refusing")
    if not verify_signature(data, fw):
        raise OtaError("the bundled firmware's signature doesn't verify — refusing")
    return {"data": data, "sha256": sha, "size": len(data), "version": fw.get("version") or "?"}


def supported(state):
    """The stick's @state says it takes updates over Bluetooth (firmware 1.8.0+)."""
    return bool(state) and int(state.get("ota") or 0) >= 1


def check_versions(new, current, force=False):
    """None, or why the update shouldn't run (older / same) without force."""
    if force or not current:
        return None
    n, c = flasher.version_tuple(new), flasher.version_tuple(current)
    if n < c:
        return f"the stick runs {current}, newer than {new} — use force to downgrade"
    if n == c:
        return f"the stick already runs {current} — use force to install it again"
    return None


class Transfer:
    """Sends one file over the OTA characteristic with the stick's flow control: a `begin`
    message, packets [offset u32 LE][bytes] at most `window` bytes past the last ack, resends
    after a gap or a silent window, `end`, then "done". Firmware (Sender, below) and face
    packages (faces.Sender) both use it; they differ in the message family (KIND: the key the
    agent sends and the stick's "evt"), the begin message, the stick's error words and the
    exception classes.

    `link` needs .connected, async .send(dict), async .ota_write(bytes) and .ota_max_write();
    stick messages are fed in with on_message(msg) (register it as a listener). emit(event)
    gets {"line"} / {"progress","stage"} events."""

    KIND = "ota"
    Error = OtaError
    Cancelled = OtaCancelled
    WORDS = {"no_answer": "the stick didn't answer", "stalled": "the stick stopped answering",
             "unconfirmed": "the stick didn't confirm the file", "failed": "the transfer failed"}

    def __init__(self, link, emit=None, clock=time.monotonic):
        self.link = link
        self.emit = emit or (lambda e: None)
        self.clock = clock
        self.inbox = asyncio.Queue()
        self.cancelled = asyncio.Event()
        self.stats = {}
        self._pct = -1
        self.done = None  # the stick's "done" message, after run()

    def on_message(self, msg):
        if msg.get("src") == "agent" and msg.get("evt") == self.KIND:
            self.inbox.put_nowait(msg)

    def cancel(self):
        self.cancelled.set()

    def begin_message(self, img, force):
        """What goes under KIND to start (or resume) the transfer."""
        raise NotImplementedError

    def reason(self, msg):
        """The stick's refusal / error message in words."""
        raise NotImplementedError

    def _progress(self, offset, size):
        pct = min(100, offset * 100 // max(1, size))
        if pct != self._pct:
            self._pct = pct
            self.emit({"progress": pct, "stage": "send"})

    async def _reply(self, states, timeout):
        """The next message whose state is in `states` (others — late acks — are skipped)."""
        deadline = self.clock() + timeout
        while True:
            left = deadline - self.clock()
            if left <= 0:
                return None
            try:
                msg = await asyncio.wait_for(self.inbox.get(), left)
            except TimeoutError:
                return None
            if msg.get("state") in states:
                return msg

    async def _wait_link(self, timeout):
        deadline = self.clock() + timeout
        while not self.link.connected:
            if self.cancelled.is_set():
                raise self.Cancelled("cancelled")
            if self.clock() > deadline:
                raise self.Error("lost the stick (Bluetooth) and it didn't come back")
            await asyncio.sleep(0.2)

    async def _begin(self, img, force):
        msg = {"src": "agent", self.KIND: self.begin_message(img, force)}
        for _ in range(3):
            if not await self.link.send(msg):
                await self._wait_link(RECONNECT_S)
                continue
            r = await self._reply({"ready", "refused", "error"}, REPLY_S)
            if r is None:
                continue
            if r["state"] != "ready":
                raise self.Error(self.reason(r))
            return r
        raise self.Error(self.WORDS["no_answer"])

    async def _abort(self):
        if self.link.connected:
            await self.link.send({"src": "agent", self.KIND: {"cancel": 1}})
            await self._reply({"cancelled"}, REPLY_S)

    async def run(self, img, force=False):
        """Sends img ({"data", "size", "sha256", ...}) and waits for "done". Returns stats, with
        the stick's "done" message as stats["done"]; raises self.Error / self.Cancelled."""
        size, data = img["size"], img["data"]
        t0 = self.clock()
        sent_bytes = resends = reconnects = 0
        done = None
        try:
            self.emit({"progress": 0, "stage": "send"})
            while True:  # one pass per connection (a dropped link resumes with a new begin)
                if self.cancelled.is_set():
                    raise self.Cancelled("cancelled")
                r = await self._begin(img, force)
                acked = sent = int(r.get("offset") or 0)
                window = int(r.get("window") or 16384)
                chunk = max(16, min(int(r.get("chunk") or 180), self.link.ota_max_write() - HEADER))
                if r.get("resumed"):
                    self.emit({"line": f"resuming at {acked * 100 // size}% ({acked} of {size} bytes)"})
                else:
                    self.emit({"line": f"sending {size // 1024} KB in {chunk}-byte packets, "
                                       f"window {window // 1024} KB"})
                last_progress = waited_from = self.clock()
                rewound = (-1, 0.0)
                dropped = False
                while acked < size:
                    if self.cancelled.is_set():
                        await self._abort()
                        raise self.Cancelled("cancelled")
                    # fill the window
                    while sent < size and sent - acked < window and self.inbox.empty():
                        pkt = struct.pack("<I", sent) + data[sent:sent + chunk]
                        if not await self.link.ota_write(pkt):
                            dropped = True
                            break
                        sent_bytes += len(pkt) - HEADER
                        sent = min(size, sent + chunk)
                    if dropped:
                        break
                    try:
                        timeout = ACK_WAIT_S if sent - acked >= window or sent >= size else 0.05
                        msg = await asyncio.wait_for(self.inbox.get(), timeout)
                    except TimeoutError:
                        msg = None
                    if msg is None:
                        if not self.link.connected:
                            dropped = True
                            break
                        full = sent - acked >= window or sent >= size
                        if full and self.clock() - waited_from >= ACK_WAIT_S:  # nothing back: resend unacked
                            resends += sent - acked
                            sent = acked
                            waited_from = self.clock()
                        if self.clock() - last_progress > STALL_S:
                            raise self.Error(self.WORDS["stalled"])
                        continue
                    st = msg.get("state")
                    if st == "ack":
                        off = int(msg.get("offset") or 0)
                        if off > acked:
                            acked = off
                            last_progress = waited_from = self.clock()
                            self._progress(acked, size)
                        if msg.get("gap"):
                            again = rewound[0] == off and self.clock() - rewound[1] < GAP_DEBOUNCE_S
                            if not again and off < sent:
                                resends += sent - off
                                sent = off
                                rewound = (off, self.clock())
                        sent = max(sent, acked)
                    elif st == "cancelled":
                        raise self.Cancelled("cancelled on the stick" if msg.get("by") == "stick" else "cancelled")
                    elif st == "error":
                        raise self.Error(self.reason(msg))
                if dropped:
                    reconnects += 1
                    self.emit({"line": f"Bluetooth dropped at {acked * 100 // size}% — "
                                       "waiting for the stick to resume"})
                    await self._wait_link(RECONNECT_S)
                    continue
                # everything acknowledged: the stick checks the hash and commits
                self.emit({"progress": 100, "stage": "send"})
                self.emit({"progress": 0, "stage": "verify"})
                if not await self.link.send({"src": "agent", self.KIND: {"end": 1}}):
                    reconnects += 1
                    await self._wait_link(RECONNECT_S)
                    continue
                r = await self._reply({"done", "error", "cancelled"}, END_REPLY_S)
                if r is None:
                    if not self.link.connected:  # the reply may be lost; a new begin says where it is
                        reconnects += 1
                        await self._wait_link(RECONNECT_S)
                        continue
                    raise self.Error(self.WORDS["unconfirmed"])
                if r["state"] == "error" and r.get("error") == "incomplete":
                    continue  # it's missing bytes after all: begin again resumes there
                if r["state"] == "cancelled":
                    raise self.Cancelled("cancelled on the stick")
                if r["state"] == "error":
                    raise self.Error(self.reason(r))
                done = r
                self.emit({"progress": 100, "stage": "verify"})
                break
        except self.Error:
            raise
        except Exception as e:  # noqa: BLE001 — Bluetooth errors
            raise self.Error(f"{self.WORDS['failed']}: {e}") from e
        secs = max(1e-6, self.clock() - t0)
        self.stats = {"bytes": size, "seconds": round(secs, 1), "kbps": round(size / 1024 / secs, 1),
                      "resent": resends, "reconnects": reconnects, "sent": sent_bytes}
        self.done = done
        log.info("%s: %d bytes in %.1f s = %.1f KB/s (%d bytes resent, %d reconnects)", self.KIND, size, secs,
                 size / 1024 / secs, resends, reconnects)
        self.emit({"line": f"sent {size // 1024} KB in {secs:.0f} s ({size / 1024 / secs:.1f} KB/s"
                           f"{f', {resends // 1024} KB resent' if resends >= 1024 else ''}"
                           f"{f', {reconnects} reconnects' if reconnects else ''}) — the stick verified it"})
        return self.stats


class Sender(Transfer):
    """Sends one firmware image (from bundled_app()): PROTOCOL.md, "Firmware update over Bluetooth"."""

    WORDS = {"no_answer": "the stick didn't answer the update request (firmware older than 1.8.0?)",
             "stalled": "the stick stopped answering during the update",
             "unconfirmed": "the stick didn't confirm the image", "failed": "update failed"}

    def begin_message(self, img, force):
        return {"begin": {"size": img["size"], "sha256": img["sha256"], "version": img["version"],
                          "compressed": False, "force": force}}

    def reason(self, msg):
        return reason(msg)


async def update(agent, query_state, emit=None, force=False, sender=None, reboot_s=REBOOT_S):
    """The whole update with the agent's link: checks the stick, sends the bundled app
    image, then waits for the stick to come back with the new version.
    query_state() -> the stick's @state dict (or None). Returns a summary dict."""
    emit = emit or (lambda e: None)
    link = agent.link
    if not link.connected:
        raise OtaError("the stick isn't connected by Bluetooth")
    img = bundled_app()
    st = await query_state()
    if not st:
        raise OtaError("the stick didn't answer (@state)")
    if not supported(st):
        raise OtaError(f"the stick's firmware {st.get('fw') or '?'} can't update over Bluetooth — "
                       "update it once over USB (1.8.0 or newer)")
    if st.get("otaPending"):
        raise OtaError(REASONS["pending"])
    why = check_versions(img["version"], st.get("fw"), force)
    if why:
        raise OtaError(why)
    emit({"line": f"updating {st.get('name') or 'the stick'} from {st.get('fw')} to {img['version']} over Bluetooth"})
    s = sender or Sender(link, emit)
    agent.listeners.append(s.on_message)
    try:
        stats = await s.run(img, force)
    finally:
        if s.on_message in agent.listeners:
            agent.listeners.remove(s.on_message)
    emit({"progress": 0, "stage": "reboot"})
    emit({"line": "the stick is restarting with the new firmware"})
    result = {**stats, "from": st.get("fw"), "to": img["version"], "version": None, "confirmed": False}
    # wait for it to go away and come back, then read its version
    deadline = time.monotonic() + reboot_s
    await asyncio.sleep(2)
    while time.monotonic() < deadline:
        if link.connected:
            now = await query_state()
            if now and flasher.version_tuple(now.get("fw")) == flasher.version_tuple(img["version"]):
                result["version"] = now.get("fw")
                if not now.get("otaPending"):
                    result["confirmed"] = True
                    break
            elif now and now.get("fw") and now.get("fw") == st.get("fw") and time.monotonic() > deadline - reboot_s / 2:
                result["version"] = now.get("fw")
                break
        await asyncio.sleep(2)
    emit({"progress": 100, "stage": "reboot"})
    if result["confirmed"]:
        emit({"line": f"done — the stick runs {result['version']} (confirmed; its settings and pairings are kept)"})
    elif result["version"] == img["version"]:
        emit({"line": f"the stick runs {result['version']}; it confirms the new firmware after ~25 s of running"})
    elif result["version"]:
        raise OtaError(f"the stick came back with {result['version']} — the new firmware didn't start and it "
                       "rolled back (nothing lost)")
    else:
        emit({"line": "the stick hasn't reconnected yet — it should be back in a few seconds"})
    return result


# --- `stickybot flash --ble`: asks the running agent (it owns the Bluetooth link) ---

def _api(path, body=None, timeout=10):
    import json
    import urllib.request

    from . import config
    req = urllib.request.Request(f"http://127.0.0.1:{config.PORT}{path}", method="POST" if body is not None else "GET",
                                 data=json.dumps(body).encode() if body is not None else None,
                                 headers={"Content-Type": "application/json", "X-Stick-Token": config.token()})
    return urllib.request.urlopen(req, timeout=timeout)


def cli(force=False, out=print):
    """`stickybot flash --ble [--force]`: the update runs in the agent; this follows its progress."""
    import json
    import urllib.error
    try:
        with _api("/api/firmware/ota", {"force": force}) as r:
            job = json.load(r)["job"]
    except urllib.error.HTTPError as e:
        try:
            msg = json.load(e).get("error")
        except ValueError:
            msg = str(e)
        out(f"failed: {msg}")
        return 1
    except OSError as e:
        out(f"the agent isn't running ({e}) — start it with: stickybot start")
        return 1
    result = None
    try:
        with _api(f"/api/firmware/jobs/{job}", timeout=None) as stream:
            for raw in stream:
                line = raw.decode(errors="replace").strip()
                if not line.startswith("data:"):
                    continue
                e = json.loads(line[5:])
                if "line" in e:
                    out(e["line"])
                elif "progress" in e:
                    out(f"\r  {e['stage']}: {e['progress']:3d}%", end="\n" if e["progress"] == 100 else "", flush=True)
                if e.get("done"):
                    result = e
                    break
    except KeyboardInterrupt:
        out("\ncancelling…")
        try:
            _api(f"/api/firmware/jobs/{job}/cancel", {}).close()
        except OSError:
            pass
        return 130
    if not result or not result.get("ok"):
        out(f"failed: {(result or {}).get('error') or 'lost the agent'}")
        return 1
    return 0
