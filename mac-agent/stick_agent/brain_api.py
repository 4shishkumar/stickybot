"""Dashboard API for the brains (brains.py): Settings → Brain.

  GET    /api/brain                        {"brain", "status", "openrouter": {"key", "model", ...}}
  PUT    /api/brain/openrouter/key         {"key"} -> saved in the Keychain; answers the key's status only
  DELETE /api/brain/openrouter/key         removes it
  GET    /api/brain/openrouter/models      {"models": [{id, name, in, out, tools, context}], "suggested", "error"}
                                           (OpenRouter's public list, cached 10 min; ?refresh=1 reloads)
  POST   /api/brain/test                   {"model"?} -> one short request: {"ok", "reply", "total_ms", "cost"}

The key is write-only: no answer here carries it (status: "saved", "hint": "…abcd"). Picking the brain
and the model is ordinary config (PUT /api/config "brain", "openrouter_model").
"""
import asyncio
import logging
import re

from aiohttp import web

from . import brains, keychain

log = logging.getLogger("server")

KEY_RE = re.compile(r"[A-Za-z0-9._\-]{20,300}")
MODEL_RE = re.compile(r"[A-Za-z0-9._\-/:~]{1,200}")


def valid_config(body):
    """'' if the brain settings in a PUT /api/config body are fine, else what's wrong."""
    if "brain" in body and body["brain"] not in brains.NAMES:
        return f"brain must be one of: {', '.join(brains.NAMES)}"
    m = body.get("openrouter_model")
    if m is not None and (not isinstance(m, str) or m and not MODEL_RE.fullmatch(m)):
        return "openrouter_model: not a model id"
    n = body.get("openrouter_max_tokens")
    if n is not None and (isinstance(n, bool) or not isinstance(n, int) or not 50 <= n <= 4000):
        return "openrouter_max_tokens must be 50-4000"
    return ""


def routes(agent):
    def openrouter():
        return getattr(agent.brain, "openrouter", None)

    async def key_status():
        return await asyncio.get_running_loop().run_in_executor(None, keychain.status, "openrouter")

    async def status(_):
        b = agent.brain
        return web.json_response({
            "brain": brains.selected(agent.cfg), "choices": list(brains.NAMES),
            "status": b.status() if getattr(b, "status", None) else None,
            "openrouter": {"key": await key_status(), "model": agent.cfg.get("openrouter_model") or "",
                           "max_tokens": agent.cfg.get("openrouter_max_tokens")}})

    async def put_key(request):
        try:
            body = await request.json()
        except ValueError:
            body = None
        key = body.get("key") if isinstance(body, dict) else None
        key = key.strip() if isinstance(key, str) else ""
        if not KEY_RE.fullmatch(key):
            return web.json_response({"error": "That doesn't look like an API key (letters, digits, - and _ only)."},
                                     status=400)
        try:
            await asyncio.get_running_loop().run_in_executor(None, keychain.save, "openrouter", key)
        except (keychain.KeychainError, OSError) as e:
            log.warning("openrouter key: %s", e)
            return web.json_response({"error": f"Couldn't save it in the Keychain: {e}"}, status=500)
        return web.json_response({"ok": True, "key": await key_status()})

    async def delete_key(_):
        try:
            had = await asyncio.get_running_loop().run_in_executor(None, keychain.remove, "openrouter")
        except (keychain.KeychainError, OSError) as e:
            return web.json_response({"error": f"Couldn't remove it from the Keychain: {e}"}, status=500)
        return web.json_response({"ok": True, "removed": had, "key": await key_status()})

    async def models(request):
        orb = openrouter()
        if orb is None:
            return web.json_response({"error": "Not available in this agent."}, status=503)
        listed, suggested, err = await orb.list_models(force=request.query.get("refresh") == "1")
        return web.json_response({"models": listed, "suggested": suggested, "error": err},
                                 status=200 if listed or not err else 502)

    async def test(request):
        orb = openrouter()
        if orb is None:
            return web.json_response({"error": "Not available in this agent."}, status=503)
        try:
            body = await request.json()
        except ValueError:
            body = {}
        model = body.get("model") if isinstance(body, dict) else None
        if model is not None and (not isinstance(model, str) or not MODEL_RE.fullmatch(model)):
            return web.json_response({"error": "not a model id"}, status=400)
        return web.json_response(await orb.test(model or None))

    return [web.get("/api/brain", status), web.put("/api/brain/openrouter/key", put_key),
            web.delete("/api/brain/openrouter/key", delete_key), web.get("/api/brain/openrouter/models", models),
            web.post("/api/brain/test", test)]
