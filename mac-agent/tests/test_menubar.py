import pytest

rumps = pytest.importorskip("rumps")  # macOS only

from stick_agent import menubar  # noqa: E402


def test_check_result_says_what_the_check_found():
    assert menubar.check_result({"available": True, "title": "Update available: v1.9.0"}) == \
        "Update available: v1.9.0 — see the item above"
    assert menubar.check_result({"error": "HTTP 403", "current": {"agent": "1.8.1"}}) == \
        "Couldn't check for updates: HTTP 403"
    assert menubar.check_result({"current": {"agent": "1.8.1"}}) == "Up to date (v1.8.1)"


def test_volume_and_mute_keep_the_sticks_level():
    from types import SimpleNamespace

    sent = []
    agent = SimpleNamespace(link=SimpleNamespace(connected=True), sounds={"volume": 94, "on": True})
    menu = SimpleNamespace(agent=agent, _stick=sent.append)
    menubar.Menu.set_volume(menu, SimpleNamespace(value=37))
    menubar.Menu.set_volume(menu, SimpleNamespace(value=0))
    menubar.Menu.toggle_sounds(menu, None)
    agent.sounds["on"] = False
    menubar.Menu.toggle_sounds(menu, None)
    agent.sounds["volume"] = 0
    menubar.Menu.toggle_sounds(menu, None)
    assert sent == [{"volume": 94, "on": True}, {"on": False}, {"on": False, "volume": 94},
                    {"on": True, "volume": 94}, {"on": True, "volume": 140}]
