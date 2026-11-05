"""The stick tools (mcp_server.py): the MCP endpoint (auth, both protocol eras, errors), the tools
on a fake stick (ask flow, rate limits, fail-closed), and `stick-agent mcp install` (mcp_setup.py)
with a mocked subprocess — Claude Code is never run for real."""
import asyncio
import base64
import json
import subprocess
import time

import pytest

from stick_agent import config, home, mcp_server, mcp_setup, server

HOST = {"Host": f"localhost:{config.PORT}"}
MODERN = "2026-07-28"


@pytest.fixture
async def client(agent, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.agent = agent
    c.auth = {**HOST, "Authorization": f"Bearer {config.token()}"}
    c.n = 0

    async def rpc(method, params=None, headers=None, modern=False, notification=False):
        """-> (status, body or None). modern: adds the 2026-07-28 _meta and mirrored headers."""
        params = dict(params or {})
        hdrs = {**c.auth, "Accept": "application/json, text/event-stream", **(headers or {})}
        if modern:
            params["_meta"] = {"io.modelcontextprotocol/protocolVersion": MODERN,
                               "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1"},
                               "io.modelcontextprotocol/clientCapabilities": {}}
            hdrs.setdefault("MCP-Protocol-Version", MODERN)
            hdrs.setdefault("Mcp-Method", method)
            if method == "tools/call":
                hdrs.setdefault("Mcp-Name", params.get("name", ""))
        msg = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            c.n += 1
            msg["id"] = c.n
        r = await c.post("/mcp", json=msg, headers=hdrs)
        if r.content_type == "text/event-stream":
            text = await r.text()
            data = [line[6:] for line in text.splitlines() if line.startswith("data: ")]
            return r.status, json.loads(data[-1])
        body = await r.text()
        return r.status, (json.loads(body) if body else None)
    c.rpc = rpc

    async def call(name, args=None, **kw):
        status, body = await rpc("tools/call", {"name": name, "arguments": args or {}}, **kw)
        assert status == 200, body
        res = body["result"]
        return res["content"][0]["text"], res["isError"], res
    c.call = call
    return c


@pytest.fixture(autouse=True)
def no_alert_gap(monkeypatch):
    """Tests call alerts back to back; the rate-limit tests set their own limiter."""
    monkeypatch.setattr(mcp_server, "ALERT_GAP_S", 0.0)


def sent_events(agent):
    return [m["event"] for m in agent.link.sent if "event" in m]


async def until(cond, timeout=3.0):
    end = time.monotonic() + timeout
    while not cond():
        assert time.monotonic() < end, "timed out"
        await asyncio.sleep(0.01)


# --- the endpoint: guards ------------------------------------------------------------------

async def test_needs_the_token(client):
    msg = {"jsonrpc": "2.0", "id": 1, "method": "ping"}
    for headers in (HOST, {**HOST, "Authorization": "Bearer wrong"}, {**HOST, "Authorization": config.token()},
                    {**HOST, "Authorization": "Bearer ünïcode"}):
        r = await client.post("/mcp", json=msg, headers=headers)
        assert r.status == 403
    r = await client.get("/mcp", headers=HOST)  # even a GET
    assert r.status == 403
    r = await client.post("/mcp", json=msg, headers={**HOST, "X-Stick-Token": config.token()})
    assert r.status == 200 and (await r.json())["result"] == {}


async def test_bad_host_and_origin(client):
    msg = {"jsonrpc": "2.0", "id": 1, "method": "ping"}
    r = await client.post("/mcp", json=msg, headers={**client.auth, "Host": "evil.example"})
    assert r.status == 403
    r = await client.post("/mcp", json=msg, headers={**client.auth, "Origin": "https://evil.example"})
    assert r.status == 403  # DNS rebinding: a page elsewhere can't use it even with a stolen token
    r = await client.post("/mcp", json=msg, headers={**client.auth, "Origin": f"http://localhost:{config.PORT}"})
    assert r.status == 200


async def test_get_and_delete_are_405(client):
    for method in ("GET", "DELETE"):
        r = await client.request(method, "/mcp", headers=client.auth)
        assert r.status == 405


async def test_turned_off(client):
    client.agent.cfg["mcp_server"] = False
    status, body = await client.rpc("tools/list")
    assert status == 503 and "turned off" in body["error"]["message"]


async def test_malformed_messages(client):
    r = await client.post("/mcp", data="{nope", headers={**client.auth, "Content-Type": "application/json"})
    assert r.status == 400 and (await r.json())["error"]["code"] == -32700
    r = await client.post("/mcp", json=[{"jsonrpc": "2.0", "id": 1, "method": "ping"}], headers=client.auth)
    assert r.status == 400  # no batches
    r = await client.post("/mcp", json={"id": 1, "method": "ping"}, headers=client.auth)
    assert r.status == 400 and (await r.json())["error"]["code"] == -32600


# --- legacy clients (initialize) --------------------------------------------------------------

async def test_initialize_and_initialized(client):
    status, body = await client.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                                   "clientInfo": {"name": "x", "version": "1"}})
    assert status == 200
    res = body["result"]
    assert res["protocolVersion"] == "2025-06-18" and res["capabilities"]["tools"] == {"listChanged": False}
    assert res["serverInfo"]["name"] == "stick" and "pager" in res["instructions"]
    assert "resultType" not in res
    status, body = await client.rpc("notifications/initialized", notification=True)
    assert status == 202 and body is None


async def test_initialize_unknown_version_gets_our_newest_legacy(client):
    _, body = await client.rpc("initialize", {"protocolVersion": "1999-01-01"})
    assert body["result"]["protocolVersion"] == "2025-11-25"


async def test_legacy_requests(client):
    _, body = await client.rpc("tools/list", headers={"MCP-Protocol-Version": "2025-11-25"})
    names = [t["name"] for t in body["result"]["tools"]]
    assert names == ["notify", "ask", "celebrate", "progress", "pin_home", "open_url", "open_app", "quick_command",
                     "stick_status", "speak"]
    for t in body["result"]["tools"]:
        assert t["description"] and t["inputSchema"]["type"] == "object"
    status, body = await client.rpc("tools/list", headers={"MCP-Protocol-Version": "1999-01-01"})
    assert status == 400 and body["error"]["code"] == -32022
    status, body = await client.rpc("nope/nope")
    assert status == 200 and body["error"]["code"] == -32601


# --- modern clients (2026-07-28: _meta + mirrored headers, server/discover) -------------------

async def test_discover(client):
    status, body = await client.rpc("server/discover", modern=True)
    assert status == 200
    res = body["result"]
    assert res["resultType"] == "complete" and MODERN in res["supportedVersions"]
    assert "2025-11-25" in res["supportedVersions"]
    assert res["_meta"]["io.modelcontextprotocol/serverInfo"]["name"] == "stick"


async def test_modern_tools_call(client):
    text, err, res = await client.call("stick_status", modern=True)
    assert res["resultType"] == "complete" and not err


async def test_modern_header_mismatches(client):
    for hdrs in ({"MCP-Protocol-Version": "2025-11-25"}, {"Mcp-Method": "tools/call"}):
        status, body = await client.rpc("tools/list", headers=hdrs, modern=True)
        assert status == 400 and body["error"]["code"] == -32020
    status, body = await client.rpc("tools/call", {"name": "stick_status", "arguments": {}},
                                    headers={"Mcp-Name": "notify"}, modern=True)
    assert status == 400 and body["error"]["code"] == -32020
    enc = "=?base64?" + base64.b64encode(b"stick_status").decode() + "?="  # the sentinel form is decoded
    status, _ = await client.rpc("tools/call", {"name": "stick_status", "arguments": {}},
                                 headers={"Mcp-Name": enc}, modern=True)
    assert status == 200
    # a modern header without the body's _meta
    status, body = await client.rpc("tools/list", headers={"MCP-Protocol-Version": MODERN})
    assert status == 400 and body["error"]["code"] == -32020


async def test_modern_unsupported_version_and_unknown_method(client):
    hdrs = {**client.auth, "MCP-Protocol-Version": "2099-01-01", "Mcp-Method": "tools/list"}
    msg = {"jsonrpc": "2.0", "id": 9, "method": "tools/list",
           "params": {"_meta": {"io.modelcontextprotocol/protocolVersion": "2099-01-01"}}}
    r = await client.post("/mcp", json=msg, headers=hdrs)
    body = await r.json()
    assert r.status == 400 and body["error"]["code"] == -32022
    assert MODERN in body["error"]["data"]["supported"] and body["error"]["data"]["requested"] == "2099-01-01"
    status, body = await client.rpc("nope/nope", modern=True)
    assert status == 404 and body["error"]["code"] == -32601


# --- tools/call errors --------------------------------------------------------------------------

async def test_unknown_tool_is_a_protocol_error(client):
    status, body = await client.rpc("tools/call", {"name": "rm_rf", "arguments": {}})
    assert status == 200 and body["error"]["code"] == -32602


@pytest.mark.parametrize("args, why", [
    ({}, "missing argument: text"),
    ({"text": 5}, "text must be a string"),
    ({"text": "hi", "level": "loud"}, "level must be one of"),
    ({"text": "hi", "sound": "yes"}, "sound must be true or false"),
    ({"text": "hi", "colour": "red"}, "unknown argument(s): colour"),
])
async def test_bad_arguments_are_tool_errors(client, args, why):
    text, err, _ = await client.call("notify", args)
    assert err and why in text
    assert not sent_events(client.agent)


async def test_check_args_cuts_long_text_and_fills_defaults():
    schema = mcp_server.BY_NAME["ask"]["inputSchema"]
    out = mcp_server.check_args(schema, {"question": "q" * 500, "options": ["x" * 40]})
    assert len(out["question"]) == 160 and out["options"] == ["x" * 16] and out["timeout_s"] == 60
    with pytest.raises(mcp_server.ToolError, match="between 5 and 120"):
        mcp_server.check_args(schema, {"question": "q", "timeout_s": 1})
    with pytest.raises(mcp_server.ToolError, match="1-2 items"):
        mcp_server.check_args(schema, {"question": "q", "options": ["a", "b", "c"]})
    with pytest.raises(mcp_server.ToolError, match="integer"):
        mcp_server.check_args(schema, {"question": "q", "timeout_s": True})


async def test_fail_closed_without_the_stick(client):
    client.agent.link.connected = False
    for name, args in [("notify", {"text": "hi"}), ("ask", {"question": "ok?"}), ("celebrate", {}),
                       ("progress", {"text": "x"}), ("pin_home", {"text": "x"}), ("speak", {"text": "hi"}),
                       ("open_url", {"url": "https://example.com"}), ("open_app", {"name": "Notes"}),
                       ("quick_command", {"phrase": "pr 1"})]:
        text, err, _ = await client.call(name, args)
        assert err and text == "stick not connected", name
    text, err, res = await client.call("stick_status")
    assert not err and text.startswith("stick not connected") and res["structuredContent"]["connected"] is False


# --- feedback tools -------------------------------------------------------------------------------

async def test_notify_levels_and_sound(client):
    await client.call("notify", {"text": "Tests passed"})
    await client.call("notify", {"text": "Build failed: 3 errors in the parser module, see the log",
                                 "level": "heads_up"})
    await client.call("notify", {"text": "Merged", "level": "nice", "sound": False})
    await client.call("celebrate", {"text": "Shipped v2!"})
    await client.call("celebrate")
    evs = sent_events(client.agent)
    assert [(e["level"], e["sound"]) for e in evs] == [("nice", "off"), ("headsup", ""), ("nice", "off"),
                                                       ("celebrate", ""), ("celebrate", "")]
    assert evs[0]["title"] == "Tests passed" and evs[0]["sub"] == "from Claude Code"
    assert evs[1]["title"] == "Build failed: 3 errors in the parser" and evs[1]["sub"] == "module, see the log"
    assert evs[4]["title"] == "Done!" and all(e["icon"] == "claude" for e in evs)


async def test_caller_names(client):
    await client.call("notify", {"text": "a"}, headers={"X-Stick-Cwd": "/Users/me/src/web"})
    await client.call("notify", {"text": "b"}, headers={"X-Stick-Cwd": "${PWD}"})  # not expanded
    await client.call("notify", {"text": "c"}, headers={"X-Stick-Client": "voice"})
    assert [e["sub"] for e in sent_events(client.agent)] == ["from web", "from Claude Code", "from Agent"]
    log = [e for e in client.agent.events if e["kind"] == "mcp"]
    assert [e["origin"] for e in log] == ["web", "Claude Code", "Agent"]
    assert log[0]["tool"] == "notify" and log[0]["args"] == "text='a'" and log[0]["ok"]


async def test_text_is_made_drawable(client):
    await client.call("notify", {"text": "Café ✅ done"})
    assert sent_events(client.agent)[0]["title"] == "Cafe done"


async def test_notifications_can_be_turned_off(client):
    client.agent.cfg["event_rules"] = {"claude_tool": "off"}
    text, err, _ = await client.call("notify", {"text": "hi"})
    assert err and "turned notifications from Claude sessions off" in text and not sent_events(client.agent)


async def test_alerts_are_rate_limited(client, monkeypatch):
    client.agent.mcp_tools.alerts = mcp_server.RateLimit(1.0, 20)
    text, err, _ = await client.call("notify", {"text": "one"})
    assert not err
    text, err, _ = await client.call("notify", {"text": "two"})
    assert err and text.startswith("rate limited")
    text, err, _ = await client.call("celebrate")  # the same budget for every alert
    assert err
    assert len(sent_events(client.agent)) == 1


def test_rate_limit_window():
    r = mcp_server.RateLimit(1.0, 3)
    assert r.take(0) == "" and r.take(0.5).startswith("rate limited") and r.take(1.0) == "" and r.take(2.0) == ""
    assert "3 alerts in the last minute" in r.take(3.0)
    assert r.take(61.0) == ""


async def test_progress_and_pin_on_the_home_card(client):
    a = client.agent
    text, err, _ = await client.call("progress", {"text": "Running tests", "done": 40, "total": 120},
                                     headers={"X-Stick-Cwd": "/x/web"})
    assert not err
    homes = [m["home"] for m in a.link.sent if "home" in m]
    assert homes[-1] == {"k": "claude", "a": "web: 40/120 (33%)", "b": "Running tests"}
    assert a.home_card() == homes[-1]
    text, err, _ = await client.call("progress", {"text": "x", "done": 5, "total": 2})
    assert err and "more than total" in text
    await asyncio.sleep(mcp_server.CARD_PUSH_GAP_S)
    await client.call("pin_home", {"text": "PR #123 waiting", "minutes": 5})
    assert a.home_card()["a"] == "Pinned · Claude Code" and a.home_card()["b"] == "PR #123 waiting"
    assert a.home.slots["pin"]["until"] - time.time() == pytest.approx(300, abs=2)
    text, _, _ = await client.call("pin_home", {"text": ""})
    assert text == "pin removed" and a.home_card()["b"] == "Running tests"  # the progress line is back


async def test_card_pushes_are_coalesced(client, monkeypatch):
    monkeypatch.setattr(mcp_server, "CARD_PUSH_GAP_S", 0.2)
    for i in range(5):
        await client.call("progress", {"text": f"step {i}", "done": i, "total": 10})
    assert len([m for m in client.agent.link.sent if "home" in m]) == 1
    await until(lambda: len([m for m in client.agent.link.sent if "home" in m]) == 2)
    assert [m for m in client.agent.link.sent if "home" in m][-1]["home"]["b"] == "step 4"  # the newest


def test_finished_progress_expires_sooner(agent):
    agent.home.put("progress", "a", "b", home.PROGRESS_DONE_S)
    agent.home.slots["progress"]["until"] = time.time() - 1
    assert agent.home.pinned() is None and "progress" not in agent.home.slots


async def test_stick_status(client):
    a = client.agent
    orig = a.link.send

    async def send(msg):  # the stick answers {"cmd":"status"}
        ok = await orig(msg)
        if msg.get("cmd") == "status":
            asyncio.get_running_loop().call_soon(a._on_message, {"ack": "status", "ok": True, "data": {
                "bat": {"pct": 64, "mV": 3900, "usb": True}, "sys": {"up": 1200}}})
        return ok
    a.link.send = send
    a._on_message({"src": "agent", "evt": "talk", "state": "start"})  # a press
    a.rec = None
    a.state = "ready"
    a.stick_usb = True
    text, err, res = await client.call("stick_status")
    s = res["structuredContent"]
    assert not err and s["battery_pct"] == 64 and s["usb_power"] and s["plugged_into_computer"]
    assert s["last_button_press_s_ago"] == 0 and s["at_desk"] == "likely" and s["uptime_s"] == 1200
    assert "battery 64%" in text and "screen: the home screen" in text


async def test_stick_status_without_an_answer(client, monkeypatch):
    async def quick(wait=2.0):
        return None
    monkeypatch.setattr(client.agent, "stick_report", quick)
    text, err, res = await client.call("stick_status")
    assert not err and res["structuredContent"]["battery_pct"] is None and "connected" in text


async def test_speak(client, monkeypatch):
    said = []
    monkeypatch.setattr(client.agent.speaker, "say", said.append)
    text, err, _ = await client.call("speak", {"text": "Build  is green."})
    assert not err and said == ["Build is green."] and "speaking on the stick" in text
    monkeypatch.setattr(mcp_server.events, "quiet_now", lambda cfg: True)
    text, err, _ = await client.call("speak", {"text": "again"})
    assert text == "quiet hours: not spoken" and said == ["Build is green."]


# --- ask() ------------------------------------------------------------------------------------

def asks_on_stick(agent):
    return [m["prompt"] for m in agent.link.sent if m.get("src") == "cli" and "prompt" in m]


async def answer_when_shown(agent, decision, n=1):
    await until(lambda: len(asks_on_stick(agent)) >= n)
    agent._on_message({"cmd": "permission", "id": asks_on_stick(agent)[-1]["id"], "decision": decision})


@pytest.mark.parametrize("decision, options, want", [
    ("once", ["Staging", "Prod"], "Staging"),
    ("deny", ["Staging", "Prod"], "Prod"),
    ("deny", None, "No"),
    ("deny", ["OK"], "dismissed"),      # one option: B dismisses
    ("dismiss", ["A", "B"], "dismissed"),  # a shake (firmware 1.7.2)
])
async def test_ask_answered_on_the_stick(client, decision, options, want):
    a = client.agent
    args = {"question": "Deploy where?"} | ({"options": options} if options else {})
    task = asyncio.create_task(client.call("ask", args, headers={"X-Stick-Cwd": "/w/web"}))
    await answer_when_shown(a, decision)
    text, err, res = await task
    assert not err and text == want and res["structuredContent"] == {"answer": want}
    p = asks_on_stick(a)[0]
    labels = options or ["Yes", "No"]
    assert p["kind"] == "ask" and p["q"] == "Deploy where?" and p["from"] == "web"
    assert p["a"] == labels[0] and p["b"] == (labels[1] if len(labels) > 1 else "")
    assert p["hint"].startswith("Deploy where? - A: ")  # older firmware shows this
    assert a.link.sent[-1] == {"src": "cli"} and not a.asks  # cleared on the stick


async def test_ask_json_reply_when_no_sse(client):
    a = client.agent
    task = asyncio.create_task(client.call("ask", {"question": "ok?"}, headers={"Accept": "application/json"}))
    await answer_when_shown(a, "once")
    text, _, _ = await task
    assert text == "Yes"


async def test_ask_from_the_dashboard(client):
    a = client.agent
    task = asyncio.create_task(client.call("ask", {"question": "ok?", "options": ["Go", "Stop"]}))
    await until(lambda: a.asks)
    aid = next(iter(a.asks))
    assert (await client.get("/api/status", headers=HOST)).status == 200
    assert (await (await client.get("/api/status", headers=HOST)).json())["asks"][0]["options"] == ["Go", "Stop"]
    r = await client.post("/api/mcp/answer", json={"id": aid, "choice": 1},
                          headers={**HOST, "X-Stick-Token": config.token()})
    assert (await r.json())["ok"]
    assert (await task)[0] == "Stop"


async def test_ask_timeout(agent):
    answer = await agent.ask_user("still there?", ["Yes", "No"], 0.2, "web")
    assert answer == "timeout" and not agent.asks and agent.link.sent[-1] == {"src": "cli"}
    assert [e["answer"] for e in agent.events if e["kind"] == "ask_done"] == ["timeout"]


async def test_ask_busy(client):
    a = client.agent
    fut = asyncio.get_running_loop().create_future()
    a.prompts["cli-1"] = {"tool": "Bash", "hint": "ls", "from": "x", "future": fut}
    text, err, _ = await client.call("ask", {"question": "ok?"})
    assert not err and text == "busy: an approval prompt is waiting on the stick"
    a.prompts.clear()
    a.state = "listening"
    text, _, _ = await client.call("ask", {"question": "ok?"})
    assert text == "busy: the user is talking to the stick"
    a.state = "ready"
    first = asyncio.create_task(a.ask_user("one?", ["Yes", "No"], 5, "x"))
    await until(lambda: a.asks)
    text, _, _ = await client.call("ask", {"question": "two?"})
    assert text == "busy: another question is waiting on the stick"
    first.cancel()
    with pytest.raises(asyncio.CancelledError):
        await first
    assert not a.asks


async def test_an_approval_goes_in_front_of_a_question(agent, monkeypatch):
    from stick_agent import core
    monkeypatch.setattr(core, "PROMPT_RESEND_S", 0.05)
    task = asyncio.create_task(agent.ask_user("ok?", ["Yes", "No"], 5, "x"))
    await until(lambda: asks_on_stick(agent))
    approval = asyncio.create_task(agent.ask_permission("Bash", "ls", "web", wait=5))
    await until(lambda: agent.prompts)
    shown = len(asks_on_stick(agent))
    await asyncio.sleep(0.2)
    assert len([p for p in asks_on_stick(agent) if p.get("kind") == "ask"]) <= shown + 1  # not re-sent meanwhile
    assert agent.home_card()["a"].startswith("Claude wants Bash")
    pid = next(iter(agent.prompts))
    agent._on_message({"cmd": "permission", "id": pid, "decision": "once"})
    assert await approval == "allow"
    before = len(asks_on_stick(agent))
    await until(lambda: len(asks_on_stick(agent)) > before and asks_on_stick(agent)[-1].get("kind") == "ask")
    agent._on_message({"cmd": "permission", "id": asks_on_stick(agent)[-1]["id"], "decision": "deny"})
    assert await task == "No"


async def test_ask_card_on_the_home_screen(agent):
    task = asyncio.create_task(agent.ask_user("Ship it?", ["Yes", "No"], 5, "web"))
    await until(lambda: agent.asks)
    assert agent.home_card() == {"k": "approval", "a": "web asks", "b": "Ship it?", "u": True}
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task


async def test_ask_input_checks(client):
    text, err, _ = await client.call("ask", {"question": "  "})
    assert err and "empty" in text
    text, err, _ = await client.call("ask", {"question": "x", "options": ["Same", "same"]})
    assert err and "differ" in text


async def test_cancelled_notification_cancels_a_question(client):
    a = client.agent
    task = asyncio.create_task(client.rpc("tools/call", {"name": "ask", "arguments": {"question": "ok?"}},
                                          headers={"Accept": "application/json"}))
    await until(lambda: a.asks)
    status, _ = await client.rpc("notifications/cancelled", {"requestId": client.n}, notification=True)
    assert status == 202
    status, body = await task
    assert body["error"]["message"] == "cancelled"
    await until(lambda: not a.asks)
    assert a.link.sent[-1] == {"src": "cli"}


# --- tools that change the Mac --------------------------------------------------------------

@pytest.fixture
def ran(monkeypatch):
    calls = []

    async def fake(args):
        calls.append(args)
    monkeypatch.setattr(mcp_server, "_run", fake)
    return calls


async def test_open_url(client, ran):
    text, err, _ = await client.call("open_url", {"url": "https://github.com/acme/web/pull/1"})
    assert not err and text == "opened github.com" and ran == [["open", "https://github.com/acme/web/pull/1"]]
    for bad in ("file:///etc/passwd", "javascript:alert(1)", "https://a b.com", "x-apple.systempreferences:foo",
                "https://", "https://ok.com/\n--args"):
        text, err, _ = await client.call("open_url", {"url": bad})
        assert err, bad
    assert len(ran) == 1


async def test_open_app(client, ran, monkeypatch):
    monkeypatch.setattr(mcp_server.commands, "find_app", lambda n: {"slack": "Slack"}.get(n.lower()))
    text, err, _ = await client.call("open_app", {"name": "Slack"})
    assert not err and ran == [["open", "-a", "Slack"]]
    for bad in ("Slack; rm -rf ~", "$(reboot)", "`x`", "Nope"):
        text, err, _ = await client.call("open_app", {"name": bad})
        assert err, bad
    assert len(ran) == 1


async def test_quick_command(client, monkeypatch):
    cmds = {"commands": [{"name": "Say", "phrases": ["say {q}"], "shell": "echo {q}"},
                         {"name": "PR", "phrases": ["pr {n}"], "open_url": "https://x.test/pull/{n}"}]}
    config.COMMANDS.write_text(json.dumps(cmds))
    text, err, _ = await client.call("quick_command", {"phrase": "say hello; touch /tmp/pwned-by-stick-test"})
    assert not err and text == "hello touch tmp pwned by stick test"  # (matching drops the punctuation)
    # ...and a slot that still holds shell syntax stays one quoted word, not a second command
    out = mcp_server.commands.run(cmds["commands"][0], {"q": "hi; echo pwned $(id)"}, quote_shell=True)
    assert out == "hi; echo pwned $(id)"
    runs = []
    monkeypatch.setattr(mcp_server.commands.subprocess, "run", lambda args, **kw: runs.append(args))
    text, err, _ = await client.call("quick_command", {"phrase": "PR 42"})
    assert not err and text == "Opened PR #42" and runs == [["open", "https://x.test/pull/42"]]
    text, err, _ = await client.call("quick_command", {"phrase": "make coffee"})
    assert err and "no quick command" in text


# --- dashboard API ------------------------------------------------------------------------

async def test_dashboard_status_and_install_needs_confirm(client, monkeypatch):
    monkeypatch.setattr(mcp_setup, "status",
                        lambda: {"installed": False, "detail": "not added", "command": "claude mcp add …"})
    r = await client.get("/api/mcp", headers=HOST)
    body = await r.json()
    assert body["enabled"] and body["terminal"]["installed"] is False and body["url"].endswith("/mcp")
    assert {t["name"] for t in body["tools"] if t["approval"]} == {"open_url", "open_app", "quick_command"}
    auth = {**HOST, "X-Stick-Token": config.token()}
    r = await client.post("/api/mcp/terminal/install", json={}, headers=auth)
    assert r.status == 400
    done = []
    def install(confirm=None, say=print):
        say("  claude mcp add x")
        done.append(confirm)
        return True, "added"
    monkeypatch.setattr(mcp_setup, "install", install)
    r = await client.post("/api/mcp/terminal/install", json={"confirm": True}, headers=auth)
    body = await r.json()
    assert body == {"ok": True, "message": "added", "ran": ["claude mcp add x"]} and done == [None]


# --- stick-agent mcp install / uninstall / status ---------------------------------------------

class FakeRun:
    """subprocess.run for `claude mcp …`: records the argv; `get` says whatever the test set."""

    def __init__(self, get_rc=1, get_out=""):
        self.calls = []
        self.get_rc, self.get_out = get_rc, get_out

    def __call__(self, args, **kw):
        self.calls.append(args)
        if args[1:3] == ["mcp", "get"]:
            return subprocess.CompletedProcess(args, self.get_rc, self.get_out, "")
        return subprocess.CompletedProcess(args, 0, "Added HTTP MCP server stick", "")


@pytest.fixture
def fake_run(home, monkeypatch):
    f = FakeRun()
    monkeypatch.setattr(mcp_setup.subprocess, "run", f)
    cfg = config.load()
    cfg["claude_path"] = "/opt/claude"
    config.save(cfg)
    return f


def test_install_builds_the_claude_mcp_add_command(fake_run):
    asked = []
    ok, msg = mcp_setup.install(confirm=lambda q: asked.append(q) or True, say=lambda s: None)
    assert ok and asked == ["Run this?"]
    add = fake_run.calls[-1]
    assert add == ["/opt/claude", "mcp", "add", "--transport", "http", "--scope", "user", "stick",
                   f"http://127.0.0.1:{config.PORT}/mcp", "--header", f"Authorization: Bearer {config.token()}",
                   "--header", "X-Stick-Cwd: ${PWD}"]


def test_install_shows_the_command_without_the_token(fake_run):
    lines = []
    mcp_setup.install(confirm=lambda q: False, say=lines.append)
    assert len(fake_run.calls) == 1  # only `claude mcp get`: declined, nothing added
    assert config.token() not in lines[0] and '"Authorization: Bearer <token>"' in lines[0]
    assert lines[0].strip().startswith("claude mcp add --transport http --scope user stick")


def test_install_when_already_there_or_someone_elses(fake_run):
    fake_run.get_rc, fake_run.get_out = 0, f"stick:\n  Scope: User config\n  URL: {mcp_setup.url()}\n"
    assert mcp_setup.install(say=lambda s: None) == (True, "already added to Claude Code")
    assert mcp_setup.status()["installed"] and "user" in mcp_setup.status()["detail"]
    fake_run.get_out = "stick:\n  URL: https://elsewhere.example/mcp\n"
    ok, msg = mcp_setup.install(say=lambda s: None)
    assert not ok and "isn't" in msg
    assert all(c[1:3] == ["mcp", "get"] for c in fake_run.calls)  # never replaced


def test_uninstall(fake_run):
    assert mcp_setup.uninstall(say=lambda s: None) == (True, "wasn't added — nothing to remove")
    fake_run.get_rc, fake_run.get_out = 0, f"URL: {mcp_setup.url()}"
    ok, _ = mcp_setup.uninstall(say=lambda s: None)
    assert ok and fake_run.calls[-1] == ["/opt/claude", "mcp", "remove", "--scope", "user", "stick"]


def test_cli(fake_run, capsys, monkeypatch):
    monkeypatch.setattr("sys.stdin.isatty", lambda: False)
    assert mcp_setup.cli(["install"]) == 1  # not a terminal and no --yes: doesn't run it
    assert "add --yes" in capsys.readouterr().out and len(fake_run.calls) == 1
    assert mcp_setup.cli(["install", "--yes"]) == 0
    assert fake_run.calls[-1][1:3] == ["mcp", "add"]
    assert mcp_setup.cli(["status"]) == 0
    assert "stick tools for Claude Code" in capsys.readouterr().out


def test_no_claude(home, monkeypatch):
    monkeypatch.setattr(mcp_setup.config, "claude_path", lambda cfg: None)
    assert mcp_setup.status()["installed"] is None
    assert not mcp_setup.install(say=lambda s: None)[0]


def test_installer_component(fake_run):
    from stick_agent import installer
    c = installer.BY_ID["mcp"]
    assert not c.default and c.from_dashboard
    state, detail = c.status()
    assert state == "missing" and "claude mcp add" in detail and config.token() not in detail
    c.install(installer.UI(interactive=False))
    assert fake_run.calls[-1][1:3] == ["mcp", "add"]
