"""Dashboard API for the installer's components (see installer.py).

  GET  /api/components                      [{id, title, description, status, detail, needs, ...}]
  POST /api/components/{id}/install         -> {"job"}   (only components with from_dashboard)
  POST /api/components/{id}/uninstall       -> {"job"}
  GET  /api/components/jobs/{job}           SSE: {"line"} / {"progress","stage"} / {"done","ok","error"}

Components run in a worker thread (they download, call `gh`/`claude`, write files);
their output streams to the page. Nothing here asks questions: defaults are used.
"""
import asyncio
import json
import logging

from aiohttp import web

from . import config, installer
from .firmware_api import Job

log = logging.getLogger("components")

CONFIG_KEYS = {"github": "github", "wireless_mic": "wireless_mic"}  # component -> the config it changes


def routes(agent):
    jobs = {}

    async def listing(_):
        loop = asyncio.get_running_loop()
        infos = await loop.run_in_executor(None, lambda: [c.info() for c in installer.COMPONENTS])
        return web.json_response(infos)

    async def act(request):
        cid, action = request.match_info["id"], request.match_info["action"]
        comp = installer.BY_ID.get(cid)
        if comp is None:
            return web.json_response({"error": f"no component {cid!r}"}, status=404)
        if not comp.from_dashboard:
            return web.json_response({"error": f"{comp.title} can't be changed from the dashboard"}, status=400)
        if action == "uninstall" and not comp.removable:
            return web.json_response({"error": f"{comp.title} can't be removed"}, status=400)
        if any(not j.done for j in jobs.values()):
            return web.json_response({"error": "another component job is running"}, status=409)
        job = Job()
        jobs[job.id] = job
        loop = asyncio.get_running_loop()

        def emit(e):
            loop.call_soon_threadsafe(job.emit, e)

        def work():
            ui = installer.UI(interactive=False, emit=emit)
            try:
                (comp.install if action == "install" else comp.uninstall)(ui)
                return True, None
            except installer.NotAvailable as e:
                return False, str(e)
            except Exception as e:  # noqa: BLE001
                log.exception("%s %s failed", action, cid)
                return False, str(e)

        async def run():
            ok, err = await loop.run_in_executor(None, work)
            log.info("component %s %s: %s", cid, action, "ok" if ok else err)
            try:
                if cid in CONFIG_KEYS:  # pick up the change (wireless_mic: also when only BlackHole is missing)
                    agent.update_config({CONFIG_KEYS[cid]: config.load().get(CONFIG_KEYS[cid])})
            except Exception as e:  # noqa: BLE001 — the page still needs its "done"
                log.exception("applying %s failed", cid)
                ok, err = False, err or str(e)
            finally:
                job.emit({"done": True, "ok": ok, "error": err})
                job.done = True

        asyncio.ensure_future(run())
        return web.json_response({"job": job.id})

    async def job_events(request):
        job = jobs.get(request.match_info["job"])
        if not job:
            return web.json_response({"error": "no such job"}, status=404)
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

    return [
        web.get("/api/components", listing),
        web.post("/api/components/{id}/{action:install|uninstall}", act),
        web.get("/api/components/jobs/{job}", job_events),
    ]
