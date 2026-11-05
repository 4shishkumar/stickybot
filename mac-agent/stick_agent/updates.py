"""Update checks: is there a stick-key release newer than this agent, or with newer
firmware than the connected stick's?

Checked once at start-up and then every 24 h (config "update_check", default on), on
the "update_channel" ("stable" = releases only, "beta" = prereleases too). A check
lists the releases (releases.py: gh or HTTPS) and downloads the latest release's
firmware into ~/.stick-agent/firmware/<version>/ when it isn't there yet (a few MB,
sha256-checked), so "Update via USB" works straight away.

What it tells you:
  - the dashboard: a banner with the release notes and an Update button, and
    Settings → Updates (channel, auto-check, Check now) — GET /api/update
  - the menu bar: "Update available: vX" (opens the dashboard) — status()["title"]
  - the stick: one "nice" toast per release ("Update vX ready — see the dashboard"),
    held back during quiet hours and while the stick is away; remembered across restarts

Updating: the agent updates itself in one click for install.sh installs (selfupdate.py;
a developer checkout gets instructions instead), and the stick's firmware goes on with
the USB flasher (firmware_api) or any updater registered below (Bluetooth OTA).

  GET  /api/update                     status() + the release notes, how this agent is
                                       installed ("install") and the firmware updaters
  POST /api/update/check               check now -> the same
  POST /api/update/agent               {"tag"?} (default: the latest) -> {"job"}
  POST /api/update/firmware/{updater}  -> {"job"}: a registered firmware updater
  GET  /api/update/jobs/{id}           SSE: {"line"} / {"progress","stage"} / {"done","ok","error"}
State: ~/.stick-agent/updates.json.
"""
import asyncio
import json
import logging
import time

from aiohttp import web

from . import __version__, config, events, flasher, releases, selfupdate

log = logging.getLogger("updates")

CHECK_EVERY_S = 24 * 3600
FIRST_CHECK_S = 20       # after start-up, once the rest is running
TICK_S = 60
STICK_FW_EVERY_S = 3600  # ask the connected stick for its firmware version this often (and on connect)
NOTICE = ("Update {tag} ready", "see the dashboard")


def state_path():
    return config.HOME / "updates.json"


def channel(cfg):
    return cfg.get("update_channel") if cfg.get("update_channel") in releases.CHANNELS else "stable"


def summary(rel):
    """The parts of a release the dashboard and the state file keep."""
    return {k: rel.get(k) for k in ("tag", "version", "prerelease", "name", "notes", "url", "published")}


class Updates:
    def __init__(self, agent, clock=time.time):
        self.agent = agent
        self.clock = clock
        self.checking = False
        self.ran = False           # checked since this agent started
        self.error = ""
        self.note = ""             # e.g. "no release yet — ..."
        self.stick_fw = None       # the connected stick's firmware version, when known
        self._stick_asked = 0.0
        self._lock = asyncio.Lock()
        s = self._load()
        self.checked_at = float(s.get("checked_at") or 0)
        self.latest = s.get("latest")          # summary() + "firmware" (its firmware version)
        self.notified = list(s.get("notified") or [])  # release tags already toasted on the stick

    # --- state -------------------------------------------------------------------------

    def _load(self):
        try:
            s = json.loads(state_path().read_text())
            return s if isinstance(s, dict) else {}
        except (OSError, ValueError):
            return {}

    def _save(self):
        try:
            state_path().write_text(json.dumps({"checked_at": self.checked_at, "latest": self.latest,
                                                "notified": self.notified[-20:]}, indent=2))
        except OSError as e:
            log.warning("couldn't save %s: %s", state_path(), e)

    # --- what's new ----------------------------------------------------------------------

    def status(self, notes=False):
        cfg = self.agent.cfg
        lat = self.latest if isinstance(self.latest, dict) else None
        agent_update = bool(lat and releases.newer(lat.get("version"), __version__))
        fw = (lat or {}).get("firmware")
        firmware_update = bool(fw and self.stick_fw and releases.newer(fw, self.stick_fw))
        tag = (lat or {}).get("tag")
        if agent_update:
            title = f"Update available: {tag}"
        elif firmware_update:
            title = f"Stick firmware {fw} available"
        else:
            title = ""
        out = {"enabled": cfg.get("update_check", True) is not False, "channel": channel(cfg),
               "checking": self.checking, "checked_at": self.checked_at or None, "error": self.error,
               "note": self.note, "current": {"agent": __version__, "firmware": self.stick_fw},
               "latest": ({k: v for k, v in lat.items() if notes or k != "notes"} if lat else None),
               "agent_update": agent_update, "firmware_update": firmware_update,
               "available": agent_update or firmware_update, "title": title}
        return out

    def due(self, now=None):
        """A check is due: on, and not yet since start-up or the last one was 24 h ago."""
        if self.agent.cfg.get("update_check", True) is False:
            return False
        now = self.clock() if now is None else now
        return not self.ran or now - self.checked_at >= CHECK_EVERY_S

    def _fetch(self, chan):
        """(blocking) -> (summary + firmware version, note)."""
        rel, src = releases.latest(chan)
        info = {**summary(rel), "firmware": None}
        if releases.MANIFEST in rel["assets"]:
            try:
                _folder, manifest = releases.download_firmware(rel, src, say=lambda line: log.info("%s", line))
                info["firmware"] = manifest.get("version")
            except releases.ReleaseError as e:
                log.warning("firmware of %s: %s", rel["tag"], e)
                return info, f"couldn't get its firmware: {e}"
        return info, ""

    async def check(self):
        """Checks now (one at a time) -> status()."""
        async with self._lock:
            self.checking = True
            chan = channel(self.agent.cfg)
            try:
                info, note = await asyncio.get_running_loop().run_in_executor(None, self._fetch, chan)
                self.latest, self.note, self.error = info, note, ""
                log.info("update check (%s): latest %s (firmware %s); this agent %s", chan, info["tag"],
                         info["firmware"] or "?", __version__)
            except releases.NoRelease as e:
                self.latest, self.note, self.error = None, str(e), ""
                log.info("update check (%s): %s", chan, e)
            except releases.ReleaseError as e:
                self.error = str(e)
                log.warning("update check failed: %s", e)
            except Exception as e:  # noqa: BLE001 — never take the agent down for this
                self.error = f"unexpected error: {e}"
                log.exception("update check failed")
            finally:
                self.checking = False
                self.ran = True
                self.checked_at = self.clock()
                self._save()
        await self.refresh_stick()
        st = self.status()
        self.agent.emit("update", **{k: st[k] for k in ("available", "title", "agent_update", "firmware_update",
                                                         "error", "note")})
        await self.notify()
        return self.status(notes=True)

    # --- the connected stick's firmware ------------------------------------------------------

    def link_changed(self, state):
        if state == "connected":
            self._stick_asked = 0.0  # ask again (it may have been flashed meanwhile)

    async def refresh_stick(self, force=False):
        stick = getattr(self.agent, "stick_api", None)
        if stick is None or not self.agent.link.connected:
            return
        if not force and self.clock() - self._stick_asked < STICK_FW_EVERY_S:
            return
        self._stick_asked = self.clock()
        try:
            info = await stick.connected()
        except Exception as e:  # noqa: BLE001
            log.info("stick version: %s", e)
            return
        if info and info.get("version"):
            self.stick_fw = info["version"]

    # --- the toast on the stick --------------------------------------------------------------

    async def notify(self):
        """One "nice" toast per release, once the stick is there and it isn't quiet hours."""
        st = self.status()
        tag = (st["latest"] or {}).get("tag")
        if not st["available"] or not tag or tag in self.notified:
            return None
        if not self.agent.link.connected or events.quiet_now(self.agent.cfg):
            return None  # later: the next tick tries again
        name = tag if st["agent_update"] else f"firmware {st['latest']['firmware']}"
        ev = await self.agent.notifier.notify("update", NOTICE[0].format(tag=name), NOTICE[1])
        self.notified.append(tag)  # also when the kind is off: don't keep trying
        self._save()
        return ev

    # --- the loop ------------------------------------------------------------------------------

    async def run(self):
        await asyncio.sleep(FIRST_CHECK_S)
        while True:
            try:
                if self.due():
                    await self.check()
                else:
                    await self.refresh_stick()
                    await self.notify()
            except Exception:  # noqa: BLE001
                log.exception("update loop")
            await asyncio.sleep(TICK_S)


# --- ways to put new firmware on the stick ----------------------------------------------------
#
# The dashboard's firmware panel (Stick -> Firmware; the banner's Update button leads there)
# always has the USB flasher ("Update via USB": firmware_api's /api/firmware/flash). Other ways
# register here and show up next to it, as buttons, whenever available() says they can run:
#
#   register_firmware_updater("ota", "Update over Bluetooth", available, run)
#     available(agent) -> "" if it can run now, else why not (shown under the button)
#     run(agent, fw, emit)  async; fw = flasher.bundled() (manifest + "files" [(offset, path)]);
#                           emit({"line": ...}) / emit({"progress": 0-100, "stage": ...});
#                           raise to fail (the message is shown)
#
# TODO(OTA): the Bluetooth OTA sender registers itself here — e.g. at the end of its module,
# imported by core.py — and nothing else needs to change.
FIRMWARE_UPDATERS = {}


def register_firmware_updater(uid, title, available, run):
    FIRMWARE_UPDATERS[uid] = {"id": uid, "title": title, "available": available, "run": run}


def firmware_updaters(agent):
    out = []
    for u in FIRMWARE_UPDATERS.values():
        try:
            why = u["available"](agent) or ""
        except Exception as e:  # noqa: BLE001
            why = f"error: {e}"
        out.append({"id": u["id"], "title": u["title"], "available": not why, "why": why})
    return out


def routes(agent):
    from .firmware_api import Job, stream_job  # (the flash jobs work the same way)
    jobs = {}

    def full():
        st = agent.updates.status(notes=True)
        st["install"] = selfupdate.install_info()
        st["firmware_updaters"] = firmware_updaters(agent)
        return st

    def start(work):
        if any(not j.done for j in jobs.values()):
            return None
        job = Job()
        jobs[job.id] = job

        async def go():
            ok, err = False, None
            try:
                await work(job.emit)
                ok = True
            except (selfupdate.UpdateError, flasher.FlashError, releases.ReleaseError) as e:
                err = str(e)
            except Exception as e:  # noqa: BLE001
                log.exception("update job failed")
                err = f"unexpected error: {e}"
            log.info("update job %s: %s", job.id, "ok" if ok else err)
            job.emit({"done": True, "ok": ok, "error": err})
            job.done = True
        asyncio.ensure_future(go())
        return job

    async def get_update(_):
        return web.json_response(await asyncio.get_running_loop().run_in_executor(None, full))

    async def check_now(_):
        await agent.updates.check()
        return web.json_response(await asyncio.get_running_loop().run_in_executor(None, full))

    async def update_agent(request):
        try:
            body = await request.json()
        except ValueError:
            body = {}
        latest = agent.updates.status()["latest"] or {}
        tag = str(body.get("tag") or latest.get("tag") or "")
        if not tag:
            return web.json_response({"error": "no release to update to — check for updates first"}, status=400)
        info = await asyncio.get_running_loop().run_in_executor(None, selfupdate.install_info)
        if not info["can_update"]:
            return web.json_response({"error": info["why"], "install": info}, status=409)

        async def work(emit):
            emit({"line": f"updating stick-agent {__version__} -> {tag} in {info['path']}"})
            await selfupdate.update(tag, emit, restart=lambda: agent.restart_soon(1.0))
        job = start(work)
        if job is None:
            return web.json_response({"error": "an update is already running"}, status=409)
        return web.json_response({"job": job.id})

    async def update_firmware(request):
        u = FIRMWARE_UPDATERS.get(request.match_info["updater"])
        if u is None:
            return web.json_response({"error": "no such firmware updater"}, status=404)
        why = next(x["why"] for x in firmware_updaters(agent) if x["id"] == u["id"])
        if why:
            return web.json_response({"error": why}, status=409)
        fw = flasher.bundled()
        if not fw:
            return web.json_response({"error": flasher.NO_FIRMWARE}, status=409)

        async def work(emit):
            emit({"line": f"{u['title']}: firmware {fw['version']} ({flasher.describe(fw)})"})
            await u["run"](agent, fw, emit)
            await agent.updates.refresh_stick(force=True)
        job = start(work)
        if job is None:
            return web.json_response({"error": "an update is already running"}, status=409)
        return web.json_response({"job": job.id})

    async def job_events(request):
        job = jobs.get(request.match_info["id"])
        if not job:
            return web.json_response({"error": "no such job"}, status=404)
        return await stream_job(request, job)

    return [web.get("/api/update", get_update), web.post("/api/update/check", check_now),
            web.post("/api/update/agent", update_agent),
            web.post("/api/update/firmware/{updater}", update_firmware),
            web.get("/api/update/jobs/{id}", job_events)]
