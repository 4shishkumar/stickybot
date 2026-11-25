"""A physical button is scoped to the current stick and one saved, fixed action."""
import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock

import pytest

from stick_agent import buttons, commands, firmware_api, flasher, link


@pytest.fixture
def wired(agent):
    agent.link.client = object()
    agent.link.target = {"address": "stick-a"}
    agent.stick_api = SimpleNamespace(ble=AsyncMock())
    return agent


async def press(agent, action, gesture="b2"):
    agent._on_message({"src": "agent", "evt": "button_action", "gesture": gesture, "action": action})
    await agent.button_task


async def test_physical_app_uses_current_binding_and_exact_app(wired, monkeypatch):
    assert wired._hello_extra()["buttonActions"] is True
    monkeypatch.setattr(commands, "list_apps", lambda: {"notes": "Notes"})
    opened = Mock()
    monkeypatch.setattr(buttons.subprocess, "run", opened)
    wired.stick_api.ble.return_value = {"ok": True, "buttons": {"b2": "app:Notes"}}
    await press(wired, "app:Notes")
    wired.stick_api.ble.assert_awaited_once_with("@state")
    opened.assert_called_once_with(["/usr/bin/open", "-a", "Notes"], check=True, timeout=10)
    await press(wired, "app:Notes", gesture="b3")
    await press(wired, "app:Terminal")
    wired.stick_api.ble.return_value["buttons"]["b2"] = "app:Not"
    await press(wired, "app:Not")
    assert opened.call_count == 1


async def test_reconnect_during_check_drops_action(wired, monkeypatch):
    run = Mock()
    monkeypatch.setattr(commands, "run", run)

    async def changed(_):
        wired.link.client = object()
        return {"ok": True, "buttons": {"b2": "quick:Desk"}}

    wired.stick_api.ble.side_effect = changed
    await press(wired, "quick:Desk")
    run.assert_not_called()


async def test_quick_press_requires_one_exact_saved_command(wired, monkeypatch):
    saved = [{"name": "Desk", "open_app": "Notes", "phrases": ["desk time"]}]
    monkeypatch.setattr(commands, "load", lambda: list(saved))
    runner = Mock(return_value="Opened Notes.")
    monkeypatch.setattr(commands, "run", runner)
    wired.stick_api.ble.return_value = {"ok": True, "buttons": {"b2": "quick:Desk"}}
    await press(wired, "quick:Desk")
    runner.assert_called_once_with(saved[0], {}, quote_shell=True)
    saved.append(dict(saved[0]))
    await press(wired, "quick:Desk")
    wired.stick_api.ble.return_value["buttons"]["b2"] = "quick:desk time"
    await press(wired, "quick:desk time")
    assert runner.call_count == 1


async def test_buttons_save_never_falls_back_to_another_stick_on_usb(wired, monkeypatch):
    stick = firmware_api.Stick(wired)
    monkeypatch.setattr(stick, "ble", AsyncMock(return_value=None))
    monkeypatch.setattr(flasher, "espressif_ports", lambda: pytest.fail("selected stick changed transport"))
    for connected in (True, False):
        wired.link.connected = connected
        assert (await stick.run("@buttons b2=launcher&b3=unlock&ab=reply&bh=talk"))["ok"] is False

async def test_queued_button_write_cannot_switch_bluetooth_client():
    transport = link.Link(None, None, None)
    old = SimpleNamespace(is_connected=True, write_gatt_char=AsyncMock())
    new = SimpleNamespace(is_connected=True, write_gatt_char=AsyncMock())
    transport.client = old
    async with transport._write_lock:
        queued = asyncio.create_task(transport.send({"proto": "@buttons b2=none&b3=none&ab=none&bh=talk"}))
        await asyncio.sleep(0)
        transport.client = new
    assert not await queued
    old.write_gatt_char.assert_not_called()
    new.write_gatt_char.assert_not_called()
