"""The OpenRouter brain (openai_brain.py), the brain switch (brains.py), the Keychain module and the
Settings → Brain API (brain_api.py). A fake OpenRouter (an aiohttp test server) stands in for the
real one; keys live in keychain.MemoryBackend (conftest.py) — nothing reaches the network or the
real Keychain."""
import asyncio
import json
import logging
import time

import pytest
from aiohttp import web

from stick_agent import brains, config, keychain, mcp_server, openai_brain, server

HOST = {"Host": f"localhost:{config.PORT}"}
KEY = "sk-or-v1-" + "0123456789abcdef" * 4
MODEL = "test/fast-model"


def text_chunk(t):
    return {"choices": [{"index": 0, "delta": {"content": t}}]}


def tool_chunk(i, call_id, name, args):
    return {"choices": [{"index": 0, "delta": {"tool_calls": [
        {"index": i, "id": call_id, "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}]}}]}


def usage_chunk(cost=0.00012, finish="stop"):
    return {"choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": finish}],
            "usage": {"prompt_tokens": 50, "completion_tokens": 8, "total_tokens": 58, "cost": cost}}


def answer(*parts, cost=0.00012):
    return ("stream", [text_chunk(p) for p in parts] + [usage_chunk(cost)])


def tool_call(name, args, preamble="", call_id="call_1"):
    chunks = [text_chunk(preamble)] if preamble else []
    return ("stream", chunks + [tool_chunk(0, call_id, name, args), usage_chunk(0.0001, "tool_calls")])


class FakeOpenRouter:
    """POST /chat/completions answers with the next scripted reply; GET /models with a model list."""

    def __init__(self):
        self.replies = []      # ("stream", [chunks]) | ("status", code, message) | ("hang", [chunks])
        self.requests = []     # {"headers", "body"}
        self.model_list = [
            {"id": MODEL, "name": "Test: Fast", "pricing": {"prompt": "0.0000001", "completion": "0.0000004"},
             "supported_parameters": ["tools", "max_tokens"], "architecture": {"output_modalities": ["text"]},
             "context_length": 100000},
            {"id": "test/no-tools", "name": "Test: No tools", "pricing": {"prompt": "0.00000005", "completion": "0"},
             "supported_parameters": ["max_tokens"], "architecture": {"output_modalities": ["text"]}},
            {"id": "test/image", "name": "Image maker", "pricing": {"prompt": "0", "completion": "0"},
             "architecture": {"output_modalities": ["image"]}},
        ]
        self.model_fetches = 0
        self.disconnected = asyncio.Event()

    async def chat(self, request):
        body = await request.json()
        self.requests.append({"headers": dict(request.headers), "body": body})
        spec = self.replies.pop(0) if self.replies else answer("OK.")
        if spec[0] == "status":
            return web.json_response({"error": {"code": spec[1], "message": spec[2]}}, status=spec[1])
        resp = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
        await resp.prepare(request)
        try:
            await resp.write(b": OPENROUTER PROCESSING\n\n")
            for c in spec[1]:
                await resp.write(f"data: {json.dumps(c)}\n\n".encode())
            if spec[0] == "hang":
                for _ in range(500):
                    await asyncio.sleep(0.01)
                    await resp.write(b": OPENROUTER PROCESSING\n\n")
                return resp
            await resp.write(b"data: [DONE]\n\n")
        except (ConnectionResetError, asyncio.CancelledError):
            self.disconnected.set()
            raise
        return resp

    async def models(self, _):
        self.model_fetches += 1
        return web.json_response({"data": self.model_list})


@pytest.fixture
async def fake(aiohttp_server):
    f = FakeOpenRouter()
    app = web.Application()
    app.router.add_post("/api/v1/chat/completions", f.chat)
    app.router.add_get("/api/v1/models", f.models)
    srv = await aiohttp_server(app)
    f.url = str(srv.make_url("/api/v1"))
    return f


def set_config(**kw):
    cfg = config.load()
    cfg.update(kw)
    config.save(cfg)


@pytest.fixture
async def orb(agent, fake, monkeypatch):
    """An OpenRouter brain on the fake server, with a key and a model set."""
    monkeypatch.setattr(openai_brain.OpenRouterBrain, "base_url", fake.url)
    keychain.save("openrouter", KEY)
    set_config(openrouter_model=MODEL)
    agent.cfg = config.load()
    b = openai_brain.OpenRouterBrain(agent)
    yield b
    await b.close()


class Collect:
    def __init__(self):
        self.progress, self.texts = [], []

    async def on_progress(self, line):
        self.progress.append(line)

    async def on_text(self, text):
        self.texts.append(text)


async def ask(b, text="what's the capital of France?"):
    c = Collect()
    ok, reply = await b.ask(text, c.on_progress, c.on_text)
    return ok, reply, c


# --- streaming, headers, usage --------------------------------------------------------------

async def test_streams_the_answer_with_cost(orb, fake):
    fake.replies = [answer("It's **Paris**. ", "Lovely city.", cost=0.00025)]
    ok, reply, c = await ask(orb)
    assert ok and reply == "It's Paris. Lovely city."
    assert c.texts[-1] == "It's **Paris**. Lovely city." and len(c.texts) >= 2  # as it was written
    req = fake.requests[0]
    assert req["headers"]["Authorization"] == f"Bearer {KEY}"
    assert req["headers"]["HTTP-Referer"] == "https://github.com/ashish-kumar-noon/stick-key"
    assert req["headers"]["X-Title"] == "stick-key"
    body = req["body"]
    assert body["model"] == MODEL and body["stream"] is True and body["max_tokens"] == 400
    assert body["messages"][0]["role"] == "system" and "135x240" in body["messages"][0]["content"]
    assert body["messages"][-1] == {"role": "user", "content": "what's the capital of France?"}
    assert {t["function"]["name"] for t in body["tools"]} == set(mcp_server.BY_NAME)
    last = orb.last
    assert last["ok"] and last["cost"] == pytest.approx(0.00025) and last["first_token_ms"] is not None
    st = orb.status()
    assert st["provider"] == "OpenRouter" and st["model"] == MODEL and st["avg"]["openrouter"]["n"] == 1


async def test_mid_stream_error_is_a_failure(orb, fake):
    fake.replies = [("stream", [text_chunk("Hel"), {"error": {"code": 502, "message": "provider fell over"},
                                                      "choices": [{"delta": {}, "finish_reason": "error"}]}])]
    ok, reply, _ = await ask(orb)
    assert not ok and "unavailable" in reply
    assert orb.history == []  # a failed turn isn't remembered


# --- tools ---------------------------------------------------------------------------------

async def test_tool_round_trip(orb, fake, agent, monkeypatch):
    async def report(wait=2.0):
        return {"bat": {"pct": 81, "usb": False}}
    monkeypatch.setattr(agent, "stick_report", report)
    fake.replies = [tool_call("stick_status", {}, preamble="Let me check."), answer("Battery is at 81%.")]
    ok, reply, c = await ask(orb, "how's the stick's battery?")
    assert ok and reply == "Battery is at 81%."
    assert "Let me check." in c.texts and c.texts[c.texts.index("Let me check.") + 1] == ""  # preamble withdrawn
    assert c.progress == ["Checking the stick", "Thinking…"]
    second = fake.requests[1]["body"]["messages"]
    assert second[-2]["role"] == "assistant" and second[-2]["tool_calls"][0]["function"]["name"] == "stick_status"
    assert second[-1]["role"] == "tool" and second[-1]["tool_call_id"] == "call_1"
    assert "battery 81%" in second[-1]["content"]
    assert orb.last["cost"] == pytest.approx(0.00022) and orb.last["tools"] == ["stick_status"]
    assert [m["role"] for m in orb.history] == ["user", "assistant", "tool", "assistant"]


async def approve_when_asked(agent, allow):
    for _ in range(300):
        if agent.prompts:
            pid, p = next(iter(agent.prompts.items()))
            assert p["tool"] == "open_url" and p["hint"] == "https://example.com/docs" and p["from"] == "Agent"
            agent.decide(pid, allow)
            return
        await asyncio.sleep(0.01)
    raise AssertionError("no approval prompt")


@pytest.mark.parametrize("allow", [True, False])
async def test_mac_tool_asks_on_the_stick(orb, fake, agent, monkeypatch, allow):
    ran = []

    async def run(args):
        ran.append(args)
    monkeypatch.setattr(mcp_server, "_run", run)
    fake.replies = [tool_call("open_url", {"url": "https://example.com/docs"}),
                    answer("Opened it." if allow else "OK, I won't.")]
    approver = asyncio.ensure_future(approve_when_asked(agent, allow))
    ok, reply, c = await ask(orb, "open the docs")
    await approver
    assert ok and c.progress[0] == "Opening example.com/docs"
    assert any(m.get("prompt", {}).get("tool") == "open_url" for m in agent.link.sent)  # shown on the stick
    result = fake.requests[1]["body"]["messages"][-1]["content"]
    if allow:
        assert ran == [["open", "https://example.com/docs"]] and result == "opened example.com"
    else:
        assert ran == [] and result == "error: the user didn't approve it"


async def test_mac_tool_without_stick_approvals_fails_closed(orb, fake, agent, monkeypatch):
    agent.cfg["stick_approvals"] = False
    monkeypatch.setattr(mcp_server, "_run", lambda args: (_ for _ in ()).throw(AssertionError("ran")))
    fake.replies = [tool_call("open_app", {"name": "Slack"}), answer("I can't without approval.")]
    ok, _, _ = await ask(orb, "open slack")
    assert ok and "approvals are turned off" in fake.requests[1]["body"]["messages"][-1]["content"]


async def test_tool_loop_is_capped(orb, fake, agent, monkeypatch):
    async def report(wait=2.0):
        return {}
    monkeypatch.setattr(agent, "stick_report", report)
    fake.replies = [tool_call("stick_status", {}, call_id=f"c{i}") for i in range(openai_brain.MAX_ROUNDS)]
    fake.replies.append(answer("Done checking."))
    ok, reply, _ = await ask(orb)
    assert ok and reply == "Done checking."
    assert len(fake.requests) == openai_brain.MAX_ROUNDS + 1
    assert "tools" in fake.requests[-2]["body"] and "tools" not in fake.requests[-1]["body"]


async def test_model_without_tools_falls_back(orb, fake):
    fake.replies = [("status", 404, "No endpoints found that support tool use. Try disabling \"tools\"."),
                    answer("Hi."), answer("Hi again.")]
    ok, reply, _ = await ask(orb)
    assert ok and reply == "Hi."
    assert "tools" in fake.requests[0]["body"] and "tools" not in fake.requests[1]["body"]
    await ask(orb)
    assert "tools" not in fake.requests[2]["body"]  # remembered for this model


async def test_model_list_says_no_tools(orb, fake):
    set_config(openrouter_model="test/no-tools")
    await orb.list_models()
    fake.replies = [answer("Hi.")]
    ok, _, _ = await ask(orb)
    assert ok and "tools" not in fake.requests[0]["body"]


async def test_stick_tools_off_means_no_tools(orb, fake):
    set_config(mcp_server=False)
    await ask(orb)
    assert "tools" not in fake.requests[0]["body"]


# --- errors ----------------------------------------------------------------------------------

async def test_no_key(orb, fake):
    keychain.remove("openrouter")
    ok, reply, _ = await ask(orb)
    assert not ok and reply == "Add your OpenRouter key in Settings → Brain."
    assert fake.requests == []


async def test_no_model(orb, fake):
    set_config(openrouter_model="")
    ok, reply, _ = await ask(orb)
    assert not ok and "Pick an OpenRouter model" in reply and fake.requests == []


@pytest.mark.parametrize("status, words", [(401, "rejected the key"), (402, "Out of OpenRouter credits"),
                                           (429, "rate limiting"), (503, "unavailable")])
async def test_http_errors(orb, fake, status, words):
    fake.replies = [("status", status, "nope")]
    ok, reply, _ = await ask(orb)
    assert not ok and words in reply
    assert orb.last["error"] == reply


async def test_network_error(orb, monkeypatch, unused_tcp_port):
    monkeypatch.setattr(openai_brain.OpenRouterBrain, "base_url", f"http://127.0.0.1:{unused_tcp_port}/api/v1")
    ok, reply, _ = await ask(orb)
    assert not ok and reply == "Couldn't reach OpenRouter. Check the internet connection."


async def test_cancel_mid_stream(orb, fake):
    fake.replies = [("hang", [text_chunk("Once upon a time")])]
    c = Collect()
    task = asyncio.ensure_future(orb.ask("tell me a story", c.on_progress, c.on_text))
    for _ in range(300):
        if c.texts and c.texts[-1]:
            break
        await asyncio.sleep(0.01)
    assert orb.busy
    t = time.monotonic()
    orb.cancel()
    ok, reply = await asyncio.wait_for(task, 2)
    assert (ok, reply) == (False, "Cancelled.") and orb.cancelled and not orb.busy
    assert time.monotonic() - t < 1
    await asyncio.wait_for(fake.disconnected.wait(), 3)  # the HTTP stream was closed
    assert orb.history == []


async def test_busy_rejects_a_second_request(orb, fake):
    fake.replies = [("hang", [text_chunk("...")])]
    task = asyncio.ensure_future(orb.ask("one", None, None))
    for _ in range(300):
        if fake.requests:
            break
        await asyncio.sleep(0.01)
    assert await orb.ask("two", None, None) == (False, "Still working on the last request.")
    orb.cancel()
    await task


# --- the conversation -------------------------------------------------------------------------

async def test_history_forget_and_idle(orb, fake):
    fake.replies = [answer("Paris."), answer("About two million."), answer("Hi."), answer("Hello.")]
    await ask(orb, "capital of France?")
    await ask(orb, "how many people live there?")
    msgs = fake.requests[1]["body"]["messages"]
    assert [m["role"] for m in msgs] == ["system", "user", "assistant", "user"]
    assert msgs[2]["content"] == "Paris."
    orb.forget()
    await ask(orb, "hi")
    assert [m["role"] for m in fake.requests[2]["body"]["messages"]] == ["system", "user"]
    orb.last_at = time.time() - 11 * 60  # brain_idle_min (10) passed
    await ask(orb, "hello")
    assert [m["role"] for m in fake.requests[3]["body"]["messages"]] == ["system", "user"]


def test_history_is_trimmed_by_whole_turns():
    b = openai_brain.OpenAICompatBrain()
    for i in range(40):
        b._remember([{"role": "user", "content": f"q{i}"}, {"role": "assistant", "content": "", "tool_calls": []},
                     {"role": "tool", "content": "r"}, {"role": "assistant", "content": f"a{i}"}])
    assert len(b.history) <= openai_brain.HISTORY_MESSAGES
    assert b.history[0]["role"] == "user" and b.history[-1]["content"] == "a39"


# --- the model list -------------------------------------------------------------------------

async def test_model_list_is_cached(orb, fake):
    models, suggested, err = await orb.list_models()
    assert err is None and [m["id"] for m in models] == [MODEL, "test/no-tools"]  # the image model is out
    m = models[0]
    assert m["in"] == pytest.approx(0.1) and m["out"] == pytest.approx(0.4) and m["tools"] is True
    assert suggested == MODEL  # no preferred one listed: the cheapest that can use tools
    await orb.list_models()
    assert fake.model_fetches == 1
    await orb.list_models(force=True)
    assert fake.model_fetches == 2


async def test_model_list_offline_keeps_the_old_one(orb, fake, monkeypatch, unused_tcp_port):
    await orb.list_models()
    monkeypatch.setattr(openai_brain.OpenRouterBrain, "base_url", f"http://127.0.0.1:{unused_tcp_port}/api/v1")
    models, _, err = await orb.list_models(force=True)
    assert err and len(models) == 2


def test_suggest_prefers_known_fast_models():
    models = [{"id": "x/pricey", "in": 5.0, "out": 20.0, "tools": True},
              {"id": "anthropic/claude-haiku-5.5", "in": 0.1, "out": 0.5, "tools": True},
              {"id": "x/cheap:free", "in": 0.0, "out": 0.0, "tools": True}]
    assert openai_brain.suggest(models) == "anthropic/claude-haiku-5.5"
    assert openai_brain.suggest(models[:1] + models[2:]) == "x/pricey"
    assert openai_brain.suggest([]) is None


# --- the brain switch ---------------------------------------------------------------------------

class FakeClaude:
    busy = alive = cancelled = False
    pid = 4242

    def __init__(self):
        self.asked, self.forgot, self.warmed = [], 0, 0

    async def ask(self, text, on_progress, on_text=None):
        self.asked.append(text)
        return True, "Claude says hi."

    def forget(self):
        self.forgot += 1

    def cancel(self):
        pass

    async def prewarm(self):
        self.warmed += 1

    def status(self):
        return {"mode": "session", "alive": False}


@pytest.fixture
async def switch(agent, orb):
    sw = brains.Brains(agent)
    sw.claude = FakeClaude()
    sw.openrouter = orb
    agent.brain = sw
    return sw


async def test_switching_brains_without_a_restart(switch, agent, fake):
    assert switch.name == "claude" and switch.status()["brain"] == "claude"
    assert await switch.ask("hi", None) == (True, "Claude says hi.")
    await switch.prewarm()
    assert switch.claude.warmed == 1
    agent.update_config({"brain": "openrouter"})
    assert switch.claude.forgot == 1  # Claude's session stops
    fake.replies = [answer("OpenRouter says hi.")]
    assert await switch.ask("hi", None) == (True, "OpenRouter says hi.")
    assert switch.claude.asked == ["hi"] and switch.status()["brain"] == "openrouter"
    await switch.prewarm()
    assert switch.claude.warmed == 1  # no Claude Code start-up while OpenRouter is picked
    assert switch.pid == 4242
    agent.update_config({"brain": "claude"})
    assert switch.openrouter.history == []


async def test_voice_request_through_openrouter(switch, agent, fake):
    agent.update_config({"brain": "openrouter"})
    fake.replies = [answer("It's Paris.")]
    await agent.handle_text("capital of France?")
    assert agent.link.states()[-1] == ("done", "It's Paris.")


async def test_new_chat_phrase_forgets_openrouter(switch, agent, fake):
    agent.update_config({"brain": "openrouter"})
    fake.replies = [answer("Paris."), answer("Hi.")]
    await agent.handle_text("capital of France?")
    assert switch.openrouter.history
    await agent.handle_text("new chat")
    assert switch.openrouter.history == []


# --- the Keychain module ------------------------------------------------------------------------

def test_keychain_status_never_has_the_key(no_real_keychain):
    assert keychain.status("openrouter")["saved"] is False
    keychain.save("openrouter", KEY)
    st = keychain.status("openrouter")
    assert st == {"saved": True, "source": "keychain", "hint": "…cdef", "env_var": "STICK_AGENT_OPENROUTER_KEY"}
    assert KEY not in json.dumps(st) and no_real_keychain.items["openrouter"] == KEY
    assert keychain.remove("openrouter") and keychain.status("openrouter")["saved"] is False


def test_env_var_overrides(monkeypatch):
    keychain.save("openrouter", KEY)
    monkeypatch.setenv("STICK_AGENT_OPENROUTER_KEY", "sk-or-dev-" + "9" * 30)
    assert keychain.get("openrouter").startswith("sk-or-dev-")
    assert keychain.status("openrouter")["source"] == "env"


def test_known_status_doesnt_read_the_keychain(no_real_keychain):
    no_real_keychain.items["openrouter"] = KEY  # saved earlier, not read yet
    assert keychain.known_status("openrouter")["saved"] is None
    keychain.get("openrouter")
    assert keychain.known_status("openrouter")["hint"] == "…cdef"


# --- the dashboard API ------------------------------------------------------------------------

@pytest.fixture
async def client(switch, agent, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def test_key_api_is_write_only(client, agent, caplog):
    caplog.set_level(logging.DEBUG)
    keychain.remove("openrouter")
    other = "sk-or-v1-" + "fedcba9876543210" * 4
    r = await client.put("/api/brain/openrouter/key", headers=HOST, json={"key": other})
    assert r.status == 403  # needs the token
    r = await client.put("/api/brain/openrouter/key", headers=client.auth, json={"key": f"  {other}\n"})
    body = await r.text()
    assert r.status == 200 and other not in body and json.loads(body)["key"]["hint"] == "…3210"
    assert keychain.get("openrouter") == other
    for path in ("/api/brain", "/api/config", "/api/status"):
        text = await (await client.get(path, headers=HOST)).text()
        assert other not in text, path
    assert other not in config.CONFIG.read_text()
    assert other not in caplog.text
    r = await client.delete("/api/brain/openrouter/key", headers=client.auth)
    assert (await r.json())["key"]["saved"] is False and keychain.get("openrouter") is None


@pytest.mark.parametrize("bad", [None, "", "short", "has spaces in it 1234567890", 42, "x" * 400])
async def test_key_api_validates(client, bad):
    r = await client.put("/api/brain/openrouter/key", headers=client.auth, json={"key": bad})
    assert r.status == 400


async def test_brain_status_api(client):
    s = await (await client.get("/api/brain", headers=HOST)).json()
    assert s["brain"] == "claude" and s["choices"] == ["claude", "openrouter"]
    assert s["openrouter"]["key"]["saved"] is True and s["openrouter"]["model"] == MODEL


async def test_models_api(client, fake):
    r = await client.get("/api/brain/openrouter/models", headers=HOST)
    body = await r.json()
    assert r.status == 200 and body["suggested"] == MODEL and len(body["models"]) == 2
    await client.get("/api/brain/openrouter/models", headers=HOST)
    assert fake.model_fetches == 1
    await client.get("/api/brain/openrouter/models?refresh=1", headers=HOST)
    assert fake.model_fetches == 2


async def test_test_api(client, fake):
    fake.replies = [answer("Hello from the test model.", cost=0.00001)]
    r = await client.post("/api/brain/test", headers=client.auth, json={"model": "test/other"})
    body = await r.json()
    assert body["ok"] and body["reply"] == "Hello from the test model." and body["cost"] == pytest.approx(0.00001)
    assert body["total_ms"] >= 0 and body["model"] == "test/other"
    req = fake.requests[0]["body"]
    assert req["model"] == "test/other" and "tools" not in req and req["max_tokens"] == 60
    r = await client.post("/api/brain/test", headers=client.auth, json={"model": "bad model id!"})
    assert r.status == 400
    fake.replies = [("status", 401, "no")]
    body = await (await client.post("/api/brain/test", headers=client.auth, json={})).json()
    assert not body["ok"] and "rejected the key" in body["error"]


@pytest.mark.parametrize("bad", [{"brain": "gpt"}, {"openrouter_model": "no spaces allowed"},
                                 {"openrouter_max_tokens": 5}, {"openrouter_model": 3}])
async def test_config_validates_brain_settings(client, bad):
    r = await client.put("/api/config", headers=client.auth, json=bad)
    assert r.status == 400


async def test_config_switches_the_brain(client, switch, fake):
    r = await client.put("/api/config", headers=client.auth, json={"brain": "openrouter", "openrouter_model": MODEL})
    assert r.status == 200 and switch.name == "openrouter" and switch.claude.forgot == 1
