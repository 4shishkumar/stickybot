"""Host actions from a physical button notification on the active Bluetooth link."""
import asyncio
import logging
import subprocess
import unicodedata

from . import commands

log = logging.getLogger("agent")


async def run(agent, msg, source, target):
    link = agent.link

    def current():
        return link.connected and not link.paused and link.client is source and link.target is target

    def run_if_current(func, *args, **kwargs):
        if current():
            return func(*args, **kwargs)
        return None

    gesture, action = msg.get("gesture"), msg.get("action")
    if gesture not in ("b2", "b3", "ab") or not isinstance(action, str) or not current():
        return
    kind, _, name = action.partition(":")
    if (kind not in ("app", "quick") or not name or name != name.strip()
            or any(ch in "/\\" or unicodedata.category(ch) in ("Cc", "Cs") for ch in name)
            or len(name.encode("utf-8")) > 32):
        return
    # A fresh read uses this BLE connection only. A late event cannot follow a remap or a device switch.
    state = await agent.stick_api.ble("@state")
    bindings = state.get("buttons") if isinstance(state, dict) and state.get("ok") is True else None
    if not current() or not isinstance(bindings, dict) or bindings.get(gesture) != action:
        return
    reply = None
    title = "Quick command"
    try:
        if kind == "app":
            app = commands.list_apps().get(name.lower())
            if not app:
                raise ValueError("The mapped Mac app is unavailable")
            argv = ["/usr/bin/open", "-a", app]
            await asyncio.to_thread(run_if_current, subprocess.run, argv, check=True, timeout=10)
        else:
            matches = [cmd for cmd in commands.load() if cmd.get("name") == name]
            if len(matches) != 1:
                raise ValueError("Choose a unique saved quick command")
            cmd = matches[0]
            if "ir" in cmd:
                ok, reply = await agent.ir.voice(cmd["ir"])
                if not ok:
                    raise ValueError(reply)
            else:
                reply = await asyncio.to_thread(run_if_current, commands.run, cmd, {}, quote_shell=True)
                if not reply:
                    raise ValueError("The saved quick command is unavailable")
    except (KeyError, ValueError, OSError, subprocess.SubprocessError) as exc:
        log.info("Button action unavailable: %s", type(exc).__name__)
        title, reply = "Button action unavailable", "Button action unavailable. Check its name and any required arguments."
    if reply and current():
        agent.emit("reply", via="button", text=reply)
        if agent.state == "ready" and not (agent.brain.busy or agent.rec or agent.asks or agent.prompts):
            await link.send({"src": "agent", "event": {"level": "nice", "title": title, "sub": reply[:90],
                                                      "icon": "bell", "sound": "off"}})
