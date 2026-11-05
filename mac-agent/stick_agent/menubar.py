"""StickyBot's menu-bar icon: state at a glance, the dashboard (its own window or the browser),
update check, pause, restart, quit."""
import asyncio
import logging
import threading
import time
import webbrowser

import rumps

from . import config

# The menu-bar icon is the dashboard's brand mark (the stick with two eyes), drawn as a template
# image so it follows light / dark mode; the eyes show the state.
STYLES = {"ready": "eyes", "listening": "filled", "transcribing": "up", "thinking": "up", "working": "up",
          "error": "x", "paused": "closed", "no-stick": "dashed", "searching": "dashed", "starting": "dashed",
          "approval": "badge"}
_images = {}


def mark(style):
    """NSImage (18 x 18 pt, template) of the StickyBot mark in `style`, or None (no AppKit)."""
    if style not in _images:
        try:
            _images[style] = _draw_mark(style)
        except Exception:  # noqa: BLE001 — no AppKit: fall back to text
            logging.getLogger("menubar").exception("menu-bar icon")
            _images[style] = None
    return _images[style]


def _draw_mark(style):
    from AppKit import NSBezierPath, NSColor, NSCompositingOperationClear, NSGraphicsContext, NSImage
    from Foundation import NSMakeRect, NSMakeSize, NSPoint
    img = NSImage.alloc().initWithSize_(NSMakeSize(18, 18))
    img.lockFocus()
    try:
        NSColor.blackColor().set()
        body = NSBezierPath.bezierPathWithRoundedRect_xRadius_yRadius_(NSMakeRect(4.6, 1.2, 8.8, 15.6), 3.4, 3.4)
        body.setLineWidth_(1.4)
        if style == "dashed":
            body.setLineDash_count_phase_([2.2, 1.6], 2, 0)
        if style == "filled":
            body.fill()
        else:
            body.stroke()
        if style == "filled":  # eyes cut out of the filled stick
            NSGraphicsContext.currentContext().setCompositingOperation_(NSCompositingOperationClear)
        y = 10.6 if style == "up" else 8.6
        if style in ("eyes", "filled", "up", "badge"):
            for x in (6.9, 9.6):
                NSBezierPath.bezierPathWithRoundedRect_xRadius_yRadius_(NSMakeRect(x, y, 1.6, 3.6), 0.8, 0.8).fill()
        elif style == "closed":
            for x in (6.6, 9.4):
                NSBezierPath.fillRect_(NSMakeRect(x, 10.0, 2.2, 1.1))
        elif style == "x":
            for cx in (7.7, 10.4):
                for dx in (-1, 1):
                    line = NSBezierPath.bezierPath()
                    line.moveToPoint_(NSPoint(cx - 1, 9.4 - dx))
                    line.lineToPoint_(NSPoint(cx + 1, 9.4 + dx))
                    line.setLineWidth_(1.0)
                    line.stroke()
        if style == "badge":  # something waits for you
            NSBezierPath.bezierPathWithOvalInRect_(NSMakeRect(12.4, 12.4, 5.2, 5.2)).fill()
    finally:
        img.unlockFocus()
    img.setTemplate_(True)
    img.setAccessibilityDescription_("StickyBot")
    return img


CHECK = "Check for Updates…"
SETUP = "Finish setup…"
SETUP_EVERY_S = 30


def check_result(s):
    """An update check's status() -> what the menu item says."""
    if s.get("available"):
        return f"{s['title']} — see the item above"
    if s.get("error"):
        return f"Couldn't check for updates: {str(s['error'])[:60]}"
    return f"Up to date (v{s.get('current', {}).get('agent', '?')})"


LABELS = {"ready": "Ready — hold B on the stick", "listening": "Listening…", "transcribing": "Transcribing…",
          "thinking": "Thinking…", "working": "Working…", "error": "Error", "paused": "Paused",
          "no-stick": "No stick chosen — open the dashboard", "searching": "Looking for the stick…",
          "starting": "Starting…"}


class Menu(rumps.App):
    def __init__(self, agent):
        super().__init__("StickyBot", title="", quit_button=None)
        self.shown = None
        self.agent = agent
        self.window = None  # the dashboard window (window.py), if WebKit is there
        try:
            from . import window
            if window.available():
                self.window = window.Window()
        except Exception:  # noqa: BLE001 — the browser then
            logging.getLogger("window").exception("no dashboard window")
        agent.window = self.window.show_soon if self.window else None
        self.reopen_listening = False
        self.status_item = rumps.MenuItem("Starting…")
        # "Update available: v1.8.0" (updates.py) — hidden until there is one
        self.update_item = rumps.MenuItem("", callback=lambda _: self.open_page("settings"))
        self.update_shown = None
        self.check_item = rumps.MenuItem(CHECK, callback=self.check_updates)  # its title shows the result
        self.check_said = 0.0
        self.pause_item = rumps.MenuItem("Pause", callback=self.toggle_pause)
        # Stick speaker: volume levels + sounds on/off (saved on the stick)
        self.volume_menu = rumps.MenuItem("Stick volume")
        self.volume_items = {}
        for pct in (0, 25, 50, 75, 100):
            item = rumps.MenuItem("Mute" if pct == 0 else f"{pct}%", callback=lambda _, p=pct: self.set_volume(p))
            self.volume_items[pct] = item
            self.volume_menu.add(item)
        self.sounds_item = rumps.MenuItem("Sounds on the stick", callback=self.toggle_sounds)
        # "Finish setup…" (the setup page) while the speech model or the stick isn't set up yet
        self.setup_item = rumps.MenuItem(SETUP, callback=lambda _: self.open_page("install"))
        self.setup_pending, self.setup_checked = None, 0.0
        self.menu = [self.status_item, self.update_item, self.setup_item, None,
                     rumps.MenuItem("Open StickyBot", callback=lambda _: self.open_page()),
                     rumps.MenuItem("Open in browser", callback=lambda _: webbrowser.open(f"http://localhost:{config.PORT}")),
                     self.check_item, self.volume_menu, self.sounds_item, None,
                     self.pause_item, rumps.MenuItem("Restart", callback=self.restart), None,
                     rumps.MenuItem("Quit StickyBot", callback=self.quit)]
        self.timer = rumps.Timer(self.refresh, 0.5)
        self.timer.start()

    def open_page(self, page=""):
        """Main thread (a menu callback): the dashboard in the StickyBot window, else the browser."""
        if self.window:
            try:
                self.window.show(page)
                return
            except Exception:  # noqa: BLE001
                logging.getLogger("window").exception("couldn't show the window")
        webbrowser.open(f"http://localhost:{config.PORT}/" + (f"#{page}" if page else ""))

    def refresh(self, _):
        if self.window and not self.reopen_listening:  # NSApplication has finished launching by now
            self.reopen_listening = True
            try:
                self.window.listen_for_reopen()
            except Exception:  # noqa: BLE001
                logging.getLogger("window").exception("no reopen handler")
        s = self.agent.state
        prompts = len(self.agent.prompts)
        key = "approval" if prompts else s
        if key != self.shown:
            item = getattr(getattr(self, "_nsapp", None), "nsstatusitem", None)
            img = mark(STYLES.get(key, "dashed"))
            if item is not None and not getattr(self, "named", False):
                item.setAutosaveName_("stick-agent")  # macOS remembers where you ⌘-drag it
                self.named = True
            if item is not None and img is not None:
                item.button().setImage_(img)
                item.button().setTitle_("")
                self.shown = key
            elif item is not None:
                self.title = "StickyBot"
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
        self.show_setup()
        if self.check_said and time.monotonic() - self.check_said > 8:
            self.check_item.title, self.check_said = CHECK, 0.0

    def check_updates(self, _):
        updates = getattr(self.agent, "updates", None)
        if not updates or not self.agent.loop:
            return
        self.check_item.title, self.check_said = "Checking for updates…", 0.0

        def said(fut):
            try:
                self.check_item.title = check_result(fut.result())
            except Exception as e:  # noqa: BLE001
                self.check_item.title = f"Couldn't check for updates: {str(e)[:60]}"
            self.check_said = time.monotonic()  # back to "Check for Updates…" a few seconds later

        asyncio.run_coroutine_threadsafe(updates.check(), self.agent.loop).add_done_callback(said)

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

    def show_setup(self):
        """Shows "Finish setup…" while a recommended setup step is open; checked every 30 s off the
        menu-bar thread (it looks at the model cache)."""
        now = time.monotonic()
        if now - self.setup_checked >= SETUP_EVERY_S:
            self.setup_checked = now

            def check():
                from . import install_flow
                try:
                    self.setup_pending = install_flow.quick_pending(self.agent.cfg)
                except Exception:  # noqa: BLE001
                    self.setup_pending = False
            threading.Thread(target=check, daemon=True, name="setup-check").start()
        try:
            self.setup_item._menuitem.setHidden_(not self.setup_pending)
        except AttributeError:
            self.setup_item.title = SETUP if self.setup_pending else ""

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
