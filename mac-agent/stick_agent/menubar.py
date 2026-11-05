"""Menu-bar icon: state at a glance, dashboard, pause, restart, quit."""
import asyncio
import webbrowser

import rumps

from . import config

# SF Symbols (monochrome, follow light/dark mode like the system's own icons)
SYMBOLS = {"ready": "waveform.circle", "listening": "waveform.circle.fill", "transcribing": "ellipsis.circle.fill",
           "thinking": "ellipsis.circle.fill", "working": "ellipsis.circle.fill", "error": "xmark.circle",
           "paused": "pause.circle", "no-stick": "circle.dashed", "searching": "circle.dashed",
           "starting": "circle.dashed", "approval": "exclamationmark.circle.fill"}
_images = {}


def symbol(name):
    """NSImage for an SF Symbol, sized for the menu bar, or None."""
    if name not in _images:
        try:
            from AppKit import NSImage, NSImageSymbolConfiguration
            img = NSImage.imageWithSystemSymbolName_accessibilityDescription_(name, "stick-agent")
            cfg = NSImageSymbolConfiguration.configurationWithPointSize_weight_(15, 0.3)  # ~ regular weight
            img = img.imageWithSymbolConfiguration_(cfg) if img else None
            if img:
                img.setTemplate_(True)
            _images[name] = img
        except Exception:  # noqa: BLE001 — older macOS: fall back to text
            _images[name] = None
    return _images[name]
LABELS = {"ready": "Ready — hold B on the stick", "listening": "Listening…", "transcribing": "Transcribing…",
          "thinking": "Thinking…", "working": "Working…", "error": "Error", "paused": "Paused",
          "no-stick": "No stick chosen — open the dashboard", "searching": "Looking for the stick…",
          "starting": "Starting…"}


class Menu(rumps.App):
    def __init__(self, agent):
        super().__init__("stick-agent", title="", quit_button=None)
        self.shown = None
        self.agent = agent
        self.status_item = rumps.MenuItem("Starting…")
        # "Update available: v1.8.0" (updates.py) — hidden until there is one
        self.update_item = rumps.MenuItem("", callback=lambda _: webbrowser.open(f"http://localhost:{config.PORT}/#settings"))
        self.update_shown = None
        self.pause_item = rumps.MenuItem("Pause", callback=self.toggle_pause)
        # Stick speaker: volume levels + sounds on/off (saved on the stick)
        self.volume_menu = rumps.MenuItem("Stick volume")
        self.volume_items = {}
        for pct in (0, 25, 50, 75, 100):
            item = rumps.MenuItem("Mute" if pct == 0 else f"{pct}%", callback=lambda _, p=pct: self.set_volume(p))
            self.volume_items[pct] = item
            self.volume_menu.add(item)
        self.sounds_item = rumps.MenuItem("Sounds on the stick", callback=self.toggle_sounds)
        self.menu = [self.status_item, self.update_item, None,
                     rumps.MenuItem("Open dashboard", callback=lambda _: webbrowser.open(f"http://localhost:{config.PORT}")),
                     self.volume_menu, self.sounds_item, None,
                     self.pause_item, rumps.MenuItem("Restart", callback=self.restart), None,
                     rumps.MenuItem("Quit stick-agent", callback=self.quit)]
        self.timer = rumps.Timer(self.refresh, 0.5)
        self.timer.start()

    def refresh(self, _):
        s = self.agent.state
        prompts = len(self.agent.prompts)
        key = "approval" if prompts else s
        if key != self.shown:
            item = getattr(getattr(self, "_nsapp", None), "nsstatusitem", None)
            img = symbol(SYMBOLS.get(key, "circle.dashed"))
            if item is not None and not getattr(self, "named", False):
                item.setAutosaveName_("stick-agent")  # macOS remembers where you ⌘-drag it
                self.named = True
            if item is not None and img is not None:
                item.button().setImage_(img)
                item.button().setTitle_("")
                self.shown = key
            elif item is not None:
                self.title = "Stick"
                self.shown = key
        text = f"Waiting for approval ({prompts})" if prompts else LABELS.get(s, s)
        if self.agent.detail and s in ("working", "error"):
            text += f" — {self.agent.detail[:50]}"
        self.status_item.title = text
        self.pause_item.title = "Resume" if self.agent.link.paused else "Pause"
        snd = self.agent.sounds or {}
        if "volume" in snd:
            vol = round(snd["volume"] / 2.55)
            for pct, item in self.volume_items.items():
                item.state = 1 if abs(vol - pct) < 13 and (pct > 0 or vol == 0) else 0
        self.sounds_item.state = 1 if snd.get("on") else 0
        self.show_update()

    def show_update(self):
        updates = getattr(self.agent, "updates", None)
        title = updates.status().get("title", "") if updates else ""
        if title == self.update_shown:
            return
        self.update_shown = title
        self.update_item.title = f"{title} — open the dashboard" if title else ""
        try:
            self.update_item._menuitem.setHidden_(not title)
        except AttributeError:  # (rumps internals moved) — an empty title still says nothing
            pass

    def _stick(self, payload):
        """Runs a sound command on the agent's loop (we're on the menu-bar thread)."""
        if self.agent.loop:
            asyncio.run_coroutine_threadsafe(self.agent.sound(payload), self.agent.loop)

    def set_volume(self, pct):
        self._stick({"volume": round(pct * 2.55)})
        if pct:
            self._stick({"play": "answer"})  # a sample at the new level

    def toggle_sounds(self, _):
        self._stick({"on": not (self.agent.sounds or {}).get("on", True)})

    def toggle_pause(self, _):
        self.agent.loop.call_soon_threadsafe(self.agent.link.pause, not self.agent.link.paused)

    def restart(self, _):
        self.agent.restart_now()

    def quit(self, _):
        rumps.quit_application()  # exit 0: the login item doesn't bring it back until next login / start
