"""Dashboard API for face packages (faces.py; docs/faces/CATALOG.md, "The dashboard").

routes(agent) returns the aiohttp routes; server.py adds them after firmware_api's (it uses
agent.stick_api to talk to the stick).

  GET  /api/faces/catalog[?refresh=1]    the catalog (cached; works offline), what the stick has
                                         and takes: {"url","faces","installed","stick","signed",
                                         "fetched","error"}
  GET  /api/faces/preview/{id}/{side}    a catalog face's preview (portrait | landscape), cached
  POST /api/faces/install                {"id"} (from the catalog) or {"file": base64, "name"}
                                         -> {"job"}
  GET  /api/faces/jobs/{id}              SSE: {"line"} / {"progress","stage"} / {"done","ok","error","result"}
  POST /api/faces/jobs/{id}/cancel       stops an install (the stick keeps what it had)
  POST /api/faces/remove                 {"id"} -> {"ok","id"}
  POST /api/faces/use                    {"id"} -> {"ok","id"}
"""
import asyncio
import base64
import binascii
import logging

from aiohttp import web

from . import faces
from .firmware_api import Job, stream_job

log = logging.getLogger("faces")


def routes(agent):
    jobs = {}
    agent.faces_busy = False  # an install owns the OTA characteristic (firmware_api checks it)

    def stick():
        return agent.stick_api  # firmware_api.Stick

    def catalog():
        return faces.Catalog(agent.cfg.get("faces_catalog_url") or faces.DEFAULT_CATALOG_URL)

    def err(text, status=400):
        return web.json_response({"error": text[:1].upper() + text[1:]}, status=status)

    async def ask(line):
        return await stick().ble(line)

    async def get_catalog(request):
        cat = catalog()
        have = await cat.refresh(force=request.query.get("refresh") == "1")
        info = {"connected": agent.link.connected, "supported": False, "fw": None, "needs": faces.MIN_FIRMWARE,
                "n": 0, "max": None, "free": None, "error": None, "busy": agent.faces_busy}
        support, on_stick = None, None
        if agent.link.connected and not stick().flashing:
            st = await stick().ble("@state")
            if st:
                support = faces.stick_support(st)
                info.update(supported=support["supported"], fw=support["fw"], fmt=support["fmt"], n=support["n"])
                if support["supported"]:
                    try:
                        lst = await faces.installed(ask)
                        on_stick = lst["faces"]
                        info.update(n=len(on_stick), max=lst.get("max"), free=lst.get("free"))
                    except faces.FaceError as e:
                        info["error"] = str(e)
                else:
                    info["error"] = faces.needs_firmware_text(support["fw"])
            else:
                info["error"] = "the stick didn't answer"
        listed, mine = faces.annotate(have["catalog"] if have else None, support, on_stick)
        for f in listed:  # previews come through the agent (cached, and offline)
            f["previews"] = {k: f"/api/faces/preview/{f['id']}/{k}" for k in f.get("previews") or {}}
        return web.json_response({
            "url": cat.url, "faces": listed, "installed": mine if on_stick is not None else None, "stick": info,
            "signed": bool(have and have["signed"]), "fetched": have and have["fetched"],
            "generated": have and have["catalog"].get("generated"), "error": cat.error,
            "offline": bool(cat.error and have)})

    async def preview(request):
        data = await catalog().preview(request.match_info["id"], request.match_info["side"])
        if not data:
            return web.Response(status=404)
        return web.Response(body=data, content_type="image/png", headers={"Cache-Control": "max-age=3600"})

    async def install(request):
        try:
            body = await request.json()
        except ValueError:
            return err("expected JSON")
        if agent.faces_busy or any(not j.done for j in jobs.values()):
            return err("a face is already being installed", 409)
        if stick().flashing:
            return err("the stick's firmware is being updated; try again when it's done", 409)
        if not agent.link.connected:
            return err("the stick isn't connected by Bluetooth", 409)
        cat = catalog()
        entry, blob = None, None
        if body.get("file"):
            try:
                raw = base64.b64decode(str(body["file"]), validate=True)
                blob = faces.read_upload(raw, str(body.get("name") or ""))
                faces.sfb_info(blob)
            except (binascii.Error, ValueError):
                return err("couldn't read the file")
            except faces.FaceError as e:
                return err(str(e))
        else:
            fid = str(body.get("id") or "")
            entry = cat.entry(fid) or (await cat.refresh() and cat.entry(fid))
            if not entry:
                return err(f"{fid or 'that face'} isn't in the faces catalog", 404)
        job = Job()
        jobs[job.id] = job
        sender = faces.Sender(agent.link, job.emit)
        job.cancel = sender.cancel

        async def work():
            ok, error, result = False, None, None
            agent.faces_busy = True
            try:
                data = blob
                if entry is not None:
                    job.emit({"progress": 0, "stage": "download"})
                    data = await cat.download(entry)
                    job.emit({"progress": 100, "stage": "download"})
                result = await faces.install(agent, ask, data, entry, job.emit, sender=sender)
                ok = True
            except faces.FaceError as e:
                error = str(e)
            except Exception as e:  # noqa: BLE001
                log.exception("face install failed")
                error = f"unexpected error: {e}"
            finally:
                agent.faces_busy = False
            log.info("face install %s: %s", (entry or {}).get("id") or "(file)", "ok" if ok else error)
            job.emit({"done": True, "ok": ok, "error": error and error[:1].upper() + error[1:], "result": result})
            job.done = True

        asyncio.ensure_future(work())
        return web.json_response({"job": job.id})

    async def job_events(request):
        job = jobs.get(request.match_info["id"])
        if not job:
            return err("no such job", 404)
        return await stream_job(request, job)

    async def job_cancel(request):
        job = jobs.get(request.match_info["id"])
        if not job:
            return err("no such job", 404)
        if job.done or not job.cancel:
            return err("nothing to cancel", 409)
        job.cancel()
        return web.json_response({"ok": True})

    async def remove(request):
        body = await request.json()
        if agent.faces_busy:
            return err("a face is being installed; try again when it's done", 409)
        try:
            r = await faces.remove(agent, str(body.get("id") or ""))
        except faces.FaceError as e:
            return err(str(e), 409)
        return web.json_response({"ok": True, **r})

    async def use(request):
        body = await request.json()
        try:
            r = await faces.use(ask, str(body.get("id") or ""))
        except faces.FaceError as e:
            return err(str(e), 409)
        return web.json_response({"ok": True, **r})

    return [
        web.get("/api/faces/catalog", get_catalog),
        web.get("/api/faces/preview/{id}/{side}", preview),
        web.post("/api/faces/install", install),
        web.get("/api/faces/jobs/{id}", job_events),
        web.post("/api/faces/jobs/{id}/cancel", job_cancel),
        web.post("/api/faces/remove", remove),
        web.post("/api/faces/use", use),
    ]
