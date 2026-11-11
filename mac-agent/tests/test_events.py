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
    ev = await agent.notifier.notify("pr_merged", "PR #12 merged!", "web - Café ✨ fix")
    assert ev["level"] == "celebrate" and ev["icon"] == "merge" and ev["id"] == "e1"
    msg = agent.link.sent[-1]
    assert msg == {"src": "agent", "event": ev}
    assert ev["sub"] == "web - Cafe  fix"          # sanitised for the stick's fonts
    logged = [json.loads(line) for line in events.EVENTS_LOG.read_text().splitlines()]
    assert logged[-1]["kind"] == "pr_merged" and logged[-1]["sent"] is True
    assert any(e["kind"] == "event" for e in agent.events)
    assert events.history()[-1]["title"] == "PR #12 merged!"


async def test_notify_off_sends_nothing(agent):
    agent.cfg["event_rules"] = {"release": "off"}
    assert await agent.notifier.notify("release", "Release v1", "web") is None
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
    assert await agent.claude_finished(30, "/Users/me/web", "fix it") is None
    ev = await agent.claude_finished(245, "/Users/me/web/", "fix the build please")
    assert ev["title"] == "Claude finished in web" and ev["sub"].startswith("took 4 min - fix the build")


# --- a sound per event kind (firmware 2.7.0's celebration clips) ---------------------------

STICK_TUNES = {"tunes": ["jingle_win", "confirmation_002", "question_001", "applause", "cheer", "party_horn",
                         "tada", "ping-3"]}


def test_sounds_defaults_and_overrides():
    s = events.sounds({})
    assert (s["pr_merged"], s["release"], s["ci_fixed"]) == ("applause", "party_horn", "cheer")
    assert s["ci_failed"] == "" and s["claude_done"] == "" and set(s) == set(events.KINDS)
    s = events.sounds({"event_sounds": {"pr_merged": "off", "ci_failed": "tada", "release": "",
                                        "nope": "tada", "claude_done": "bad name!", "pr_created": 3}})
    assert (s["pr_merged"], s["ci_failed"], s["release"]) == ("off", "tada", "")
    assert s["claude_done"] == "" and s["pr_created"] == "" and "nope" not in s


@pytest.mark.parametrize("body, tunes, saved, bad", [
    ({}, None, {}, ""),
    ({"event_sounds": {"pr_merged": "tada", "ci_failed": "off", "release": ""}}, None, {}, ""),
    ({"event_sounds": {"pr_merged": "anything_newer"}}, None, {}, ""),   # stick unknown: only the shape
    ({"event_sounds": {"pr_merged": "tada"}}, STICK_TUNES["tunes"], {}, ""),
    ({"event_sounds": {"pr_merged": "coins"}}, STICK_TUNES["tunes"], {}, "the stick has no sound 'coins'"),
    ({"event_sounds": {"pr_merged": "coins"}}, STICK_TUNES["tunes"], {"event_sounds": {"pr_merged": "coins"}}, ""),
    # the page sends every kind: a default an older stick lacks may stay; a new one may not
    ({"event_sounds": {"pr_merged": "applause", "ci_failed": "tada"}}, ["tada"], {}, ""),
    ({"event_sounds": {"ci_failed": "applause"}}, ["tada"], {}, "the stick has no sound 'applause'"),
    ({"event_sounds": ["tada"]}, None, {}, "must map"),
    ({"event_sounds": {"bogus": "tada"}}, None, {}, "no event kind"),
    ({"event_sounds": {"pr_merged": "../x"}}, None, {}, "not a sound name"),
    ({"event_sounds": {"pr_merged": 5}}, None, {}, "not a sound name"),
])
def test_valid_config(body, tunes, saved, bad):
    got = events.valid_config(body, tunes, saved)
    assert (bad in got) if bad else got == ""


async def test_notify_sends_the_kinds_sound(agent, monkeypatch):
    at(monkeypatch, 12)
    assert (await agent.notifier.notify("pr_merged", "PR merged", "web"))["sound"] == "applause"
    assert (await agent.notifier.notify("ci_failed", "CI failed", "web"))["sound"] == ""   # the level's
    agent.cfg["event_sounds"] = {"ci_failed": "tada", "pr_merged": "off"}
    assert (await agent.notifier.notify("ci_failed", "CI failed", "web"))["sound"] == "tada"
    assert (await agent.notifier.notify("pr_merged", "PR merged", "web"))["sound"] == "off"
    # a caller's own sound wins (the stick tools: sound=False -> "off")
    assert (await agent.notifier.notify("ci_failed", "x", sound="off"))["sound"] == "off"


async def test_a_sound_the_stick_lacks_goes_out_as_the_level_default(agent, monkeypatch):
    at(monkeypatch, 12)
    agent.sounds = {"tunes": ["jingle_win", "confirmation_002", "question_001"]}   # firmware before 2.7.0
    assert (await agent.notifier.notify("pr_merged", "PR merged", "web"))["sound"] == ""
    agent.sounds = STICK_TUNES
    assert (await agent.notifier.notify("pr_merged", "PR merged", "web"))["sound"] == "applause"


async def test_quiet_hours_beat_the_kinds_sound(agent, monkeypatch):
    agent.cfg["quiet_hours"] = {"on": True, "from": "19:00", "to": "09:00"}
    at(monkeypatch, 22)
    assert (await agent.notifier.notify("pr_merged", "PR merged", "web"))["sound"] == "off"


async def test_test_uses_the_level_and_sound_picked_on_the_page(agent, monkeypatch):
    at(monkeypatch, 12)
    ev = await agent.notifier.test("ci_failed", "celebrate", "cheer")    # not saved: still what's sent
    assert (ev["level"], ev["sound"], ev["title"]) == ("celebrate", "cheer", "CI failed")
    ev = await agent.notifier.test("pr_merged", "nice", "")             # "" = the level's default
    assert (ev["level"], ev["sound"]) == ("nice", "")
    ev = await agent.notifier.test("pr_merged", "loud", "../x")         # junk: the saved ones
    assert (ev["level"], ev["sound"]) == ("celebrate", "applause")
    assert await agent.notifier.test("pr_merged", "off") is None
