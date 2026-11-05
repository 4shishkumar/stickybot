"""Dashboard + local API on http://localhost:8660 (this Mac only).

Every request must come with Host localhost/127.0.0.1, and anything that
changes state needs the token in ~/.stick-agent/token (the dashboard page has
it built in) — so a random website can't drive the agent through your browser.
"""
import asyncio
import hmac
import json
import logging
from pathlib import Path

from aiohttp import web

from . import clips, commands, components_api, config, firmware_api, install_flow, link, mcp_server, mcp_setup, updates
from . import events as evmod

log = logging.getLogger("server")
PAGE = Path(__file__).parent / "dashboard.html"
STATIC = Path(__file__).parent / "static"  # the dashboard's CSS / JS / images
OK_HOSTS = {f"localhost:{config.PORT}", f"127.0.0.1:{config.PORT}"}


def make_app(agent):
    tok = config.token()

    @web.middleware
    async def guard(request, handler):
        if request.host not in OK_HOSTS:
            return web.json_response({"error": "bad host"}, status=403)
        if request.path == "/mcp":  # every method; MCP clients send the token as a bearer token
            same = lambda a, b: hmac.compare_digest(a.encode("utf-8", "replace"), b.encode())
            if not (same(request.headers.get("Authorization", ""), f"Bearer {tok}")
                    or same(request.headers.get("X-Stick-Token", ""), tok)):
                return web.json_response({"error": "bad token"}, status=403)
        elif request.method != "GET" and request.headers.get("X-Stick-Token") != tok:
            return web.json_response({"error": "bad token"}, status=403)
        resp = await handler(request)
        if request.path == "/" or request.path.startswith("/static/"):
            # revalidate every load (cheap 304s): a cached old page module after an update hides new UI
            resp.headers.setdefault("Cache-Control", "no-cache")
        return resp

    async def page(_):
        return web.Response(text=PAGE.read_text().replace("{{TOKEN}}", tok), content_type="text/html")

    async def status(_):
        return web.json_response(agent.status())

    async def events(request):
        resp = web.StreamResponse(headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
        await resp.prepare(request)
        q = asyncio.Queue()
        agent.subscribers.add(q)
        try:
            for ev in list(agent.events)[-150:]:  # catch up
                await resp.write(f"data: {json.dumps(ev)}\n\n".encode())
            while True:
                try:
                    ev = await asyncio.wait_for(q.get(), 15)
                    await resp.write(f"data: {json.dumps(ev)}\n\n".encode())
                except TimeoutError:
                    await resp.write(b": keep-alive\n\n")
        except (ConnectionResetError, asyncio.CancelledError):
            pass
        finally:
            agent.subscribers.discard(q)
        return resp

    async def history(request):
        n = int(request.query.get("n", 50))
        try:
            lines = config.HISTORY.read_text().splitlines()[-n:]
        except OSError:
            lines = []
        return web.json_response([json.loads(line) for line in reversed(lines) if line.strip()])

    async def logs(request):
        n = int(request.query.get("n", 300))
        try:
            text = config.LOG.read_text(errors="replace").splitlines()[-n:]
        except OSError:
            text = []
        return web.json_response(text)

    async def ask(request):
        body = await request.json()
        text = (body.get("text") or "").strip()
        if not text:
            return web.json_response({"error": "empty"}, status=400)
        asyncio.ensure_future(agent.handle_text(text, source="typed"))
        return web.json_response({"ok": True})

    async def get_commands(_):
        return web.json_response(json.loads(config.COMMANDS.read_text()))

    async def put_commands(request):
        body = await request.json()
        if not isinstance(body.get("commands"), list):
            return web.json_response({"error": "expected {\"commands\": [...]}"}, status=400)
        for c in body["commands"]:  # check every phrase compiles
            for p in c.get("phrases", []):
                commands.phrase_regex(p)
        config.COMMANDS.write_text(json.dumps(body, indent=2))
        return web.json_response({"ok": True})

    async def test_command(request):
        body = await request.json()
        hit = commands.match(body.get("text", ""))
        return web.json_response({"normalized": commands.normalize(body.get("text", "")),
                                  "match": hit and {"name": hit[0].get("name"), "slots": hit[1]}})

    async def get_config(_):
        return web.json_response(agent.cfg)

    async def put_config(request):
        body = await request.json()
        allowed = {k: v for k, v in body.items() if k in config.DEFAULT_CONFIG}
        agent.update_config(allowed)
        return web.json_response(agent.cfg)

    async def show_mic(request):
        """The stick-mic-bluetooth device for a minute, so it can be ranked in Wispr's settings."""
        try:
            body = await request.json()
        except ValueError:
            body = {}
        secs = min(300, max(5, int(body.get("secs") or 60)))
        why = await agent.holds.show(secs)
        return web.json_response({"ok": not why, "error": why or None, "secs": secs, "name": agent.mic_dev.name},
                                 status=409 if why else 200)

    async def scan(_):
        return web.json_response(await link.scan())

    async def permission(request):
        """Long-poll from the Claude Code hook: waits for A/B on the stick or the dashboard."""
        body = await request.json()
        decision = await agent.ask_permission(body.get("tool", "?"), body.get("hint", ""), body.get("from", "terminal"),
                                              float(body.get("wait", 30)))
        return web.json_response({"decision": decision})

    async def decide(request):
        body = await request.json()
        agent.decide(body.get("id"), bool(body.get("allow")))
        return web.json_response({"ok": True})

    async def pause(request):
        body = await request.json()
        agent.link.pause(bool(body.get("paused")))
        return web.json_response({"ok": True})

    async def voices(_):
        """Installed macOS voices: [{name, lang}] (from `say -v ?`)."""
        proc = await asyncio.create_subprocess_exec("say", "-v", "?", stdout=asyncio.subprocess.PIPE)
        out, _ = await proc.communicate()
        vs = []
        for line in out.decode(errors="replace").splitlines():
            head = line.split("#")[0].rstrip()
            parts = head.rsplit(None, 1)
            if len(parts) == 2:
                vs.append({"name": parts[0].strip(), "lang": parts[1]})
        return web.json_response(vs)

    async def test_voice(request):
        body = await request.json()
        await agent.speak(body.get("text") or "It's 28 degrees in Bangalore with light rain.", force=True)
        return web.json_response({"ok": True})

    async def get_sounds(_):
        s = await agent.sound({"get": 1})
        return web.json_response(s or {"error": "the stick didn't answer (connected?)"}, status=200 if s else 503)

    async def put_sounds(request):
        body = await request.json()
        payload = {k: body[k] for k in ("set", "on", "volume") if k in body}
        s = await agent.sound(payload)
        return web.json_response(s or {"error": "the stick didn't answer"}, status=200 if s else 503)

    async def play_sound(request):
        body = await request.json()
        s = await agent.sound({"play": str(body.get("name", "test"))})
        return web.json_response({"ok": bool(s)})

    async def clip_list(_):
        return web.json_response(clips.library())

    async def clip_play(request):
        body = await request.json()
        err = await agent.play_clip(str(body.get("pack", "")), str(body.get("name", "")))
        return web.json_response({"ok": err is None, "error": err})

    async def event_test(request):
        body = await request.json()
        ev = await agent.notifier.test(str(body.get("kind", "test")))
        return web.json_response({"ok": True, "event": ev})

    async def event_history(request):
        return web.json_response(evmod.history(int(request.query.get("n", 50))))

    async def event_kinds(_):
        return web.json_response({"kinds": {k: {"default": v[0], "icon": v[1], "title": v[2]}
                                            for k, v in evmod.KINDS.items()},
                                  "rules": evmod.rules(agent.cfg), "levels": evmod.LEVELS})

    async def claude_event(request):
        body = await request.json()
        ev = await agent.claude_finished(int(body.get("elapsed_s", 0)), str(body.get("cwd", "")),
                                         str(body.get("prompt", "")))
        return web.json_response({"ok": True, "shown": bool(ev)})

    # --- the stick tools (MCP) for Claude sessions --------------------------------------

    async def mcp_status(_):
        """The server switch, recent use, and whether terminal Claude Code sessions have the tools."""
        st = await asyncio.get_running_loop().run_in_executor(None, mcp_setup.status)
        tools = getattr(agent, "mcp_tools", None)
        return web.json_response({"enabled": agent.cfg.get("mcp_server", True), "url": mcp_setup.url(),
                                  "terminal": st, "calls": tools.calls if tools else 0,
                                  "last_call": tools.last_call if tools else None,
                                  "tools": [{"name": t["name"], "title": t["title"],
                                             "approval": t["name"] in mcp_server.MAC_TOOLS}
                                            for t in mcp_server.TOOLS]})

    async def mcp_terminal(request):
        """Install / remove the tools for terminal sessions (`claude mcp add|remove`). The page shows
        the command first; the request must say {"confirm": true}."""
        body = await request.json()
        if body.get("confirm") is not True:
            return web.json_response({"error": "needs {\"confirm\": true}"}, status=400)
        action = request.match_info["action"]
        lines = []
        fn = mcp_setup.install if action == "install" else mcp_setup.uninstall
        ok, msg = await asyncio.get_running_loop().run_in_executor(None, lambda: fn(confirm=None, say=lines.append))
        log.info("mcp %s for terminal sessions: %s", action, msg)
        return web.json_response({"ok": ok, "message": msg, "ran": [x.strip() for x in lines]},
                                 status=200 if ok else 500)

    async def mcp_answer(request):
        body = await request.json()
        choice = body.get("choice")
        ok = agent.answer(body.get("id"), choice if isinstance(choice, int) else None)
        return web.json_response({"ok": ok}, status=200 if ok else 404)

    async def new_chat(_):
        agent.brain.forget()
        agent.emit("info", text="Next request starts a new conversation")
        return web.json_response({"ok": True})

    async def restart(_):
        log.info("restart requested from the dashboard")
        agent.restart_soon()
        return web.json_response({"ok": True})

    app = web.Application(middlewares=[guard])
    app.router.add_static("/static/", STATIC, show_index=False)
    app.add_routes(firmware_api.routes(agent))
    app.add_routes(components_api.routes(agent))
    app.add_routes(install_flow.routes(agent))  # the setup page (#install); after firmware_api (stick_api)
    app.add_routes(mcp_server.routes(agent))
    app.add_routes(updates.routes(agent))
    app.add_routes([
        web.get("/api/mcp", mcp_status), web.post("/api/mcp/terminal/{action:install|uninstall}", mcp_terminal),
        web.post("/api/mcp/answer", mcp_answer),
    ])
    app.add_routes([
        web.get("/", page), web.get("/api/status", status), web.get("/api/events", events),
        web.get("/api/history", history), web.get("/api/logs", logs), web.post("/api/ask", ask),
        web.get("/api/commands", get_commands), web.put("/api/commands", put_commands),
        web.post("/api/commands/test", test_command),
        web.get("/api/config", get_config), web.put("/api/config", put_config), web.get("/api/scan", scan),
        web.post("/api/permission", permission), web.post("/api/permission/decide", decide),
        web.get("/api/sounds", get_sounds), web.put("/api/sounds", put_sounds),
        web.post("/api/sounds/play", play_sound),
        web.post("/api/events/test", event_test), web.get("/api/events/history", event_history),
        web.get("/api/events/kinds", event_kinds), web.post("/api/claude-event", claude_event),
        web.get("/api/clips", clip_list), web.post("/api/clips/play", clip_play),
        web.get("/api/voices", voices), web.post("/api/voice/test", test_voice),
        web.post("/api/wireless-mic/show", show_mic),
        web.post("/api/pause", pause), web.post("/api/new-chat", new_chat), web.post("/api/restart", restart),
    ])
    return app


async def serve(agent):
    runner = web.AppRunner(make_app(agent), access_log=None)
    await runner.setup()
    await web.TCPSite(runner, "127.0.0.1", config.PORT).start()
    log.info("dashboard on http://localhost:%d", config.PORT)
