"""The <stick>-bt-mic device: created around an A hold, removed after it (fake CoreAudio)."""
import asyncio
import os

import pytest
from conftest import BLACKHOLE, REAL_CORE_AUDIO, FakeCoreAudio

from stick_agent import config, stick_mic_device
from stick_agent.stick_mic_device import Holds, StickMicDevice

UID = stick_mic_device.UID


@pytest.fixture(autouse=True)
def fast(monkeypatch):
    monkeypatch.setattr(stick_mic_device, "POLL_S", 0.005)
    monkeypatch.setattr(stick_mic_device, "WAIT_READER_S", 0.2)


# --- the device ---------------------------------------------------------------------------

def test_creates_a_public_aggregate_on_blackhole(no_core_audio):
    d = StickMicDevice(own_pid=1)
    assert d.ensure_present() == "created"
    assert no_core_audio.created == [("stickybot-bt-mic", UID, "BlackHole2ch_UID")]
    assert d.present and d.create_ms is not None
    assert d.ensure_present() == "present" and len(no_core_audio.created) == 1  # not twice


def test_remove_destroys_it(no_core_audio):
    d = StickMicDevice(own_pid=1)
    d.ensure_present()
    assert d.remove() == 1
    assert not d.present and no_core_audio.ours() == [] and d.destroy_ms is not None
    assert d.remove() == 0  # nothing left: fine


def test_stale_ones_are_removed(no_core_audio):
    no_core_audio.list[250] = ("stickybot-bt-mic", UID)  # left by an agent that was killed
    no_core_audio.list[251] = ("stickybot-bt-mic", UID)
    no_core_audio.list[252] = ("Someone else's aggregate", "com.other.agg")
    d = StickMicDevice(own_pid=1)
    assert d.cleanup_stale() == 2
    assert no_core_audio.ours() == [] and 252 in no_core_audio.list


def test_an_existing_one_is_reused_after_a_restart(no_core_audio):
    no_core_audio.list[250] = ("stickybot-bt-mic", UID)
    d = StickMicDevice(own_pid=1)
    assert d.ensure_present() == "present" and d.dev == 250 and no_core_audio.created == []


def test_renamed_it_is_made_again(no_core_audio):
    d = StickMicDevice(own_pid=1)
    d.ensure_present()
    d.name = "stick-mic-bt"
    assert d.ensure_present() == "created"
    assert [no_core_audio.list[x][0] for x in no_core_audio.ours()] == ["stick-mic-bt"]


def test_without_blackhole_it_says_how_to_install_it(no_core_audio):
    del no_core_audio.list[120]
    d = StickMicDevice(own_pid=1)
    assert d.ensure_present() == "" and "brew install --cask blackhole-2ch" in d.error
    assert no_core_audio.created == []


def test_a_coreaudio_error_is_reported(no_core_audio):
    no_core_audio.fail_create = -50
    d = StickMicDevice(own_pid=1)
    assert d.ensure_present() == "" and "-50" in d.error and not d.present


def test_coreaudio_missing_doesnt_crash():
    def broken():
        raise stick_mic_device.Unavailable("no CoreAudio here")
    d = StickMicDevice(backend=broken)
    assert d.ensure_present() == "" and "no CoreAudio" in d.error
    assert d.remove() == 0


def test_recording_sees_wispr_on_the_sub_device(no_core_audio):
    d = StickMicDevice(own_pid=1)
    d.ensure_present()
    assert d.recording() is False
    no_core_audio.readers[120] = [1]  # our own output stream: not a reader
    assert d.recording() is False
    no_core_audio.readers[120] = [1, 4242]  # macOS reports the aggregate's reader on BlackHole
    assert d.recording() is True


def test_recording_is_unknown_when_blackhole_already_had_a_reader(no_core_audio):
    no_core_audio.readers[120] = [777]  # e.g. Wispr still set to BlackHole itself
    d = StickMicDevice(own_pid=1)
    d.ensure_present()
    assert d.recording() is None
    no_core_audio.per_process = False  # macOS before 14.2
    d.remove()
    d.ensure_present()
    assert d.recording() is None


# --- the hold flow ------------------------------------------------------------------------

class Sent(list):
    async def __call__(self, msg):
        self.append(msg)
        return True


def holds(lead=20, tail=40, allowed=lambda: ""):
    sent = Sent()
    h = Holds(StickMicDevice(own_pid=1), send=sent, allowed=allowed, settings=lambda: (lead, tail))
    return h, sent


async def wait_for(cond, secs=2.0):
    for _ in range(int(secs / 0.005)):
        if cond():
            return True
        await asyncio.sleep(0.005)
    return cond()


async def test_prepare_creates_waits_the_lead_then_says_ready(no_core_audio):
    h, sent = holds(lead=60)
    t = asyncio.get_running_loop().time()
    assert await h.prepare(7)
    took = asyncio.get_running_loop().time() - t
    assert sent == [{"src": "agent", "wispr": "ready", "id": 7}]
    assert took >= 0.06 and no_core_audio.ours()
    assert h.last is None and h._cur["lead_ms"] == 60 and h._cur["made"] == "created"
    await h.drop("test")


async def test_device_stays_for_the_tail_then_goes(no_core_audio):
    h, sent = holds(lead=0, tail=80)
    await h.prepare(1)
    h.stop()
    await asyncio.sleep(0.03)
    assert no_core_audio.ours()  # still there: Wispr is finishing
    assert await wait_for(lambda: not no_core_audio.ours())
    assert await wait_for(lambda: h.last is not None)
    assert h.last["kept_ms"] >= 80


async def test_a_hold_during_the_tail_keeps_it_and_is_ready_at_once(no_core_audio):
    h, sent = holds(lead=200, tail=60)
    await h.prepare(1)
    h.stop()
    await asyncio.sleep(0.02)
    t = asyncio.get_running_loop().time()
    await h.prepare(2)  # already there: Wispr knows it, no lead time
    assert asyncio.get_running_loop().time() - t < 0.1
    assert sent[-1] == {"src": "agent", "wispr": "ready", "id": 2}
    await asyncio.sleep(0.1)
    assert no_core_audio.ours() and len(no_core_audio.created) == 1  # held: never removed in between
    h.stop()
    assert await wait_for(lambda: not no_core_audio.ours())
    assert no_core_audio.destroyed and len(no_core_audio.created) == 1


async def test_removal_waits_while_wispr_still_reads_it(no_core_audio):
    h, sent = holds(lead=0, tail=20)
    await h.prepare(1)
    no_core_audio.readers[120] = [4242]  # Wispr dictating
    await asyncio.sleep(0.03)
    h.stop()
    await asyncio.sleep(0.08)
    assert no_core_audio.ours()  # tail over, Wispr still reading: not "MicDisconnected"
    no_core_audio.readers[120] = []
    assert await wait_for(lambda: not no_core_audio.ours())
    assert await wait_for(lambda: h.last is not None)
    assert h.last["wispr_open_ms"] is not None and h.last["wispr_closed_ms"] is not None


async def test_removal_gives_up_waiting_eventually(no_core_audio):
    h, sent = holds(lead=0, tail=0)
    await h.prepare(1)
    no_core_audio.readers[120] = [4242]  # a reader that never stops
    h.stop()
    assert await wait_for(lambda: not no_core_audio.ours(), secs=1.0)


async def test_released_before_ready_sends_nothing_and_still_removes_it(no_core_audio):
    h, sent = holds(lead=80, tail=20)
    task = asyncio.ensure_future(h.prepare(3))
    await asyncio.sleep(0.02)
    h.stop()  # the stick cancelled: A released before ready
    await task
    assert sent == []
    assert await wait_for(lambda: not no_core_audio.ours())
    assert h.last["released_early"] is True


async def test_a_stale_ready_is_not_sent_for_a_newer_hold(no_core_audio):
    h, sent = holds(lead=80, tail=500)
    first = asyncio.ensure_future(h.prepare(1))
    await asyncio.sleep(0.01)
    h.stop()
    await h.prepare(2)  # created by the first: no lead
    await first
    assert sent == [{"src": "agent", "wispr": "ready", "id": 2}]
    await h.drop("test")


async def test_not_allowed_answers_at_once_without_a_device(no_core_audio):
    h, sent = holds(allowed=lambda: "the stick is plugged in by USB")
    assert not await h.prepare(5)
    assert sent == [{"src": "agent", "wispr": "ready", "id": 5, "device": False}]
    assert no_core_audio.created == []


async def test_no_blackhole_answers_ready_without_a_device(no_core_audio):
    del no_core_audio.list[120]
    h, sent = holds()
    assert not await h.prepare(5)
    assert sent == [{"src": "agent", "wispr": "ready", "id": 5, "device": False}]
    assert h.last["made"] == "failed"


async def test_show_keeps_it_for_the_ranking_step(no_core_audio):
    h, sent = holds(tail=0)
    assert await h.show(0.15) == ""
    assert h.status()["present"] and no_core_audio.ours()
    await asyncio.sleep(0.05)
    assert no_core_audio.ours()
    assert await wait_for(lambda: not no_core_audio.ours())
    assert h.status()["showing_s"] == 0


async def test_show_refused_when_not_allowed(no_core_audio):
    h, _ = holds(allowed=lambda: "the stick isn't connected by Bluetooth")
    assert "connected" in await h.show(60)
    assert no_core_audio.created == []


async def test_drop_removes_it_now(no_core_audio):
    h, _ = holds(tail=5000)
    await h.prepare(1)
    h.stop()
    await h.drop("the stick is on USB")
    assert no_core_audio.ours() == [] and not h.status()["present"]


def test_shutdown_is_synchronous_and_removes_leftovers(no_core_audio):
    no_core_audio.list[250] = ("stickybot-bt-mic", UID)
    h, _ = holds()
    h.shutdown()
    assert no_core_audio.ours() == []


# --- in the agent -------------------------------------------------------------------------

@pytest.fixture
async def wired(agent, no_real_audio):
    no_real_audio.devices = [BLACKHOLE]
    agent.update_config({"wireless_mic": True, "wispr_switch_ms": 10, "wispr_tail_ms": 30})
    agent._on_link("connected")
    return agent


def readies(agent):
    return [m for m in agent.link.sent if m.get("wispr") == "ready"]


async def test_config_defaults(home):
    cfg = config.load()
    assert cfg["wispr_mic_name"] == ""  # named after the stick
    assert cfg["wispr_switch_ms"] == 250 and cfg["wispr_tail_ms"] == 800
    assert "wispr_fallback_mic" not in cfg


async def test_old_fallback_setting_is_dropped(home):
    config.save({**config.load(), "wispr_fallback_mic": "default"})
    assert "wispr_fallback_mic" not in config.load()


async def test_hello_says_the_agent_answers_prepare(wired):
    extra = wired._hello_extra()
    assert extra["wmic"] is True and extra["wready"] is True


async def test_prepare_stream_stop_through_the_agent(wired, no_core_audio, no_real_audio):
    wired._on_message({"src": "agent", "evt": "wispr", "state": "prepare", "id": 3, "rate": 16000,
                       "codec": "ima-adpcm"})
    assert wired.wispr.active and no_real_audio.streams[0].started  # output open before "ready"
    assert await wait_for(lambda: readies(wired))
    assert readies(wired) == [{"src": "agent", "wispr": "ready", "id": 3}]
    assert no_core_audio.ours()
    wired._on_message({"src": "agent", "evt": "wispr", "state": "stop"})
    assert not wired.wispr.active
    assert await wait_for(lambda: not no_core_audio.ours())
    st = wired.status()["wireless_mic"]["device_mic"]
    assert st["name"] == "stickybot-bt-mic" and not st["present"]


async def test_old_firmware_start_makes_no_device(wired, no_core_audio):
    wired._on_message({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000})
    await asyncio.sleep(0.05)
    assert wired.wispr.active and no_core_audio.created == [] and readies(wired) == []


async def test_stick_on_usb_refuses_and_removes(wired, no_core_audio):
    wired._on_message({"src": "agent", "evt": "wispr", "state": "prepare", "id": 1})
    assert await wait_for(lambda: readies(wired)) and no_core_audio.ours()
    wired._on_message({"src": "agent", "evt": "usb", "mounted": True})
    assert await wait_for(lambda: not no_core_audio.ours())
    assert wired.mic_device_unavailable().startswith("the stick is plugged in by USB")
    wired._on_message({"src": "agent", "evt": "wispr", "state": "prepare", "id": 2})
    assert await wait_for(lambda: len(readies(wired)) == 2)
    assert readies(wired)[-1] == {"src": "agent", "wispr": "ready", "id": 2, "device": False}
    assert no_core_audio.ours() == []
    wired._on_message({"src": "agent", "evt": "usb", "mounted": False})
    assert wired.mic_device_unavailable() == ""


async def test_disconnect_removes_it_at_once(wired, no_core_audio):
    wired.update_config({"wispr_tail_ms": 5000})
    wired._on_message({"src": "agent", "evt": "wispr", "state": "prepare", "id": 1})
    assert await wait_for(lambda: readies(wired))
    wired._on_message({"src": "agent", "evt": "wispr", "state": "stop"})
    wired.link.connected = False
    wired._on_link("searching")
    assert await wait_for(lambda: not no_core_audio.ours())
    assert wired.stick_usb is None


async def test_turning_it_off_removes_it(wired, no_core_audio):
    await wired.holds.show(60)
    wired.update_config({"wireless_mic": False})
    assert await wait_for(lambda: not no_core_audio.ours())


async def test_rename_from_the_settings(wired, no_core_audio):
    await wired.holds.show(60)
    wired.update_config({"wispr_mic_name": "stick-mic-bt"})
    assert await wait_for(lambda: not no_core_audio.ours())
    await wired.holds.show(60)
    assert [no_core_audio.list[d][0] for d in no_core_audio.ours()] == ["stick-mic-bt"]
    await wired.holds.drop("test")


def test_names_follow_the_stick():
    from stick_agent import stick_mic_device as m
    assert m.slug("Arbaaz StickS3") == "arbaaz-sticks3"
    assert m.slug("  stick-key ") == "stick-key" and m.slug("Ana's  key!") == "ana-s-key" and m.slug("") == ""
    assert m.name_for("StickyBot") == "stickybot-bt-mic"  # a default-named stick
    assert m.name_for("stick-key") == "stick-key-bt-mic"
    assert m.name_for("Arbaaz StickS3") == "arbaaz-sticks3-bt-mic"
    assert m.name_for(None) == m.name_for("!!") == "stickybot-bt-mic"  # no stick picked yet


@pytest.mark.parametrize("saved", ["stick-mic-bluetooth", "arbaaz-sticks3-mic-bluetooth", "stick-key-mic-bluetooth",
                                   "  desk-mic-bluetooth "])
def test_old_automatic_names_mean_automatic(saved):
    from stick_agent import stick_mic_device as m
    assert m.is_auto_name(saved)


@pytest.mark.parametrize("typed", ["my mic", "Desk Mic-bluetooth", "stickybot-bt-mic", "wispr mic bluetooth", ""])
def test_typed_names_are_not_automatic(typed):
    from stick_agent import stick_mic_device as m
    assert not m.is_auto_name(typed)


async def test_the_mic_is_named_after_the_picked_stick(wired, no_core_audio):
    wired.cfg["stick"] = {"address": "X", "name": "Arbaaz StickS3"}
    for old in ("stick-mic-bluetooth", "arbaaz-sticks3-mic-bluetooth", "stick-key-mic-bluetooth"):
        wired.cfg["wispr_mic_name"] = old  # an older automatic name, saved in older configs: automatic
        assert wired._mic_name() == "arbaaz-sticks3-bt-mic"
    wired.cfg["wispr_mic_name"] = "my mic"
    assert wired._mic_name() == "my mic"  # typed in: it wins
    wired.cfg["stick"] = {"address": "X", "name": "StickyBot"}
    wired.cfg["wispr_mic_name"] = ""
    assert wired._mic_name() == "stickybot-bt-mic"


async def test_a_renamed_stick_renames_its_mic(wired, no_core_audio):
    wired.cfg["stick"] = {"address": "X", "name": "stick-key"}
    wired.mic_dev.name = wired._mic_name()
    assert {"src": "agent", "proto": "@state", "id": "name"} in wired.link.sent  # asked on connect
    await wired.holds.show(60)
    wired._on_message({"id": "name", "ok": True, "board": "m5-sticks3", "name": "Desk Stick", "fw": "1.8.1"})
    assert wired.cfg["stick"] == {"address": "X", "name": "Desk Stick"}
    assert config.load()["stick"]["name"] == "Desk Stick"
    assert await wait_for(lambda: not no_core_audio.ours())  # the old one goes
    await wired.holds.show(60)
    assert [no_core_audio.list[d][0] for d in no_core_audio.ours()] == ["desk-stick-bt-mic"]
    await wired.holds.drop("test")
    wired._on_message({"src": "agent", "evt": "wispr", "name": "ignored"})  # not a @state reply
    assert wired.cfg["stick"]["name"] == "Desk Stick"


async def test_bad_timings_in_the_config_are_clamped(wired):
    wired.cfg.update({"wispr_switch_ms": "x", "wispr_tail_ms": 99999})
    assert wired._hold_settings() == (250, 5000)


async def test_shutdown_removes_it(wired, no_core_audio):
    await wired.holds.show(60)
    wired.shutdown()
    assert no_core_audio.ours() == []


# --- the dashboard API ----------------------------------------------------------------------

@pytest.fixture
async def client(aiohttp_client, wired):
    from stick_agent import server
    c = await aiohttp_client(server.make_app(wired), headers={"Host": f"localhost:{config.PORT}"})
    c.auth = {"X-Stick-Token": config.token()}
    return c


async def test_show_from_the_dashboard(client, wired, no_core_audio):
    r = await client.post("/api/wireless-mic/show", json={"secs": 60}, headers=client.auth)
    assert r.status == 200 and (await r.json())["ok"]
    assert no_core_audio.ours()
    st = (await (await client.get("/api/status")).json())["wireless_mic"]
    assert st["device_mic"]["present"] and st["device_mic"]["showing_s"] > 50 and st["unavailable"] == ""
    await wired.holds.drop("test")


async def test_show_refused_while_on_usb(client, wired, no_core_audio):
    wired._on_message({"src": "agent", "evt": "usb", "mounted": True})
    r = await client.post("/api/wireless-mic/show", json={}, headers=client.auth)
    assert r.status == 409 and "USB" in (await r.json())["error"]
    assert no_core_audio.created == []
    assert "USB" in (await (await client.get("/api/status")).json())["wireless_mic"]["unavailable"]


async def test_show_needs_the_token(client):
    r = await client.post("/api/wireless-mic/show", json={})
    assert r.status == 403


async def test_timings_can_be_set_from_the_dashboard(client, wired):
    r = await client.put("/api/config", json={"wispr_switch_ms": 400, "wispr_tail_ms": 1200}, headers=client.auth)
    assert r.status == 200 and wired._hold_settings() == (400, 1200)


# --- the real thing (opt-in) ------------------------------------------------------------------

@pytest.mark.coreaudio
@pytest.mark.skipif(os.environ.get("STICK_MIC_E2E") != "1", reason="creates a real CoreAudio device")
def test_real_coreaudio_create_and_destroy():
    real = REAL_CORE_AUDIO()
    d = StickMicDevice(backend=real, name="stick-mic-bluetooth-test", uid="com.stickkey.wireless-mic.test")
    try:
        d.cleanup_stale()
        assert d.ensure_present() == "created", d.error
        assert [n for _i, n, u in real.devices() if u == d.uid] == ["stick-mic-bluetooth-test"]
    finally:
        d.remove()
    assert not [1 for _i, _n, u in real.devices() if u == d.uid]


def test_fake_is_in_place():
    """Guard: the autouse fixture really replaces CoreAudio."""
    assert isinstance(StickMicDevice()._ca(), FakeCoreAudio)
