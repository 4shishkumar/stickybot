"""Events for the stick: levels, quiet hours, what's sent and logged."""
import json
import time

import pytest

from stick_agent import events


def at(monkeypatch, hh, mm=0):
    monkeypatch.setattr(events.time, "localtime", lambda: time.struct_time((2026, 10, 20, hh, mm, 0, 1, 293, 0)))


@pytest.mark.parametrize("cfg, hh, quiet", [
    ({}, 23, False),                                                        # off by default
    ({"quiet_hours": {"on": True, "from": "19:00", "to": "09:00"}}, 23, True),   # wraps midnight
    ({"quiet_hours": {"on": True, "from": "19:00", "to": "09:00"}}, 3, True),
    ({"quiet_hours": {"on": True, "from": "19:00", "to": "09:00"}}, 12, False),
    ({"quiet_hours": {"on": True, "from": "13:00", "to": "14:30"}}, 14, True),   # same-day window
    ({"quiet_hours": {"on": True, "from": "13:00", "to": "14:30"}}, 15, False),
    ({"quiet_hours": {"on": True, "from": "bad", "to": "09:00"}}, 23, False),
])
def test_quiet_hours(monkeypatch, cfg, hh, quiet):
    at(monkeypatch, hh)
    assert events.quiet_now(cfg) is quiet


def test_rules_default_and_override():
    r = events.rules({"event_rules": {"release": "off", "pr_merged": "nice", "ci_failed": "bogus"}})
    assert r["release"] == "off" and r["pr_merged"] == "nice"
    assert r["ci_failed"] == "headsup"          # invalid level ignored
    assert set(r) == set(events.KINDS)


async def test_notify_sends_logs_and_emits(agent, monkeypatch):
    at(monkeypatch, 12)
    ev = await agent.notifier.notify("pr_merged", "PR #12 merged!", "loom - Café ✨ fix")
    assert ev["level"] == "celebrate" and ev["icon"] == "merge" and ev["id"] == "e1"
    msg = agent.link.sent[-1]
    assert msg == {"src": "agent", "event": ev}
    assert ev["sub"] == "loom - Cafe  fix"          # sanitised for the stick's fonts
    logged = [json.loads(line) for line in events.EVENTS_LOG.read_text().splitlines()]
    assert logged[-1]["kind"] == "pr_merged" and logged[-1]["sent"] is True
    assert any(e["kind"] == "event" for e in agent.events)
    assert events.history()[-1]["title"] == "PR #12 merged!"


async def test_notify_off_sends_nothing(agent):
    agent.cfg["event_rules"] = {"release": "off"}
    assert await agent.notifier.notify("release", "Release v1", "loom") is None
    assert agent.link.sent == []


async def test_quiet_hours_mute_the_sound(agent, monkeypatch):
    agent.cfg["quiet_hours"] = {"on": True, "from": "19:00", "to": "09:00"}
    at(monkeypatch, 22)
    ev = await agent.notifier.notify("claude_done", "Claude finished", "took 3 min")
    assert ev["sound"] == "off"


async def test_titles_are_trimmed(agent):
    ev = await agent.notifier.notify("test", "x" * 80, "y" * 200)
    assert len(ev["title"]) == 40 and len(ev["sub"]) == 90


async def test_test_kind_and_ack(agent):
    ev = await agent.notifier.test("ci_failed")
    assert ev["level"] == "headsup" and ev["title"] == "CI failed"
    agent._on_message({"src": "agent", "evt": "event_shown", "id": ev["id"]})
    assert ev["id"] in agent.notifier.shown


async def test_claude_finished_threshold(agent):
    agent.cfg["claude_done_min_s"] = 120
    assert await agent.claude_finished(30, "/Users/me/loom", "fix it") is None
    ev = await agent.claude_finished(245, "/Users/me/loom/", "fix the build please")
    assert ev["title"] == "Claude finished in loom" and ev["sub"].startswith("took 4 min - fix the build")
