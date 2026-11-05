"""Dashboard + local API: guards, a few endpoints, the permission long-poll."""
import asyncio
import json

import pytest

from stick_agent import config, server

HOST = {"Host": f"localhost:{config.PORT}"}


@pytest.fixture
async def client(agent, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.agent = agent
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def test_wrong_host_is_refused(client):
    r = await client.get("/api/status", headers={"Host": "evil.example:80"})
    assert r.status == 403 and (await r.json())["error"] == "bad host"


async def test_status(client):
    r = await client.get("/api/status", headers=HOST)
    assert r.status == 200
    s = await r.json()
    assert s["link"] == "connected" and s["model"] == "sonnet" and s["prompts"] == []


async def test_changes_need_the_token(client):
    for method, path in [("POST", "/api/ask"), ("PUT", "/api/config"), ("PUT", "/api/commands"),
                         ("POST", "/api/permission/decide"), ("POST", "/api/restart")]:
        r = await client.request(method, path, headers=HOST, json={})
        assert r.status == 403, path
        r = await client.request(method, path, headers={**HOST, "X-Stick-Token": "wrong"}, json={})
        assert r.status == 403, path


async def test_page_has_the_token_built_in(client):
    r = await client.get("/", headers=HOST)
    assert r.status == 200 and config.token() in await r.text()
    assert r.headers["Cache-Control"] == "no-cache"


async def test_dashboard_files_are_revalidated(client):
    r = await client.get("/static/firmware.js", headers=HOST)
    assert r.status == 200 and r.headers["Cache-Control"] == "no-cache"
    r = await client.get("/api/status", headers=HOST)
    assert "Cache-Control" not in r.headers


async def test_commands_put_validates(client):
    r = await client.put("/api/commands", headers=client.auth, json={"nope": 1})
    assert r.status == 400
    good = {"commands": [{"name": "Docs", "phrases": ["docs {q}"], "open_url": "https://x.test/{q}"}]}
    r = await client.put("/api/commands", headers=client.auth, json=good)
    assert (await r.json())["ok"]
    assert json.loads(config.COMMANDS.read_text()) == good
    r = await client.get("/api/commands", headers=HOST)
    assert await r.json() == good


async def test_commands_test(client):
    r = await client.post("/api/commands/test", headers=client.auth, json={"text": "PR twelve thirty four"})
    body = await r.json()
    assert body["normalized"] == "pr 1234" and body["match"]["slots"] == {"n": "1234"}


async def test_config_put_ignores_unknown_keys(client):
    r = await client.put("/api/config", headers=client.auth, json={"model": "opus", "evil": True})
    cfg = await r.json()
    assert cfg["model"] == "opus" and "evil" not in cfg
    assert config.load()["model"] == "opus"


async def test_permission_long_poll(client):
    async def ask():
        r = await client.post("/api/permission", headers=client.auth,
                              json={"tool": "Bash", "hint": "ls", "from": "web", "wait": 5})
        return await r.json()
    task = asyncio.create_task(ask())
    for _ in range(100):
        s = await (await client.get("/api/status", headers=HOST)).json()
        if s["prompts"]:
            break
        await asyncio.sleep(0.02)
    pid = s["prompts"][0]["id"]
    r = await client.post("/api/permission/decide", headers=client.auth, json={"id": pid, "allow": True})
    assert (await r.json())["ok"]
    assert await task == {"decision": "allow"}


async def test_ask_runs_typed_requests(client, monkeypatch):
    seen = []

    async def handle_text(text, source="typed"):
        seen.append((text, source))
    monkeypatch.setattr(client.agent, "handle_text", handle_text)
    r = await client.post("/api/ask", headers=client.auth, json={"text": "  what time is it  "})
    assert (await r.json())["ok"]
    await asyncio.sleep(0)
    assert seen == [("what time is it", "typed")]
    r = await client.post("/api/ask", headers=client.auth, json={"text": "   "})
    assert r.status == 400


async def test_claude_event(client):
    client.agent.cfg["claude_done_min_s"] = 60
    r = await client.post("/api/claude-event", headers=client.auth,
                          json={"elapsed_s": 300, "cwd": "/Users/me/web", "prompt": "fix"})
    assert (await r.json())["shown"]
    assert client.agent.link.sent[-1]["event"]["title"] == "Claude finished in web"


async def test_events_test_endpoint(client):
    r = await client.post("/api/events/test", headers=client.auth, json={"kind": "pr_merged"})
    assert r.status == 200
    assert client.agent.link.sent[-1]["event"]["level"] == "celebrate"
