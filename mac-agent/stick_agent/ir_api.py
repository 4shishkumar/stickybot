"""Dashboard API for the stick's IR remote (ir.py; the dashboard's Remote page).

  GET  /api/ir                    {"connected","supported","why","fw","devices":[{id,n,buttons:[{name,proto,fav}]}],
                                   "max","free","brands":{id:{title,buttons}},"mapped":{dev:brand},"learn"}
  POST /api/ir/send               {"dev","btn","repeat"} | {"brand","btn"} | {"code"}
  POST /api/ir/learn              {"dev","btn","once"} -> starts; progress as "ir" events on /api/events
  POST /api/ir/learn/cancel
  POST /api/ir/rename             {"dev","to"} (a remote) | {"dev","btn","to"} (a button)
  POST /api/ir/remove             {"dev"} | {"dev","btn"}
  POST /api/ir/fav                {"dev","btn","on"}
  POST /api/ir/brand              {"dev","brand","install"}: map a remote to a code set (voice, ir_send), and
                                  with install: true put its buttons on the stick; brand "" unmaps
  GET  /api/ir/export             everything learned, codes included (a .json download)
  POST /api/ir/import             that format back onto the stick
  GET  /api/ir/diag               @ir diag (the pins' idle levels)
"""
import json
import logging

from aiohttp import web

from . import ir

log = logging.getLogger("ir")


def routes(agent):
    def err(text, status=400):
        text = str(text)
        return web.json_response({"error": text[:1].upper() + text[1:]}, status=status)

    async def body(request):
        try:
            b = await request.json()
            return b if isinstance(b, dict) else {}
        except ValueError:
            return {}

    async def run(coro):
        try:
            return web.json_response({"ok": True, **(await coro or {})})
        except ir.IRError as e:
            return err(e, 409)

    async def get_all(_):
        out = {"connected": agent.link.connected, "supported": False, "why": None, "fw": None, "devices": [],
               "brands": {k: {"title": v["title"], "buttons": list(v["buttons"])} for k, v in ir.BRANDS.items()},
               "mapped": agent.cfg.get("ir_brands") or {}, "learn": agent.ir.learn}
        if not agent.link.connected:
            out["why"] = "the stick isn't connected by Bluetooth"
            return web.json_response(out)
        state = await agent.ir.ask("@state")
        out["fw"] = (state or {}).get("fw")
        out["supported"], out["why"] = ir.supported(state)
        if out["supported"]:
            try:
                out.update(await agent.ir.devices(with_buttons=True))
            except ir.IRError as e:
                out["why"] = str(e)
        return web.json_response(out)

    async def send(request):
        b = await body(request)
        if b.get("code"):
            return await run(_wrap(agent.ir.send_code(b["code"], b.get("repeat", 0))))
        if b.get("brand"):
            code = ir.BRANDS.get(b["brand"], {}).get("buttons", {}).get(ir.button_name(b.get("btn")))
            if not code:
                return err("no such button in that code set", 404)
            return await run(_wrap(agent.ir.send_code(code, b.get("repeat", 0))))
        return await run(_wrap(agent.ir.send(b.get("dev", ""), b.get("btn", ""), b.get("repeat", 0))))

    async def learn(request):
        b = await body(request)
        return await run(agent.ir.start_learn(b.get("dev", ""), b.get("btn", ""), bool(b.get("once"))))

    async def learn_cancel(_):
        return await run(agent.ir.cancel())

    async def rename(request):
        b = await body(request)
        if not b.get("dev") or not b.get("to"):
            return err("say what to rename and to what")
        return await run(agent.ir.rename(b["dev"], b["to"], b.get("btn")))

    async def remove(request):
        b = await body(request)
        if not b.get("dev"):
            return err("which remote?")
        return await run(agent.ir.remove(b["dev"], b.get("btn")))

    async def fav(request):
        b = await body(request)
        return await run(agent.ir.fav(b.get("dev", ""), b.get("btn", ""), bool(b.get("on"))))

    async def brand(request):
        b = await body(request)
        dev, name = ir.device_id(b.get("dev")), str(b.get("brand") or "")
        if not dev:
            return err("which remote?")
        if name and name not in ir.BRANDS:
            return err("unknown code set", 404)
        mapped = dict(agent.cfg.get("ir_brands") or {})
        if name:
            mapped[dev] = name
        else:
            mapped.pop(dev, None)
        agent.update_config({"ir_brands": mapped})
        if name and b.get("install"):
            return await run(agent.ir.install_brand(dev, name))
        return web.json_response({"ok": True, "mapped": mapped})

    async def export(_):
        try:
            doc = await agent.ir.export()
        except ir.IRError as e:
            return err(e, 409)
        return web.Response(text=json.dumps(doc, indent=1), content_type="application/json",
                            headers={"Content-Disposition": 'attachment; filename="stickybot-remotes.json"'})

    async def import_(request):
        b = await body(request)
        return await run(agent.ir.import_doc(b.get("doc", b)))

    async def diag(_):
        return await run(agent.ir.diag())

    return [
        web.get("/api/ir", get_all), web.post("/api/ir/send", send), web.post("/api/ir/learn", learn),
        web.post("/api/ir/learn/cancel", learn_cancel), web.post("/api/ir/rename", rename),
        web.post("/api/ir/remove", remove), web.post("/api/ir/fav", fav), web.post("/api/ir/brand", brand),
        web.get("/api/ir/export", export), web.post("/api/ir/import", import_), web.get("/api/ir/diag", diag),
    ]


async def _wrap(coro):
    """A send's text -> {"result": ...}."""
    return {"result": await coro}
