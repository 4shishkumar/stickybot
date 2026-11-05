"""The Apple audio tap carrier for the wireless Wispr mic (fake CoreAudio, fake sounddevice):
picking the carrier, the tapped aggregate, the inaudible check, holds in tap mode, status / API /
setup page. Nothing here touches real audio."""
import asyncio
import sys

import numpy as np
import pytest
from conftest import BLACKHOLE, SPEAKERS

from stick_agent import adpcm, config, install_flow, installer, stick_mic_device
from stick_agent.stick_mic_device import BLACKHOLE as BH
from stick_agent.stick_mic_device import TAP, UID, Holds, StickMicDevice, Unavailable, pick_carrier

REAL_SUPPORTS_TAP = stick_mic_device.macos_supports_tap  # (conftest replaces it for every test)


@pytest.fixture(autouse=True)
def fast(monkeypatch):
    monkeypatch.setattr(stick_mic_device, "POLL_S", 0.005)
    monkeypatch.setattr(stick_mic_device, "WAIT_READER_S", 0.2)


@pytest.fixture
def tap_mac(monkeypatch):
    """A Mac with process taps (macOS 14.2+)."""
    monkeypatch.setattr(stick_mic_device, "macos_supports_tap", lambda: True)


async def wait_for(cond, secs=2.0):
    for _ in range(int(secs / 0.005)):
        if cond():
            return True
        await asyncio.sleep(0.005)
    return cond()


# --- picking the carrier ------------------------------------------------------------------

@pytest.mark.parametrize("pref, supported, state, bh, want", [
    ("auto", True, "ok", True, TAP),          # the tap wins once it's proven
    ("auto", True, "ok", False, TAP),
    ("auto", True, "failed", True, BH),       # not allowed: BlackHole meanwhile
    ("auto", True, None, True, BH),           # not checked yet: BlackHole meanwhile
    ("auto", True, "failed", False, None),
    ("auto", True, None, False, None),
    ("auto", False, None, True, BH),          # older macOS
    ("auto", False, None, False, None),
    ("tap", True, "ok", True, TAP),
    ("tap", True, "failed", True, None),      # asked for the tap: no silent BlackHole
    ("tap", False, None, True, None),
    ("blackhole", True, "ok", True, BH),
    ("blackhole", True, "ok", False, None),
    ("nonsense", True, "ok", False, TAP),     # unknown setting = auto
])
def test_carrier_matrix(pref, supported, state, bh, want):
    carrier, why = pick_carrier(pref, supported, state, bh)
    assert carrier == want
    assert bool(why) == (want is None)


def test_why_not_says_what_to_do():
    assert "System Audio Recording" in pick_carrier("auto", True, "failed", False)[1]
    assert "checking" in pick_carrier("auto", True, "checking", False)[1]
    assert "14.2" in pick_carrier("tap", False, None, True)[1]
    assert "brew install" in pick_carrier("blackhole", True, "ok", False)[1]
    assert "BlackHole" in pick_carrier("auto", True, "error", False)[1]


def test_macos_version_gate(monkeypatch):
    for v, ok in (("14.1.2", False), ("14.2", True), ("15.0", True), ("27.2", True), ("13.6", False), ("", False)):
        monkeypatch.setattr(stick_mic_device.platform, "mac_ver", lambda v=v: (v, ("", "", ""), ""))
        assert REAL_SUPPORTS_TAP() is ok, v


# --- the tapped device -----------------------------------------------------------------------

def tapped(**kw):
    return StickMicDevice(own_pid=4242, mode=TAP, **kw)


def test_tap_mode_makes_a_public_aggregate_with_our_tap(no_core_audio):
    d = tapped()
    assert d.ensure_present() == "created"
    (tap_id, (uuid, name, process)), = no_core_audio.tap_list.items()
    assert name == UID + ".tap" and process == 77  # the agent's own process object
    assert no_core_audio.created == [("stickybot-bt-mic", UID, "BuiltInSpeaker", uuid)]
    desc = no_core_audio.aggregates[d.dev]
    assert desc["private"] == 0 and desc["master"] == "BuiltInSpeaker"
    assert desc["taps"] == [{"uid": uuid, "drift": 1}] and desc["tapautostart"] == 1
    assert d.tap == tap_id and d.tap_live
    assert d.ensure_present() == "present" and len(no_core_audio.created) == 1


def test_removing_it_destroys_the_tap_too(no_core_audio):
    d = tapped()
    d.ensure_present()
    tap = d.tap
    assert d.remove() == 1
    assert no_core_audio.ours() == [] and no_core_audio.tap_list == {}
    assert ("destroy", tap) in no_core_audio.tap_calls and not d.tap_live and d.tap is None


def test_a_leftover_aggregate_without_our_tap_is_made_again(no_core_audio):
    no_core_audio.list[250] = ("stickybot-bt-mic", UID)  # from a crashed agent: its tap is gone
    d = tapped()
    assert d.ensure_present() == "created" and 250 not in no_core_audio.list and d.tap_live


def test_switching_carrier_makes_it_again(no_core_audio):
    d = StickMicDevice(own_pid=1)
    assert d.ensure_present() == "created" and not d.tap_live
    d.mode = TAP
    assert d.ensure_present() == "created" and d.tap_live
    assert len(no_core_audio.ours()) == 1


def test_tap_failure_makes_no_device(no_core_audio):
    no_core_audio.fail_tap = -50
    d = tapped()
    assert d.ensure_present() == "" and "tap" in d.error and no_core_audio.created == []


def test_an_objc_refusal_doesnt_crash(no_core_audio):
    no_core_audio.tap_error = Unavailable("this macOS doesn't understand setMuteBehavior:")
    d = tapped()
    assert d.ensure_present() == "" and "setMuteBehavior" in d.error and not d.present


def test_aggregate_failure_removes_the_tap(no_core_audio):
    no_core_audio.fail_create = -1
    d = tapped()
    assert d.ensure_present() == "" and no_core_audio.tap_list == {} and d.tap is None


def test_no_default_output(no_core_audio):
    no_core_audio.output_uid = ""
    d = tapped()
    assert d.ensure_present() == "" and "output" in d.error


def test_stale_taps_are_removed_by_name(no_core_audio):
    no_core_audio.tap_list = {60: ("A", UID + ".tap", 5), 61: ("B", UID + ".check.tap", 5),
                              62: ("C", "someone else's tap", 5)}
    no_core_audio.list[251] = ("stickybot-bt-mic-check", UID + ".check")  # an interrupted check
    StickMicDevice().cleanup_stale()
    assert list(no_core_audio.tap_list) == [62] and 251 not in no_core_audio.list


# --- the check --------------------------------------------------------------------------------

def test_check_hears_the_tone(no_core_audio):
    d = tapped()
    res = d.probe()
    assert res["state"] == "ok" and res["first_ms"] == 70.0 and res["create_ms"] is not None
    # a test-named device, never the real name / UID, and everything removed afterwards
    (_n, uid, _sub, _tap), = no_core_audio.created
    assert uid == UID + ".check" and no_core_audio.created[0][0] == "stickybot-bt-mic-check"
    assert no_core_audio.ours() == [] and no_core_audio.ours(UID + ".check") == [] and no_core_audio.tap_list == {}
    (_dev, secs, amp), = no_core_audio.measured
    assert amp <= 1e-4 and secs <= 2  # inaudible, short
    assert d.check is res and not d.present


def test_check_hears_nothing_without_the_permission(no_core_audio):
    no_core_audio.tap_hears = False
    res = tapped().probe()
    assert res["state"] == "failed" and "System Audio Recording" in res["detail"]
    assert no_core_audio.tap_list == {} and no_core_audio.ours(UID + ".check") == []


def test_check_error_is_reported(no_core_audio):
    no_core_audio.tap_error = Unavailable("this macOS has no process taps (14.2+)")
    res = tapped().probe()
    assert res["state"] == "error" and "14.2" in res["detail"]


def test_a_quiet_signal_doesnt_count(no_core_audio, monkeypatch):
    monkeypatch.setattr(no_core_audio, "measure_input",
                        lambda dev, secs, amp, freq=440.0: {"callbacks": 9, "rms": amp * 0.01, "first_ms": 50.0})
    assert tapped().probe()["state"] == "failed"


class Sent(list):
    async def __call__(self, msg):
        self.append(msg)
        return True


async def test_no_check_during_a_hold(no_core_audio):
    h = Holds(tapped(), send=Sent(), settings=lambda: (5, 30))
    await h.prepare(1)
    assert await h.check() is None and no_core_audio.measured == []
    h.stop()
    assert await wait_for(lambda: not h.device.present)
    assert (await h.check())["state"] == "ok"
    assert h.status()["check"]["state"] == "ok" and not h.status()["check"]["running"]


# --- the ObjC guard (real runtime, no CoreAudio) ------------------------------------------------

@pytest.mark.skipif(sys.platform != "darwin", reason="the Objective-C runtime is macOS-only")
def test_unknown_selectors_are_refused_not_sent():
    oc = stick_mic_device._ObjC()
    ns = oc.cls("NSObject")
    assert oc.responds(ns, "alloc") and not oc.responds(ns, "muteBehavior")
    with pytest.raises(Unavailable):
        oc.send(None, ns, "muteBehavior")  # would abort the process if it were sent
    with pytest.raises(Unavailable):
        oc.cls("NoSuchClassHere")


# --- the agent in tap mode ------------------------------------------------------------------------

@pytest.fixture
async def tap_agent(agent, tap_mac, no_real_audio):
    no_real_audio.devices = [SPEAKERS]
    agent.loop = asyncio.get_running_loop()
    agent.cfg.update({"wispr_switch_ms": 10, "wispr_tail_ms": 30})
    return agent


def readies(agent):
    return [m for m in agent.link.sent if m.get("wispr") == "ready"]


async def test_turning_it_on_checks_the_tap_then_tells_the_stick(tap_agent, no_core_audio):
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.wispr.carrier == TAP)
    assert await wait_for(lambda: {"src": "agent", "wmic": True} in tap_agent.link.sent)
    assert no_core_audio.measured and no_core_audio.ours(UID + ".check") == []
    st = tap_agent.status()["wireless_mic"]
    assert st["carrier"] == TAP and st["carrier_title"] == "Apple audio tap" and st["tap"]["state"] == "ok"
    assert st["unavailable"] == "" and st["device"] == "MacBook Pro Speakers" and st["error"] == ""


async def test_a_hold_in_tap_mode(tap_agent, no_core_audio, no_real_audio):
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.wispr.carrier == TAP)
    tap_agent._on_link("connected")
    assert no_real_audio.streams == []  # the real output device: not held open between holds
    tap_agent._on_message({"src": "agent", "evt": "wispr", "state": "prepare", "id": 5, "rate": 16000})
    stream = no_real_audio.streams[-1]
    assert stream.kw["device"] == 0 and stream.started  # the default output, open before "ready"
    assert await wait_for(lambda: readies(tap_agent))
    assert readies(tap_agent) == [{"src": "agent", "wispr": "ready", "id": 5}]
    assert tap_agent.mic_dev.tap_live and no_core_audio.ours() and no_core_audio.tap_list
    tone = (8000 * np.sin(2 * np.pi * 440 * np.arange(16000) / 16000)).astype(np.int16)
    for f in adpcm.encode(tone[:8000].tolist()):
        tap_agent._on_audio(f)
    out = stream.play(48000 // 2)
    assert np.abs(out).max() > 0.05  # the stick's audio goes out: into the tap
    tap_agent._on_message({"src": "agent", "evt": "wispr", "state": "stop"})
    assert await wait_for(lambda: not no_core_audio.ours())
    assert no_core_audio.tap_list == {} and not tap_agent.mic_dev.tap_live
    assert await wait_for(lambda: stream.closed) and tap_agent.wispr.stream is None


async def test_without_the_tap_nothing_reaches_the_speakers(tap_agent, no_core_audio, no_real_audio):
    """The gate: in tap mode audio only goes out while our tap exists (else it's the real speakers)."""
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.wispr.carrier == TAP)
    tap_agent._on_link("connected")
    tap_agent.wispr.start(16000)  # old firmware's "start": no device, no tap
    stream = no_real_audio.streams[-1]
    tone = (8000 * np.sin(2 * np.pi * 440 * np.arange(8000) / 16000)).astype(np.int16)
    for f in adpcm.encode(tone.tolist()):
        tap_agent._on_audio(f)
    assert not tap_agent.mic_dev.tap_live
    assert not stream.play(48000 // 4).any()  # silence, not ones / not the tone
    assert tap_agent.wispr.gated > 0


async def test_tap_not_allowed_and_no_blackhole(tap_agent, no_core_audio):
    no_core_audio.tap_hears = False
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.mic_dev.check.get("state") == "failed")
    assert await wait_for(lambda: {"src": "agent", "wmic": False} in tap_agent.link.sent)
    st = tap_agent.status()["wireless_mic"]
    assert st["carrier"] is None and "System Audio Recording" in st["unavailable"]
    assert tap_agent._hello_extra()["wmic"] is False
    tap_agent._on_message({"src": "agent", "evt": "wispr", "state": "prepare", "id": 1, "rate": 16000})
    assert await wait_for(lambda: readies(tap_agent))
    assert readies(tap_agent)[0]["device"] is False and no_core_audio.ours() == []


async def test_tap_not_allowed_falls_back_to_blackhole(tap_agent, no_core_audio, no_real_audio):
    no_core_audio.tap_hears = False
    no_real_audio.devices = [SPEAKERS, BLACKHOLE]
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.mic_dev.check.get("state") == "failed")
    assert tap_agent.wispr.carrier == BH and tap_agent.mic_dev.mode == BH
    assert tap_agent._hello_extra()["wmic"] is True
    # allowed later: "Check again" switches to the tap
    no_core_audio.tap_hears = True
    assert (await tap_agent.check_tap(force=True))["state"] == "ok"
    assert tap_agent.wispr.carrier == TAP and tap_agent.mic_dev.mode == TAP


async def test_set_to_blackhole_never_checks_the_tap(tap_agent, no_core_audio, no_real_audio):
    no_real_audio.devices = [SPEAKERS, BLACKHOLE]
    tap_agent.update_config({"wireless_mic": True, "wireless_mic_carrier": "blackhole"})
    await asyncio.sleep(0.05)
    assert no_core_audio.measured == [] and tap_agent.wispr.carrier == BH


async def test_switching_carrier_in_the_settings(tap_agent, no_core_audio, no_real_audio):
    no_real_audio.devices = [SPEAKERS, BLACKHOLE]
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.wispr.carrier == TAP)
    tap_agent.update_config({"wireless_mic_carrier": "blackhole"})
    assert tap_agent.wispr.carrier == BH and tap_agent.wispr.device()["name"] == "BlackHole 2ch"
    tap_agent.update_config({"wireless_mic_carrier": "auto"})
    assert tap_agent.wispr.carrier == TAP  # already proven: no new check needed


async def test_older_macos_is_blackhole_as_before(agent, no_core_audio, no_real_audio):
    no_real_audio.devices = [BLACKHOLE]
    agent.update_config({"wireless_mic": True})
    assert agent.wispr.carrier == BH and no_core_audio.measured == []
    assert agent.status()["wireless_mic"]["tap"]["supported"] is False


async def test_config_default_carrier(home):
    assert config.load()["wireless_mic_carrier"] == "auto"


# --- dashboard API ----------------------------------------------------------------------------

@pytest.fixture
async def client(aiohttp_client, tap_agent):
    from stick_agent import server
    c = await aiohttp_client(server.make_app(tap_agent), headers={"Host": f"localhost:{config.PORT}"})
    c.auth = {"X-Stick-Token": config.token()}
    return c


async def test_check_from_the_dashboard(client, tap_agent, no_core_audio):
    tap_agent.update_config({"wireless_mic": True})
    no_core_audio.tap_hears = False
    r = await client.post("/api/wireless-mic/check", json={}, headers=client.auth)
    body = await r.json()
    assert r.status == 200 and body["ok"] is False and body["state"] == "failed"
    assert "System Audio Recording" in body["detail"]
    no_core_audio.tap_hears = True
    body = await (await client.post("/api/wireless-mic/check", json={}, headers=client.auth)).json()
    assert body["ok"] and body["wireless_mic"]["carrier"] == TAP
    st = (await (await client.get("/api/status")).json())["wireless_mic"]
    assert st["tap"]["state"] == "ok" and st["carrier_pref"] == "auto"


async def test_check_needs_the_token(client):
    assert (await client.post("/api/wireless-mic/check", json={})).status == 403


async def test_check_refused_on_older_macos(client, monkeypatch):
    monkeypatch.setattr(stick_mic_device, "macos_supports_tap", lambda: False)
    r = await client.post("/api/wireless-mic/check", json={}, headers=client.auth)
    assert r.status == 409 and "14.2" in (await r.json())["error"]


async def test_show_in_tap_mode_plays_silence_through_the_tap(client, tap_agent, no_core_audio, no_real_audio):
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.wispr.carrier == TAP)
    r = await client.post("/api/wireless-mic/show", json={"secs": 60}, headers=client.auth)
    assert r.status == 200 and (await r.json())["ok"]
    assert tap_agent.mic_dev.tap_live and tap_agent.wispr.stream is not None and no_real_audio.streams[-1].started
    await tap_agent.holds.drop("test")
    assert no_core_audio.tap_list == {} and tap_agent.wispr.stream is None


async def test_show_refused_while_the_tap_isnt_allowed(client, tap_agent, no_core_audio):
    no_core_audio.tap_hears = False
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.mic_dev.check.get("state") == "failed")
    r = await client.post("/api/wireless-mic/show", json={}, headers=client.auth)
    assert r.status == 409 and "System Audio Recording" in (await r.json())["error"]
    assert no_core_audio.ours() == []


# --- the setup page ---------------------------------------------------------------------------

async def test_setup_step_on_a_tap_mac(tap_agent, no_core_audio, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    state, detail, data = install_flow.check_wispr(tap_agent)
    assert state == "todo" and data["tap_mode"] and "no driver" in detail
    assert data["needs"] == {"permission": "System Audio Recording (macOS asks once)"}
    tap_agent.update_config({"wireless_mic": True})
    config.save({**config.load(), "wireless_mic": True})
    assert await wait_for(lambda: tap_agent.wispr.carrier == TAP)
    state, detail, data = install_flow.check_wispr(tap_agent)
    assert state == "done" and "Apple audio tap" in detail


async def test_setup_step_waits_for_the_permission(tap_agent, no_core_audio, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    no_core_audio.tap_hears = False
    config.save({**config.load(), "wireless_mic": True})
    tap_agent.update_config({"wireless_mic": True})
    assert await wait_for(lambda: tap_agent.mic_dev.check.get("state") == "failed")
    state, detail, data = install_flow.check_wispr(tap_agent)
    assert state == "todo" and "System Audio Recording" in detail and data["tap"]["state"] == "failed"


async def test_setup_step_with_blackhole_chosen(tap_agent, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: True)
    monkeypatch.setattr(installer, "blackhole_version", lambda: "0.7.1")
    config.save({**config.load(), "wireless_mic": True, "wireless_mic_carrier": "blackhole"})
    state, _detail, data = install_flow.check_wispr(tap_agent)
    assert state == "done" and not data["tap_mode"] and data["tap_supported"] and "needs" not in data


@pytest.fixture
async def page(aiohttp_client, tap_agent):
    from stick_agent import server
    c = await aiohttp_client(server.make_app(tap_agent), headers={"Host": f"localhost:{config.PORT}"})
    c.auth = {"X-Stick-Token": config.token()}
    return c


async def wait_job(page, job):
    r = await page.get(f"/api/install/jobs/{job}")
    text = await r.text()
    return text


async def test_setup_turn_on_runs_the_check(page, tap_agent, no_core_audio, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    monkeypatch.setattr(installer, "install_blackhole", lambda ui: pytest.fail("no BlackHole on a tap Mac"))
    r = await page.post("/api/install/wispr/run", json={"carrier": "tap"}, headers=page.auth)
    text = await wait_job(page, (await r.json())["job"])
    assert "the Apple audio tap works" in text and '"ok": true' in text
    assert config.load()["wireless_mic"] is True and tap_agent.wispr.carrier == TAP
    step = await (await page.get("/api/install/wispr")).json()
    assert step["state"] == "done" and step["needs"] == {"permission": "System Audio Recording (macOS asks once)"}


async def test_setup_turn_on_reports_the_missing_permission(page, tap_agent, no_core_audio, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    no_core_audio.tap_hears = False
    r = await page.post("/api/install/wispr/run", json={"carrier": "tap"}, headers=page.auth)
    text = await wait_job(page, (await r.json())["job"])
    assert '"ok": false' in text and "System Audio Recording" in text


async def test_setup_use_blackhole_instead(page, tap_agent, monkeypatch):
    calls = []
    monkeypatch.setattr(installer, "install_blackhole", lambda ui: calls.append(1))
    monkeypatch.setattr(installer, "blackhole_installed", lambda: True)
    r = await page.post("/api/install/wispr/run", json={"carrier": "blackhole"}, headers=page.auth)
    await wait_job(page, (await r.json())["job"])
    assert calls == [1] and config.load()["wireless_mic_carrier"] == "blackhole"


async def test_setup_rejects_a_bad_carrier(page):
    r = await page.post("/api/install/wispr/run", json={"carrier": "cable"}, headers=page.auth)
    assert r.status == 400


def test_component_on_a_tap_mac(home, tap_mac, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    c = installer.WirelessMic()
    assert c.needs == {"permission": "System Audio Recording (macOS asks once)"}
    assert c.status()[0] == "missing"

    class UI:
        lines = []

        def say(self, s):
            self.lines.append(s)
    c.install(UI())  # no BlackHole needed: no NotAvailable
    assert config.load()["wireless_mic"] is True and c.status()[0] == "installed"
    assert any("System Audio Recording" in x for x in UI.lines)


def test_component_set_to_blackhole_still_needs_it(home, tap_mac, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    config.save({**config.load(), "wireless_mic_carrier": "blackhole"})
    c = installer.WirelessMic()
    assert c.needs.get("admin")
    with pytest.raises(installer.NotAvailable):
        c.install(type("UI", (), {"say": lambda self, s: None})())
