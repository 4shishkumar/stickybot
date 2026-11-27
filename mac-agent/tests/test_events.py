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
    assert ev["level"] == "celebrate" and ev["icon"] == "merge" and ev["id"].startswith("e")
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


def detail(ev, action, **fields):
    return {"src": "agent", "evt": "event_action", "id": ev["id"], "action": action, **fields}


async def test_detail_open_and_read_use_only_delivered_values(agent, monkeypatch):
    import asyncio
    from unittest.mock import AsyncMock, Mock

    opened = AsyncMock(return_value=Mock(wait=AsyncMock()))
    spoken = AsyncMock()
    monkeypatch.setattr(events.asyncio, "create_subprocess_exec", opened)
    monkeypatch.setattr(agent, "speak", spoken)
    assert agent._hello_extra()["eventActions"] == 1
    url = "https://example.org/a?x=1&y=2"
    ev = await agent.notifier.notify("test", "Review", "project #9", actor="sender", url=url, body="Update deps")
    for msg in (detail(ev, "open", url="https://example.org/b"), detail(ev, "open"), detail(ev, "read", text="Forged"),
                detail({"id": "absent"}, "open", url=url), detail(ev, "open", url=url, extra=1)):
        agent._on_message(msg)
    await asyncio.sleep(0)
    opened.assert_not_called()
    spoken.assert_not_called()
    for _ in range(2):
        agent._on_message(detail(ev, "open", url=url))
        agent._on_message(detail(ev, "read"))
    await asyncio.sleep(0)
    opened.assert_awaited_once_with("/usr/bin/open", url)
    spoken.assert_awaited_once_with("Review. sender. project #9. Update deps", force=True)


@pytest.mark.parametrize("url", ["file:///tmp/x", "https:///nohost", "https://u:p@example.org",
                                  "https://example.org/bad\nline", "http://example.org:bad/p",
                                  "https://a.test/" + "z" * 200, "https://a.test\\other.test", "https://[::1]oops"])
async def test_invalid_notification_link_is_omitted_and_display_text_is_bounded(agent, url):
    ev = await agent.notifier.notify("test", "Review", actor="a" * 30, body="b\n" * 250, url=url)
    assert len(ev["actor"]) == 24 and len(ev["body"]) == 300 and "\n" not in ev["body"]
    assert "url" not in ev
    assert agent.notifier.act(detail(ev, "open", url=url)) is None


async def test_unsent_expired_evicted_and_reconnected_events_cannot_act(agent, monkeypatch):
    from types import SimpleNamespace
    from unittest.mock import AsyncMock

    url = "https://example.org/pr/9"
    ev = await agent.notifier.notify("test", "Review", url=url)
    agent.link.client = SimpleNamespace(address=agent.link.client.address)
    assert agent.notifier.act(detail(ev, "open", url=url)) is None
    latest = await agent.notifier.notify("test", "New review", url=url)
    agent.link.paused = True
    assert agent.notifier.act(detail(latest, "read")) is None
    agent.link.paused = False
    first = await agent.notifier.notify("test", "Review", url=url)
    for _ in range(events.DETAIL_LIMIT):
        last = await agent.notifier.notify("test", "Review", url=url)
    assert agent.notifier.act(detail(first, "open", url=url)) is None
    monkeypatch.setattr(events, "DETAIL_TTL_S", 0)
    expired = await agent.notifier.notify("test", "Expired", url=url)
    assert agent.notifier.act(detail(expired, "read")) is None
    monkeypatch.setattr(agent.link, "send", AsyncMock(return_value=False))
    failed = await agent.notifier.notify("test", "No delivery", url=url)
    assert agent.notifier.act(detail(failed, "read")) is None
    agent.link.target = {"address": "other"}
    assert agent.notifier.act(detail(last, "read")) is None


async def test_peer_switch_before_async_open_cancels_it(agent, monkeypatch):
    from types import SimpleNamespace
    from unittest.mock import AsyncMock

    url = "https://example.org/pr/9"
    ev = await agent.notifier.notify("test", "Review", url=url)
    opened = AsyncMock()
    monkeypatch.setattr(events.asyncio, "create_subprocess_exec", opened)
    task = agent.notifier.act(detail(ev, "open", url=url))
    agent.link.client = SimpleNamespace(address=agent.link.client.address)
    await task
    opened.assert_not_awaited()


async def test_maximum_detail_notification_fits_firmware_rx_frame_and_preserves_link(agent):
    from types import SimpleNamespace

    from stick_agent import link

    encoded = bytearray()

    async def receive(_characteristic, data, response):
        encoded.extend(data)

    transport = link.Link(None, None, None)
    transport.client = SimpleNamespace(address="test", is_connected=True, write_gatt_char=receive)
    transport.target = {"address": "test"}
    agent.link = transport
    url = "https://example.org/" + "😀" * 180
    ev = await agent.notifier.notify("test", "—" * 40, "—" * 90, actor="—" * 24, body="—" * 300, url=url)
    assert len(encoded) < 4096
    assert json.loads(encoded)["event"]["url"] == ev["url"] == url


async def test_event_speech_cannot_move_to_another_stick_or_fall_back_to_mac(agent, monkeypatch):
    from types import SimpleNamespace
    from unittest.mock import AsyncMock

    from stick_agent import link

    source = agent.link.client
    with link.pin_peer(source, agent.link.target):
        agent.link.client = SimpleNamespace(address=source.address)
        play_mac = AsyncMock()
        send_audio = AsyncMock()
        monkeypatch.setattr(agent.speaker, "_play_mac", play_mac)
        monkeypatch.setattr(agent.link, "send_audio", send_audio)
        for connected in (True, False):
            agent.link.connected = connected
            await agent.speaker.play([{"text": "review", "pcm": []}], set())
        play_mac.assert_not_awaited()
        send_audio.assert_not_awaited()
