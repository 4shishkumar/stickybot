"""Update checks: scheduling (start-up + every 24 h, the switch), what counts as an
update (agent version, firmware vs the connected stick), the stick toast (once per
release, not in quiet hours, not without the stick), the state file, and the API.
GitHub is faked: releases.latest / download_firmware are replaced."""
import asyncio
import time

import pytest

from stick_agent import __version__, config, events, releases, server, updates

HOST = {"Host": f"localhost:{config.PORT}"}


def release(tag, fw=None):
    v = tag[1:]
    assets = {"manifest.json": {"size": 1}} if fw else {}
    return {"tag": tag, "version": v, "prerelease": releases.is_prerelease(v), "name": f"stick-key {tag}",
            "notes": f"### Added\n- things in {tag}", "url": f"https://example/{tag}", "published": "2026-11-06",
            "assets": assets}


class FakeGitHub:
    def __init__(self):
        self.rel = None          # what releases.latest returns (None = no release yet)
        self.fail = None
        self.fw = "1.7.2"
        self.channels = []

    def latest(self, chan="stable", repo=None):
        self.channels.append(chan)
        if self.fail:
            raise releases.ReleaseError(self.fail)
        if self.rel is None:
            raise releases.NoRelease(releases.NO_RELEASE)
        return self.rel, object()

    def download(self, rel, src, root=None, say=None):
        return None, {"version": self.fw}


@pytest.fixture
def gh(monkeypatch):
    fake = FakeGitHub()
    monkeypatch.setattr(releases, "latest", fake.latest)
    monkeypatch.setattr(releases, "download_firmware", fake.download)
    return fake


class FakeStickApi:
    def __init__(self, version="1.7.2"):
        self.version = version
        self.asked = 0

    async def connected(self):
        self.asked += 1
        return {"name": "my-stick", "version": self.version} if self.version else None


@pytest.fixture
def up(agent, gh, monkeypatch):
    monkeypatch.setattr(events.time, "localtime", lambda: time.struct_time((2026, 11, 6, 12, 0, 0, 4, 310, 0)))
    agent.stick_api = FakeStickApi()
    clock = [1_000_000.0]
    agent.updates = updates.Updates(agent, clock=lambda: clock[0])
    agent.updates.tick = clock
    return agent.updates


def toasts(agent):
    return [m["event"] for m in agent.link.sent if "event" in m]


def bump(v):
    major, minor, patch, _ = releases.parse_version(v)
    return f"v{major}.{minor}.{patch + 1}"


# --- scheduling -------------------------------------------------------------------------

async def test_checks_at_start_up_then_every_24_hours(up, gh):
    assert up.due()                                 # never checked since start-up
    gh.rel = release(f"v{__version__}", fw="1.7.2")
    await up.check()
    assert not up.due()
    up.tick[0] += updates.CHECK_EVERY_S - 60
    assert not up.due()
    up.tick[0] += 120
    assert up.due()


async def test_a_restart_checks_again_but_the_switch_turns_it_off(up, gh, agent):
    gh.rel = release(f"v{__version__}")
    await up.check()
    again = updates.Updates(agent, clock=lambda: up.tick[0] + 5)
    assert again.checked_at == up.checked_at and again.due()   # remembered, but a fresh start checks once
    agent.cfg["update_check"] = False
    assert not again.due()


async def test_the_loop_checks_when_due(up, gh, monkeypatch):
    gh.rel = release(f"v{__version__}")
    monkeypatch.setattr(updates, "FIRST_CHECK_S", 0)
    monkeypatch.setattr(updates, "TICK_S", 0.01)
    task = asyncio.ensure_future(up.run())
    try:
        for _ in range(100):
            if up.ran:
                break
            await asyncio.sleep(0.01)
    finally:
        task.cancel()
    assert up.ran and gh.channels == ["stable"]


# --- what counts as an update --------------------------------------------------------------

async def test_a_newer_release_is_an_agent_update(up, gh, agent):
    tag = bump(__version__)
    gh.rel = release(tag, fw="1.7.2")
    st = await up.check()
    assert st["agent_update"] and not st["firmware_update"] and st["available"]
    assert st["title"] == f"Update available: {tag}"
    assert st["latest"]["notes"].startswith("### Added") and st["latest"]["firmware"] == "1.7.2"
    assert agent.status()["update"] == {"available": True, "title": f"Update available: {tag}", "agent_update": True,
                                        "firmware_update": False, "checking": False}
    assert [e for e in agent.events if e["kind"] == "update"][-1]["available"] is True


async def test_the_same_version_is_no_update(up, gh):
    gh.rel = release(f"v{__version__}", fw="1.7.2")
    st = await up.check()
    assert not st["available"] and st["title"] == "" and st["current"]["firmware"] == "1.7.2"


async def test_newer_firmware_than_the_connected_stick(up, gh, agent):
    gh.rel, gh.fw = release(f"v{__version__}", fw="1.8.0"), "1.8.0"
    st = await up.check()
    assert st["firmware_update"] and not st["agent_update"] and st["title"] == "Firmware 1.8.0 available"
    agent.stick_api.version = "1.8.0"
    await up.refresh_stick(force=True)
    assert not up.status()["available"]


async def test_no_stick_no_firmware_update(up, gh, agent):
    agent.link.connected = False
    gh.rel, gh.fw = release(f"v{__version__}", fw="1.8.0"), "1.8.0"
    st = await up.check()
    assert not st["firmware_update"] and st["current"]["firmware"] is None


async def test_no_release_yet_and_errors(up, gh):
    st = await up.check()
    assert not st["available"] and "no release yet" in st["note"] and st["error"] == ""
    gh.rel = release(bump(__version__))
    await up.check()
    gh.fail = "gh: offline"
    st = await up.check()
    assert st["error"] == "gh: offline" and st["agent_update"]   # the last good answer stays


async def test_the_beta_channel(up, gh, agent):
    agent.cfg["update_channel"] = "beta"
    gh.rel = release(bump(__version__) + "-beta.1")
    st = await up.check()
    assert gh.channels == ["beta"] and st["channel"] == "beta" and st["agent_update"]
    agent.cfg["update_channel"] = "nonsense"
    assert up.status()["channel"] == "stable"


async def test_changing_the_channel_checks_again(agent, monkeypatch):
    calls = []

    async def check():
        calls.append(agent.cfg["update_channel"])
    monkeypatch.setattr(agent.updates, "check", check)
    agent.loop = asyncio.get_running_loop()
    agent.update_config({"update_channel": "stable", "update_check": True})  # unchanged: the Save button
    agent.update_config({"update_channel": "beta"})
    await asyncio.sleep(0)
    assert calls == ["beta"]


# --- the toast on the stick ------------------------------------------------------------------

async def test_one_toast_per_release(up, gh, agent):
    tag = bump(__version__)
    gh.rel = release(tag)
    await up.check()
    await up.notify()
    await up.check()
    shown = toasts(agent)
    assert len(shown) == 1 and shown[0]["level"] == "nice" and shown[0]["title"] == f"Update {tag} ready"
    assert shown[0]["sub"] == "see the dashboard"
    later = updates.Updates(agent)          # after a restart: still once
    await later.notify()
    assert len(toasts(agent)) == 1
    newer = bump(tag[1:])
    gh.rel = release(newer)
    await up.check()
    assert [e["title"] for e in toasts(agent)][-1] == f"Update {newer} ready"


async def test_no_toast_in_quiet_hours_then_after(up, gh, agent, monkeypatch):
    agent.cfg["quiet_hours"] = {"on": True, "from": "19:00", "to": "09:00"}
    monkeypatch.setattr(events.time, "localtime", lambda: time.struct_time((2026, 11, 6, 23, 0, 0, 4, 310, 0)))
    gh.rel = release(bump(__version__))
    await up.check()
    assert toasts(agent) == []
    monkeypatch.setattr(events.time, "localtime", lambda: time.struct_time((2026, 11, 7, 10, 0, 0, 5, 311, 0)))
    await up.notify()
    assert len(toasts(agent)) == 1


async def test_no_toast_without_the_stick_then_when_it_connects(up, gh, agent):
    agent.link.connected = False
    gh.rel = release(bump(__version__))
    await up.check()
    assert up.notified == []
    agent.link.connected = True
    await up.notify()
    assert len(toasts(agent)) == 1


async def test_the_toast_can_be_turned_off(up, gh, agent):
    agent.cfg["event_rules"] = {"update": "off"}
    gh.rel = release(bump(__version__))
    await up.check()
    assert toasts(agent) == [] and up.notified == [gh.rel["tag"]]


async def test_firmware_only_toast(up, gh, agent):
    gh.rel, gh.fw = release(f"v{__version__}", fw="1.9.0"), "1.9.0"
    await up.check()
    assert toasts(agent)[0]["title"] == "Update firmware 1.9.0 ready"


# --- API ------------------------------------------------------------------------------------

async def test_update_api(up, gh, agent, aiohttp_client):
    client = await aiohttp_client(server.make_app(agent))
    gh.rel = release(bump(__version__))
    r = await client.get("/api/update", headers=HOST)
    assert r.status == 200 and (await r.json())["latest"] is None
    assert (await client.post("/api/update/check", headers=HOST)).status == 403   # needs the token
    r = await client.post("/api/update/check", headers={**HOST, "X-Stick-Token": config.token()})
    d = await r.json()
    assert d["agent_update"] and d["latest"]["notes"]
    d = await (await client.get("/api/update", headers=HOST)).json()
    assert d["latest"]["tag"] == gh.rel["tag"] and d["checked_at"]
    s = await (await client.get("/api/status", headers=HOST)).json()
    assert s["update"]["available"] is True


async def test_settings_are_in_the_config(agent, aiohttp_client):
    client = await aiohttp_client(server.make_app(agent))
    assert agent.cfg["update_check"] is True and agent.cfg["update_channel"] == "stable"
    r = await client.put("/api/config", headers={**HOST, "X-Stick-Token": config.token()},
                         json={"update_check": False})
    assert (await r.json())["update_check"] is False
