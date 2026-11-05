"""Dashboard API for the stick itself: firmware (bundled build, USB flashing),
its settings (protocol commands over Bluetooth or USB) and onboarding.

routes(agent) returns the aiohttp routes; server.py adds them.

  GET  /api/firmware             bundled build, sticks on USB, the connected stick's version
  POST /api/firmware/flash       {"port", "backup"} -> {"job"}             (USB)
  POST /api/firmware/ota         {"force"} -> {"job"}                      (Bluetooth, stick 1.8.0+)
  GET  /api/firmware/jobs/{id}   SSE: {"line"} / {"progress","stage"} / {"done","ok","error"}
  POST /api/firmware/jobs/{id}/cancel   stops a Bluetooth update (the stick keeps its firmware)
  POST /api/stick/proto          {"line": "@state"} -> {"ok","via","reply","error"}
  GET  /api/onboarding           {"steps": {...}, "done"}
  POST /api/onboarding           {"step", "done"} -> same
"""
import asyncio
import json
import logging
import time
import uuid

from aiohttp import web

from . import config, flasher, ota

log = logging.getLogger("firmware")

PROTO_TIMEOUT_S = 5
USB_CACHE_S = 10
BLE_CACHE_S = 30
STEPS = ("firmware", "paired", "picked", "named", "repo", "tested")
DEFAULT_NAME = "StickyBot"  # a new or reset stick's name (firmware 1.9.0+)
DEFAULT_NAMES = {DEFAULT_NAME, "stick-key"}  # "still the default": older firmware's name counts too


# --- talking to the stick: "@..." protocol lines -----------------------------------

def usb_proto(port, line, timeout_s=2.5):
    """Sends one machine line over USB serial and returns the "@{...}" reply (dict) or None."""
    import serial
    try:
        with serial.Serial(port, 115200, timeout=0.2) as s:
            s.reset_input_buffer()
            s.write((line.strip() + "\n").encode())
            deadline, buf = time.time() + timeout_s, b""
            while time.time() < deadline:
                buf += s.read(4096)
                for raw in buf.split(b"\n"):
                    raw = raw.strip()
                    if raw.startswith(b"@{"):
                        try:
                            return json.loads(raw[1:])
                        except ValueError:
                            pass
    except (OSError, ValueError) as e:
        log.info("usb %s: %s", port, e)
    return None


class Stick:
    """Runs protocol lines on the stick: over the agent's Bluetooth link when
    connected (the stick answers {"evt":"proto"}), else over USB."""

    def __init__(self, agent):
        self.agent = agent
        self.flashing = False  # while a flash runs, don't open USB ports (it would disturb esptool)
        self.waiting = {}  # id -> future
        agent.listeners.append(self._on_message)
        self._usb_cache = (0.0, [])
        self._ble_cache = (0.0, None)

    def _on_message(self, msg):
        if msg.get("src") == "agent" and msg.get("evt") == "proto":
            fut = self.waiting.pop(msg.get("id"), None)
            if fut and not fut.done():
                fut.set_result(msg.get("reply"))

    async def ble(self, line):
        if not self.agent.link.connected:
            return None
        pid = "p" + uuid.uuid4().hex[:6]
        fut = asyncio.get_running_loop().create_future()
        self.waiting[pid] = fut
        try:
            if not await self.agent.link.send({"src": "agent", "proto": line, "id": pid}):
                return None
            return await asyncio.wait_for(fut, PROTO_TIMEOUT_S)
        except TimeoutError:
            return None
        finally:
            self.waiting.pop(pid, None)

    async def run(self, line):
        """-> {"ok", "via", "reply", "error"}"""
        line = (line or "").strip()
        if not line.startswith("@"):
            return {"ok": False, "via": None, "reply": None, "error": "expected a line like @state"}
        if self.agent.link.connected:
            reply = await self.ble(line)
            if reply is not None:
                self._ble_cache = (0.0, None)  # settings may have changed
                return {"ok": bool(reply.get("ok", True)), "via": "ble", "reply": reply, "error": reply.get("error")}
        ports = [p for p in flasher.espressif_ports() if not flasher.in_bootloader(p)]
        if self.flashing:
            return {"ok": False, "via": None, "reply": None, "error": "the stick is being flashed"}
        if len(ports) == 1:
            reply = await asyncio.get_running_loop().run_in_executor(None, usb_proto, ports[0]["port"], line)
            if reply is not None:
                return {"ok": bool(reply.get("ok", True)), "via": "usb", "reply": reply, "error": reply.get("error")}
            return {"ok": False, "via": "usb", "reply": None, "error": "the stick on USB didn't answer"}
        if len(ports) > 1:
            return {"ok": False, "via": None, "reply": None, "error": "several sticks on USB — connect one"}
        return {"ok": False, "via": None, "reply": None, "error": "no stick connected (Bluetooth or USB)"}

    async def usb_sticks(self):
        """Sticks on USB with their version (cached for a few seconds: opening the port is slow)."""
        at, cached = self._usb_cache
        if time.time() - at < USB_CACHE_S or self.flashing:
            return cached
        out = []
        for p in flasher.espressif_ports():
            entry = {"port": p["port"], "serial": p["serial"], "version": None, "board": None}
            if flasher.in_bootloader(p):
                entry["board"] = "download mode"
            else:
                st = await asyncio.get_running_loop().run_in_executor(None, usb_proto, p["port"], "@state", 1.5)
                if st:
                    entry["version"], entry["board"] = st.get("fw"), st.get("board")
            out.append(entry)
        self._usb_cache = (time.time(), out)
        return out

    async def connected(self):
        """{"name","version"} of the Bluetooth-linked stick, or None."""
        if not self.agent.link.connected:
            return None
        at, cached = self._ble_cache
        if cached and time.time() - at < BLE_CACHE_S:
            return cached
        st = await self.ble("@state")
        info = {"name": st.get("name"), "version": st.get("fw"), "ota": ota.supported(st)} if st else None
        self._ble_cache = (time.time(), info)
        return info


# --- flash jobs ----------------------------------------------------------------------

class Job:
    def __init__(self):
        self.id = uuid.uuid4().hex[:8]
        self.events = []
        self.done = False
        self.changed = asyncio.Event()
        self.cancel = None  # a Bluetooth update can be stopped

    def emit(self, e):
        self.events.append(e)
        self.changed.set()


async def stream_job(request, job):
    """A job's events as server-sent events, until it's done."""
    resp = web.StreamResponse(headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
    await resp.prepare(request)
    i = 0
    try:
        while True:
            while i < len(job.events):
                await resp.write(f"data: {json.dumps(job.events[i])}\n\n".encode())
                i += 1
            if job.done and i >= len(job.events):
                break
            job.changed.clear()
            try:
                await asyncio.wait_for(job.changed.wait(), 15)
            except TimeoutError:
                await resp.write(b": keep-alive\n\n")
    except (ConnectionResetError, asyncio.CancelledError):
        pass
    return resp


# --- onboarding ------------------------------------------------------------------------

def _history_has_voice():
    try:
        for line in config.HISTORY.read_text().splitlines():
            if '"source": "voice"' in line:
                return True
    except OSError:
        pass
    return False


def _repo_set():
    try:
        return "<org>" not in config.COMMANDS.read_text() and "<repo>" not in config.COMMANDS.read_text()
    except OSError:
        return False


async def onboarding_state(agent, stick):
    marks = dict(agent.cfg.get("onboarding") or {})
    fw = flasher.bundled()
    conn = await stick.connected()
    linked = bool(agent.cfg.get("stick")) and agent.link.connected
    if fw:
        versions = [conn["version"]] if conn and conn.get("version") else []
        versions += [u["version"] for u in await stick.usb_sticks() if u.get("version")]
        fw_ok = any(flasher.version_tuple(v) >= flasher.version_tuple(fw["version"]) for v in versions)
    else:
        fw_ok = True
    auto = {
        "firmware": fw_ok,
        "paired": linked,
        "picked": linked,
        "named": bool(conn and conn.get("name") and conn["name"] not in DEFAULT_NAMES),
        "repo": _repo_set(),
        "tested": _history_has_voice(),
    }
    steps = {k: bool(marks.get(k)) if k in marks else auto[k] for k in STEPS}
    return {"steps": steps, "done": all(steps.values())}


# --- routes ----------------------------------------------------------------------------

def routes(agent):
    stick = Stick(agent)
    agent.stick_api = stick  # the update check asks it for the connected stick's version
    jobs = {}

    async def firmware(_):
        fw = flasher.bundled()
        bundled = ({k: fw.get(k) for k in ("version", "board", "built", "sha256", "size", "source", "release")}
                   if fw else None)
        return web.json_response({"bundled": bundled, "usb": await stick.usb_sticks(),
                                  "connected": await stick.connected()})

    async def flash(request):
        body = await request.json()
        port = str(body.get("port") or "")
        try:
            flasher.check_port(port)
        except flasher.FlashError as e:
            return web.json_response({"error": str(e)}, status=400)
        if any(not j.done for j in jobs.values()):
            return web.json_response({"error": "a flash is already running"}, status=409)
        job = Job()
        jobs[job.id] = job

        async def work():
            ok, err = False, None
            stick.flashing = True
            try:
                await flasher.flash(port, bool(body.get("backup")), job.emit)
                ok = True
                stick._usb_cache = (0.0, [])
                stick._ble_cache = (0.0, None)
            except flasher.FlashError as e:
                err = str(e)
            except Exception as e:  # noqa: BLE001
                log.exception("flash failed")
                err = f"unexpected error: {e}"
            finally:
                stick.flashing = False
            log.info("flash %s: %s", port, "ok" if ok else err)
            job.emit({"done": True, "ok": ok, "error": err})
            job.done = True

        asyncio.ensure_future(work())
        return web.json_response({"job": job.id})

    async def ota_update(request):
        body = await request.json() if request.can_read_body else {}
        force = bool(body.get("force"))
        if any(not j.done for j in jobs.values()):
            return web.json_response({"error": "a flash is already running"}, status=409)
        if not agent.link.connected:
            return web.json_response({"error": "the stick isn't connected by Bluetooth"}, status=409)
        try:
            ota.bundled_app()
        except ota.OtaError as e:
            return web.json_response({"error": str(e)}, status=400)
        job = Job()
        jobs[job.id] = job
        sender = ota.Sender(agent.link, job.emit)
        job.cancel = sender.cancel

        async def work():
            ok, err, result = False, None, None
            stick.flashing = True
            try:
                result = await ota.update(agent, lambda: stick.ble("@state"), job.emit, force, sender=sender)
                ok = True
            except ota.OtaError as e:
                err = str(e)
            except Exception as e:  # noqa: BLE001
                log.exception("bluetooth update failed")
                err = f"unexpected error: {e}"
            finally:
                stick.flashing = False
                stick._ble_cache = (0.0, None)
            log.info("bluetooth update: %s", "ok" if ok else err)
            job.emit({"done": True, "ok": ok, "error": err, "result": result})
            job.done = True

        asyncio.ensure_future(work())
        return web.json_response({"job": job.id})

    async def job_cancel(request):
        job = jobs.get(request.match_info["id"])
        if not job:
            return web.json_response({"error": "no such job"}, status=404)
        if job.done or not job.cancel:
            return web.json_response({"error": "nothing to cancel"}, status=409)
        job.cancel()
        return web.json_response({"ok": True})

    async def job_events(request):
        job = jobs.get(request.match_info["id"])
        if not job:
            return web.json_response({"error": "no such job"}, status=404)
        return await stream_job(request, job)

    async def proto(request):
        body = await request.json()
        return web.json_response(await stick.run(str(body.get("line", ""))))

    async def get_onboarding(_):
        return web.json_response(await onboarding_state(agent, stick))

    async def post_onboarding(request):
        body = await request.json()
        step = body.get("step")
        marks = dict(agent.cfg.get("onboarding") or {})
        if step == "reset":
            marks = {}
        elif step in STEPS:
            marks[step] = bool(body.get("done"))
        else:
            return web.json_response({"error": f"step must be one of {', '.join(STEPS)} or reset"}, status=400)
        agent.update_config({"onboarding": marks})
        return web.json_response(await onboarding_state(agent, stick))

    return [
        web.get("/api/firmware", firmware),
        web.post("/api/firmware/flash", flash),
        web.post("/api/firmware/ota", ota_update),
        web.get("/api/firmware/jobs/{id}", job_events),
        web.post("/api/firmware/jobs/{id}/cancel", job_cancel),
        web.post("/api/stick/proto", proto),
        web.get("/api/onboarding", get_onboarding),
        web.post("/api/onboarding", post_onboarding),
    ]
