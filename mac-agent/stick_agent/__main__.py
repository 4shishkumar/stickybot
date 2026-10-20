"""stick-agent command line.

  stick-agent run        run in the foreground (menu bar + dashboard)
  stick-agent start      start the background service (also at every login)
  stick-agent stop       stop it (until `start` or the next login)
  stick-agent restart
  stick-agent status
  stick-agent logs       follow the log
  stick-agent open       open the dashboard
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
import urllib.request
import webbrowser
from pathlib import Path

from . import config

LABEL = "com.stickkey.agent"
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


def run(menubar=True):
    setup_logging()
    from . import core, server
    agent = core.Agent()
    logging.getLogger().addHandler(EventLog(agent))
    logging.getLogger("main").info("stick-agent starting (pid %d)", os.getpid())

    async def main():
        await server.serve(agent)
        if not agent.cfg.get("stick"):
            webbrowser.open(f"http://localhost:{config.PORT}")  # first run: pick your stick
        await agent.run()

    if not menubar:
        asyncio.run(main())
        return
    from . import menubar as mb
    threading.Thread(target=lambda: asyncio.run(main()), daemon=True, name="agent").start()
    mb.Menu(agent).run()


# --- background service (launchd login item) -----------------------------------

def write_plist():
    path = plist_path()
    plist = {
        "Label": LABEL,
        "ProgramArguments": [sys.executable, "-m", "stick_agent", "run"],
        "WorkingDirectory": str(Path(__file__).resolve().parent.parent),
        "EnvironmentVariables": {"PYTHONPATH": str(Path(__file__).resolve().parent.parent),
                                 "PATH": f"{Path.home()}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"},
        "RunAtLoad": True,
        "KeepAlive": {"SuccessfulExit": False},  # restart after a crash, not after Quit
        "ProcessType": "Interactive",
        "StandardOutPath": str(config.HOME / "launchd.out"),
        "StandardErrorPath": str(config.HOME / "launchd.out"),
    }
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


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "run"
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
    elif cmd == "open":
        webbrowser.open(f"http://localhost:{config.PORT}")
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
