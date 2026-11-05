"""StickyBot command line (`stickybot`; the older name `stick-agent` works too).

  stickybot run        run in the foreground (menu bar + dashboard)
  stickybot start      start the background service (also at every login)
  stickybot stop       stop it (until `start` or the next login)
  stickybot restart
  stickybot status
  stickybot logs       follow the log
  stickybot open       open the dashboard in the StickyBot window  [--browser: in the browser]
  stickybot install-page   open the setup page (speech model, Claude Code, GitHub, mic, your stick)
  stickybot app        the StickyBot.app bundle: status | build (if needed) | rebuild (+ restart)
  stickybot flash      flash the bundled firmware to a stick on USB  [--port P] [--backup]
                       --ble: over Bluetooth instead (stick 1.8.0+, needs the agent running)  [--force]
  stickybot setup      install / check / remove parts (see `stickybot setup --help`)
  stickybot mcp        the stick tools for terminal Claude Code sessions: install | uninstall | status  [--yes]
  stickybot uninstall  remove everything it installed  [--purge: also delete ~/.stick-agent]
  stickybot --help     this help (also `stickybot <command> --help`; nothing is run)
"""
import asyncio
import json
import logging
import logging.handlers
import os
import plistlib
import subprocess
import sys
import threading
import time
import urllib.request
import webbrowser
from pathlib import Path

from . import __version__, config

LABEL = os.environ.get("STICK_AGENT_LABEL", "com.stickkey.agent")  # overridable for test installs
LOGIN_PLIST = Path.home() / "Library/LaunchAgents" / f"{LABEL}.plist"  # loaded again at every login
LOCAL_PLIST = config.HOME / f"{LABEL}.plist"                            # fallback: this login only


def plist_path():
    """~/Library/LaunchAgents if we may write there (some installers leave it owned by root)."""
    d = LOGIN_PLIST.parent
    try:
        d.mkdir(parents=True, exist_ok=True)
    except OSError:
        return LOCAL_PLIST
    return LOGIN_PLIST if os.access(d, os.W_OK) else LOCAL_PLIST


def setup_logging():
    config.HOME.mkdir(parents=True, exist_ok=True)
    fmt = logging.Formatter("%(asctime)s %(name)s: %(message)s", "%H:%M:%S")
    root = logging.getLogger()
    root.setLevel(logging.INFO)
    fh = logging.handlers.RotatingFileHandler(config.LOG, maxBytes=2_000_000, backupCount=2)
    fh.setFormatter(fmt)
    root.addHandler(fh)
    if sys.stderr.isatty():
        sh = logging.StreamHandler()
        sh.setFormatter(fmt)
        root.addHandler(sh)
    for noisy in ("bleak", "asyncio", "aiohttp.access", "httpx", "httpx2", "huggingface_hub",
                  "huggingface_hub.utils._http"):
        logging.getLogger(noisy).setLevel(logging.WARNING)


class EventLog(logging.Handler):
    """Mirrors log lines to the dashboard's live feed."""

    def __init__(self, agent):
        super().__init__(logging.INFO)
        self.agent = agent
        self.setFormatter(logging.Formatter("%(name)s: %(message)s"))

    def emit(self, record):
        try:
            line = self.format(record)
            if self.agent.loop:
                self.agent.loop.call_soon_threadsafe(self.agent.emit, "log", text=line)
        except Exception:  # noqa: BLE001
            pass


def clean_exit(agent):
    """The stick mic device outlives the process that made it, so every way out removes it:
    a normal exit / Ctrl-C (atexit) and SIGTERM (launchd stop, `kill`). The handler runs on the
    main thread: within 0.5 s under the menu bar (its refresh timer runs Python code)."""
    import atexit
    import signal
    atexit.register(agent.shutdown)

    def on_term(*_):
        logging.getLogger("main").info("SIGTERM: removing the stick mic device, exiting")
        agent.shutdown()
        logging.shutdown()
        os._exit(128 + signal.SIGTERM)  # as if killed by it (launchd's KeepAlive treats it the same)
    signal.signal(signal.SIGTERM, on_term)


def run(menubar=True):
    setup_logging()
    from . import core, server
    agent = core.Agent()
    logging.getLogger().addHandler(EventLog(agent))
    from . import app
    logging.getLogger("main").info("StickyBot %s starting (pid %d, as %s)", __version__, os.getpid(),
                                   app.display_name())
    clean_exit(agent)
    if os.environ.get("XPC_SERVICE_NAME") == LABEL:  # started by the login item
        threading.Thread(target=switch_to_app, daemon=True, name="app-check").start()

    async def main():
        await server.serve(agent)
        if not installer_opens_page() and not agent.cfg.get("stick"):
            agent.open_page("install")  # first run: the setup page
        await agent.run()

    if not menubar:
        asyncio.run(main())
        return
    from . import menubar as mb
    menu = mb.Menu(agent)  # first: it gives the agent its window before the dashboard answers
    threading.Thread(target=lambda: asyncio.run(main()), daemon=True, name="agent").start()
    menu.run()


def switch_to_app():
    """The login item started us: make sure StickyBot.app is built (a first start after an update
    from the dashboard, or a new Python) and that the login item runs it. Switching the login item
    restarts the agent through launchd, from a helper outside our process group (launchd stops
    the job's whole group)."""
    from . import app
    lg = logging.getLogger("app")
    ok, msg = app.ensure(say=lg.info)
    lg.info("%s", msg)
    if not ok or not LOGIN_PLIST.exists() and not LOCAL_PLIST.exists():
        return
    program, _ = app.login_program()
    if login_item_program() == program:
        return
    path = write_plist()
    lg.info("the login item now runs %s — restarting through launchd", program[0])
    subprocess.Popen(["/bin/sh", "-c", 'sleep 1; launchctl bootout "$1"; launchctl bootstrap "$2" "$3"', "sh",
                      f"{domain()}/{LABEL}", domain(), str(path)], start_new_session=True,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def login_item_program():
    """ProgramArguments of the login item's plist on disk, or None."""
    for p in (LOGIN_PLIST, LOCAL_PLIST):
        try:
            with p.open("rb") as f:
                return plistlib.load(f).get("ProgramArguments")
        except (OSError, plistlib.InvalidFileException, ValueError):
            continue
    return None


INSTALLER_FLAG = config.HOME / "installer-opens-page"  # install.sh writes it: it opens the page itself


def installer_opens_page():
    """True (once) when install.sh just started us and opens the setup page itself — so the
    first run doesn't open a second tab. A flag older than 10 minutes is ignored."""
    try:
        fresh = time.time() - INSTALLER_FLAG.stat().st_mtime < 600
        INSTALLER_FLAG.unlink()
        return fresh
    except OSError:
        return False


# --- background service (launchd login item) -----------------------------------

def write_plist():
    """The login item. It runs StickyBot.app's executable (the venv's Python as "StickyBot", see
    app.py) when the bundle is built and starts, else the venv's python as before."""
    from . import app
    path = plist_path()
    program, env = app.login_program()
    plist = {
        "Label": LABEL,
        "ProgramArguments": program,
        "WorkingDirectory": str(Path(__file__).resolve().parent.parent),
        "EnvironmentVariables": {"PYTHONPATH": str(Path(__file__).resolve().parent.parent),
                                 "PATH": f"{Path.home()}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
                                 **env},
        "RunAtLoad": True,
        "KeepAlive": {"SuccessfulExit": False},  # restart after a crash, not after Quit
        "ProcessType": "Interactive",
        "StandardOutPath": str(config.HOME / "launchd.out"),
        "StandardErrorPath": str(config.HOME / "launchd.out"),
    }
    if env:  # System Settings → Login Items shows it under StickyBot
        plist["AssociatedBundleIdentifiers"] = [app.BUNDLE_ID]
    with path.open("wb") as f:
        plistlib.dump(plist, f)
    return path


def launchctl(*args):
    return subprocess.run(["launchctl", *args], capture_output=True, text=True)


def domain():
    return f"gui/{os.getuid()}"


def start():
    path = write_plist()
    launchctl("bootout", f"{domain()}/{LABEL}")
    r = launchctl("bootstrap", domain(), str(path))
    if r.returncode != 0:
        print(f"failed: {r.stderr.strip()}")
        return
    print("started")
    if path == LOCAL_PLIST:
        print(f"note: {LOGIN_PLIST.parent} isn't writable (owned by another user), so it won't start\n"
              f"      by itself at the next login. Fix once with:  sudo chown $(whoami) {LOGIN_PLIST.parent}\n"
              f"      then run start again.")


def stop():
    r = launchctl("bootout", f"{domain()}/{LABEL}")
    print("stopped" if r.returncode == 0 else "not running")


def restart():
    if launchctl("kickstart", "-k", f"{domain()}/{LABEL}").returncode != 0:
        start()
    else:
        print("restarted")


def status():
    loaded = launchctl("print", f"{domain()}/{LABEL}").returncode == 0
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{config.PORT}/api/status", timeout=2) as r:
            s = json.load(r)
        stick = (s.get("stick") or {}).get("name", "none chosen")
        print(f"running — {s['state']} | stick: {stick} ({s['link']}) | model {s.get('model')} | "
              f"{s['stats']['requests']} requests | dashboard http://localhost:{config.PORT}")
    except OSError:
        print("service installed but not responding" if loaded else "not running")


FLASH_OPTS = {"--port", "--backup", "--ble", "--force"}


def app_cli(args):
    """`stickybot app [status | build | rebuild]` -> exit code."""
    from . import app
    what = args[0] if args else "status"
    if what == "status":
        exe = app.executable()
        why = app.needs_build() if exe.exists() else "not built yet"
        print(f"{app.bundle()}: {why or 'built, up to date'}")
        program = login_item_program()
        print("login item: " + ("runs StickyBot.app" if program and program[0] == str(exe)
                                else "runs plain Python" if program else "not installed"))
        print(f"this process: {app.display_name()}")
        return 0
    if what == "build":  # only if needed, and never touches the login item (self-update runs it)
        ok, msg = app.ensure(say=print)
        print(msg)
        return 0 if ok else 1
    if what == "rebuild":
        try:
            r = app.build(force=True, say=print)
        except app.BuildError as e:
            print(f"couldn't build it: {e} — the agent keeps running as Python")
            return 1
        print(f"{r['path']} ready")
        if launchctl("print", f"{domain()}/{LABEL}").returncode == 0:
            start()  # the login item now runs the (new) bundle
        return 0
    print("usage: stickybot app [status | build | rebuild]")
    return 2


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "run"
    args = sys.argv[2:]
    if cmd in ("-h", "--help", "help") or (cmd not in ("setup", "mcp") and ({"-h", "--help"} & set(args))):
        print(__doc__)  # never "do it anyway": `flash --help` used to start flashing
        return
    if cmd == "run":
        run(menubar="--no-menubar" not in sys.argv)
    elif cmd == "start":
        start()
    elif cmd == "stop":
        stop()
    elif cmd == "restart":
        restart()
    elif cmd == "status":
        status()
    elif cmd == "logs":
        os.execvp("tail", ["tail", "-n", "50", "-f", str(config.LOG)])
    elif cmd == "open":  # the StickyBot window (the browser if the agent can't show one)
        from . import app
        if "--browser" in args or not app.request_window():
            webbrowser.open(f"http://localhost:{config.PORT}")
    elif cmd == "app":  # stickybot app [status | build | rebuild]
        sys.exit(app_cli(args))
    elif cmd == "install-page":  # the setup page (what install.sh opens)
        from . import install_flow
        sys.exit(install_flow.open_page())
    elif cmd == "setup":
        from . import installer
        sys.exit(installer.main(sys.argv[2:]))
    elif cmd == "mcp":  # stickybot mcp install|uninstall|status [--yes]
        from . import mcp_setup
        sys.exit(mcp_setup.cli(sys.argv[2:]))
    elif cmd == "uninstall":  # = setup --uninstall all
        from . import installer
        sys.exit(installer.main(["--uninstall", "all", *sys.argv[2:]]))
    elif cmd == "flash":  # stickybot flash [--port P] [--backup] | --ble [--force]
        from . import flasher
        unknown = [a for i, a in enumerate(args) if a not in FLASH_OPTS and not (i and args[i - 1] == "--port")]
        if unknown:
            sys.exit(f"stickybot flash: unknown option {unknown[0]} (see stickybot --help)")
        if "--ble" in args:
            from . import ota
            sys.exit(ota.cli("--force" in args))
        port = args[args.index("--port") + 1] if "--port" in args and args.index("--port") + 1 < len(args) else None
        sys.exit(flasher.cli(port, "--backup" in args))
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
