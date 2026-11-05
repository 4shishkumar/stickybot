"""The dashboard in StickyBot's own window: a WKWebView on http://localhost:<port>/.

The page gets its token from the server-rendered HTML, so loading the URL is all it
takes (the same page as in a browser). Runs on the main thread next to rumps, which
owns the NSApplication run loop: show() is called from a menu callback, or through
show_soon() (AppHelper.callAfter) from any other thread.

- Closing the window doesn't quit; the page is unloaded (no live connection left behind).
- show() again focuses the open window (and moves it to #page if one is asked for).
- While it's open StickyBot has a Dock icon and a menu bar (Edit menu: copy / paste
  work in the page's fields), and it's Accessory (menu bar icon only) again after.
- Links to anything but the dashboard open in the default browser (or System Settings).

Needs pyobjc-framework-WebKit; without it available() is False and callers use the browser.
"""
import json
import logging

from . import config

log = logging.getLogger("window")
REGULAR, ACCESSORY = 0, 1  # NSApplicationActivationPolicy
ALLOW, CANCEL = 1, 0      # WKNavigationActionPolicy
REOPEN = (0x61657674, 0x72617070)  # kCoreEventClass 'aevt', kAEReopenApplication 'rapp'
_classes = {}


def available():
    try:
        import WebKit  # noqa: F401
    except ImportError:
        return False
    return True


def url_for(page=""):
    return f"http://localhost:{config.PORT}/" + (f"#{page}" if page else "")


def ours(url):
    """Is this URL the dashboard (stays in the window) rather than something to open elsewhere?"""
    from urllib.parse import urlsplit
    if url in ("about:blank", ""):
        return True
    u = urlsplit(url)
    return u.scheme == "http" and u.hostname in ("localhost", "127.0.0.1") and u.port == config.PORT


def guarded(what, fn, *args, fallback=None):
    """Runs fn(*args) for an Objective-C callback: an exception that got back to Objective-C
    would abort the whole process, so it's logged and `fallback` returned instead."""
    try:
        return fn(*args)
    except Exception:  # noqa: BLE001
        log.exception("window: %s failed", what)
        return fallback


def navigation_policy(url, open_elsewhere):
    """ALLOW for the dashboard; anything else goes to open_elsewhere(url) (the browser) -> CANCEL.
    Never raises."""
    if not url or guarded("checking a link", ours, url, fallback=False):
        return ALLOW
    guarded("opening a link", open_elsewhere, url)
    return CANCEL


def answer_once(what, done, compute, fallback):
    """Calls a WebKit completion handler exactly once: with compute()'s result, or `fallback`
    if compute() raised. Never raises."""
    value = guarded(what, compute, fallback=fallback)
    guarded(f"{what} (answering)", done, *(() if value is None else (value,)))


def _delegate_class():
    """The Objective-C delegate class (window + navigation + UI), defined once per process.
    Every method body goes through guarded() / answer_once(): see guarded()."""
    if "delegate" in _classes:
        return _classes["delegate"]
    import objc
    import WebKit  # noqa: F401 — first: its metadata gives the delegate methods their block signatures
    from AppKit import NSAlert, NSWorkspace
    from Foundation import NSObject

    def open_url(nsurl):
        NSWorkspace.sharedWorkspace().openURL_(nsurl)

    def url_text(action):
        u = action.request().URL()
        return (str(u.absoluteString()) if u is not None else ""), u

    def alert(msg, buttons=()):
        a = NSAlert.alloc().init()
        a.setMessageText_(str(msg))
        for b in buttons:
            a.addButtonWithTitle_(b)
        return a.runModal()

    class StickyBotWindowDelegate(NSObject):
        def windowWillClose_(self, _note):
            guarded("closing", self.owner.closed)

        def webView_decidePolicyForNavigationAction_decisionHandler_(self, _view, action, handler):
            def decide():
                text, nsurl = url_text(action)
                return navigation_policy(text, lambda _t: open_url(nsurl))
            answer_once("a navigation", handler, decide, CANCEL)

        def webView_createWebViewWithConfiguration_forNavigationAction_windowFeatures_(self, _v, _c, action, _f):
            def elsewhere():  # target="_blank": the browser, never a second web view
                _text, nsurl = url_text(action)
                if nsurl is not None:
                    open_url(nsurl)
            guarded("a new-window link", elsewhere)
            return None

        def webView_runJavaScriptAlertPanelWithMessage_initiatedByFrame_completionHandler_(self, _v, msg, _f, done):
            answer_once("an alert", done, lambda: alert(msg) and None, None)

        def webView_runJavaScriptConfirmPanelWithMessage_initiatedByFrame_completionHandler_(self, _v, msg, _f, done):
            answer_once("a confirm", done, lambda: alert(msg, ("OK", "Cancel")) == 1000, False)  # first button

        @objc.typedSelector(b"v@:@@")
        def handleReopen_withReplyEvent_(self, _event, _reply):
            guarded("reopen", self.owner.show)  # StickyBot.app opened again from Finder / Spotlight / the Dock

    _classes["delegate"] = StickyBotWindowDelegate
    return StickyBotWindowDelegate


def _app():
    from AppKit import NSApplication
    return NSApplication.sharedApplication()


class Window:
    def __init__(self):
        self.win = self.view = self._menu = None
        self.delegate = _delegate_class().alloc().init()
        self.delegate.owner = self

    def listen_for_reopen(self):
        """Opening StickyBot.app while it runs (Finder, Spotlight, the Dock) shows the window.
        After NSApplication finished launching (it installs its own handler then)."""
        from Foundation import NSAppleEventManager
        NSAppleEventManager.sharedAppleEventManager().setEventHandler_andSelector_forEventClass_andEventID_(
            self.delegate, b"handleReopen:withReplyEvent:", *REOPEN)

    def show(self, page=""):
        """Main thread only. -> True."""
        app = _app()
        if self.win is None:
            self._make()
            self.view.loadRequest_(self._request(url_for(page)))
        elif page:
            self.view.evaluateJavaScript_completionHandler_(f"location.hash = {json.dumps('#' + page)}", None)
        app.setActivationPolicy_(REGULAR)
        self._main_menu()
        self.win.makeKeyAndOrderFront_(None)
        app.activateIgnoringOtherApps_(True)
        return True

    def show_soon(self, page=""):
        """Any thread: show() on the main thread. -> True (it will be shown)."""
        from PyObjCTools import AppHelper
        AppHelper.callAfter(guarded, "showing the window", self.show, page)
        return True

    def closed(self):
        from PyObjCTools import AppHelper
        win, view, self.win, self.view = self.win, self.view, None, None
        if view is not None:
            view.loadRequest_(self._request("about:blank"))  # stop the page (its live event stream)
        AppHelper.callAfter(lambda: (win, view))  # they go once the close has finished
        _app().setActivationPolicy_(ACCESSORY)

    @staticmethod
    def _request(url):
        from Foundation import NSURL, NSURLRequest
        return NSURLRequest.requestWithURL_(NSURL.URLWithString_(url))

    def _make(self):
        from AppKit import (
            NSBackingStoreBuffered,
            NSMakeRect,
            NSWindow,
            NSWindowStyleMaskClosable,
            NSWindowStyleMaskMiniaturizable,
            NSWindowStyleMaskResizable,
            NSWindowStyleMaskTitled,
        )
        from WebKit import WKWebView, WKWebViewConfiguration
        style = (NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable
                 | NSWindowStyleMaskResizable)
        win = NSWindow.alloc().initWithContentRect_styleMask_backing_defer_(
            NSMakeRect(0, 0, 1180, 820), style, NSBackingStoreBuffered, False)
        win.setTitle_("StickyBot")
        win.setReleasedWhenClosed_(False)
        win.setMinSize_((720, 480))
        win.center()
        win.setFrameAutosaveName_("StickyBotDashboard")  # macOS remembers its size and place
        win.setDelegate_(self.delegate)
        view = WKWebView.alloc().initWithFrame_configuration_(win.contentView().bounds(),
                                                              WKWebViewConfiguration.alloc().init())
        view.setAutoresizingMask_(18)  # NSViewWidthSizable | NSViewHeightSizable
        view.setNavigationDelegate_(self.delegate)
        view.setUIDelegate_(self.delegate)
        win.setContentView_(view)
        self.win, self.view = win, view

    def _main_menu(self):
        """App + Edit + View menus while the window is open (⌘C / ⌘V / ⌘R / ⌘W work in the page)."""
        from AppKit import NSMenu, NSMenuItem
        if self._menu is not None:
            return
        bar = NSMenu.alloc().init()

        def menu(title, items):
            m = NSMenu.alloc().initWithTitle_(title)
            for it in items:
                if it is None:
                    m.addItem_(NSMenuItem.separatorItem())
                    continue
                label, action, key = it
                m.addItemWithTitle_action_keyEquivalent_(label, action, key)
            top = NSMenuItem.alloc().init()
            top.setSubmenu_(m)
            bar.addItem_(top)
            return m

        menu("StickyBot", [("Hide StickyBot", "hide:", "h"), None, ("Close Window", "performClose:", "w"), None,
                           ("Quit StickyBot", "terminate:", "q")])
        menu("Edit", [("Undo", "undo:", "z"), ("Redo", "redo:", "Z"), None, ("Cut", "cut:", "x"),
                      ("Copy", "copy:", "c"), ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")])
        menu("View", [("Reload", "reload:", "r")])
        _app().setMainMenu_(bar)
        self._menu = bar
