"""StickyBot.app: a small app bundle around the agent's own Python, built on this Mac.

macOS names a process after the bundle of its main executable: run as
`venv/bin/python` the agent is "Python" (org.python.python) in permission prompts,
Privacy & Security, Activity Monitor and Force Quit. A launcher that exec()s python
doesn't help, so the bundle's executable IS the interpreter:

  ~/Applications/StickyBot.app/Contents/
    Info.plist                 com.stickybot.app, LSUIElement, the usage texts
    MacOS/StickyBot            a copy of the venv's real interpreter binary
    Resources/StickyBot.icns
    Resources/boot/sitecustomize.py   only when opened from Finder (see opened())

The copied interpreter finds the standard library and the venv's site-packages
through __PYVENV_LAUNCHER__=<venv>/bin/python: what a framework build's own
`bin/python3` sets for Python.app; CPython reads it on macOS (framework or not),
treats that path as sys.executable (so pyvenv.cfg and site-packages are the venv's)
and removes it from the environment. The login item sets it (EnvironmentVariables),
and so does Info.plist's LSEnvironment for a launch from Finder / Spotlight.

Signed ad hoc (`codesign -s -`), so no Developer ID: built locally, nothing is
quarantined and Gatekeeper never asks. macOS ties permissions to the signature, so
the bundle is rebuilt only when the interpreter changes (its sha256 is in Info.plist,
StickyBotBuild) or the bundle's layout does (FORMAT); code updates (.py) never touch it.

Test-friendly: STICK_AGENT_APP moves the bundle; build() takes a runner for the
commands it runs (codesign, install_name_tool, the health check).
"""
import ctypes
import functools
import hashlib
import logging
import os
import plistlib
import shutil
import subprocess
import sys
import sysconfig
from pathlib import Path

from . import config

log = logging.getLogger("app")

NAME = "StickyBot"
BUNDLE_ID = "com.stickybot.app"
FORMAT = 1  # bump when the bundle's layout or Info.plist changes: rebuilds it (permissions are asked again)
ICON = Path(__file__).parent / "icon" / f"{NAME}.icns"
MIN_MACOS = "13.0"
HEALTH = "import stick_agent, aiohttp, bleak, rumps"

BOOT = '''\
# StickyBot.app opened from Finder, Spotlight or the Dock while the agent isn't running:
# hand over to the login item (it starts the agent), show its window, exit.
# Only on PYTHONPATH for such launches (Info.plist LSEnvironment); never for the agent itself.
import sys

if len(getattr(sys, "orig_argv", ())) <= 1:
    from stick_agent import app

    app.opened()
'''


class BuildError(Exception):
    pass


def bundle():
    return Path(os.environ.get("STICK_AGENT_APP", Path.home() / "Applications" / f"{NAME}.app"))


def executable(app=None):
    return Path(app or bundle()) / "Contents" / "MacOS" / NAME


def venv_python():
    """The venv's python (the __PYVENV_LAUNCHER__ target), or None outside a venv."""
    if sys.prefix == sys.base_prefix:
        return None
    return Path(sys.prefix) / "bin" / "python"


def interpreter():
    """The real interpreter binary behind the venv, resolved — not a launcher stub.
    A framework build's bin/python3.X only spawns Resources/Python.app/Contents/MacOS/Python
    (that's where the "Python" app identity comes from), so that one is copied there."""
    base = Path(sys.base_prefix)
    if sysconfig.get_config_var("PYTHONFRAMEWORK"):
        exe = base / "Resources" / "Python.app" / "Contents" / "MacOS" / "Python"
    else:
        exe = base / "bin" / f"python{sys.version_info.major}.{sys.version_info.minor}"
    exe = exe.resolve()
    if not exe.is_file():
        raise BuildError(f"no Python interpreter at {exe}")
    return exe


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def launch_env(app=None, venv=None):
    """What the bundle's executable needs in its environment to be the venv's Python."""
    venv = venv or venv_python()
    return {"__PYVENV_LAUNCHER__": str(venv), "STICKYBOT_APP": str(executable(app))}


def info_plist(stamp, app=None, venv=None):
    return {
        "CFBundleName": NAME,
        "CFBundleDisplayName": NAME,
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleExecutable": NAME,
        "CFBundleIconFile": NAME,
        "CFBundlePackageType": "APPL",
        "CFBundleInfoDictionaryVersion": "6.0",
        # the bundle's own version, not the agent's: code updates must not change the bundle
        "CFBundleVersion": str(FORMAT),
        "CFBundleShortVersionString": f"{FORMAT}.0",
        "LSMinimumSystemVersion": MIN_MACOS,
        "LSUIElement": True,  # a menu bar app: a Dock icon only while its window is open
        "LSApplicationCategoryType": "public.app-category.productivity",
        "NSHighResolutionCapable": True,
        "NSBluetoothAlwaysUsageDescription": "StickyBot talks to your stick over Bluetooth.",
        "NSAudioCaptureUsageDescription": "StickyBot taps only its own sound to make the stick's Bluetooth mic.",
        "NSMicrophoneUsageDescription": "StickyBot checks the stick's Bluetooth mic by listening to it.",
        "LSEnvironment": {**launch_env(app, venv),
                          "PYTHONPATH": str(Path(app or bundle()) / "Contents" / "Resources" / "boot")},
        "StickyBotBuild": stamp,
    }


def read_stamp(app=None):
    try:
        with (Path(app or bundle()) / "Contents" / "Info.plist").open("rb") as f:
            return plistlib.load(f).get("StickyBotBuild")
    except (OSError, plistlib.InvalidFileException, ValueError, AttributeError):
        return None


def wanted_stamp(python=None, venv=None):
    python = python or interpreter()
    return {"format": FORMAT, "python": sha256(python), "python_path": str(python),
            "venv": str(venv or venv_python()), "version": sys.version.split()[0]}


def run(cmd, env=None, timeout=60):
    """subprocess.run with text output; never raises. Tests replace it (build(runner=…))."""
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                              env={**os.environ, **env} if env else None)
    except (OSError, subprocess.TimeoutExpired) as e:
        return subprocess.CompletedProcess(cmd, 127, "", str(e))


def healthy(app=None, venv=None, runner=run):
    """Does the bundle's executable start as the venv's Python with our libraries?"""
    exe = executable(app)
    if not exe.is_file():
        return False
    r = runner([str(exe), "-c", HEALTH], env=launch_env(app, venv), timeout=60)
    return r.returncode == 0


def needs_build(app=None, python=None, venv=None):
    """-> "" (up to date) or why it must be (re)built. Only the interpreter or the layout count."""
    app = Path(app or bundle())
    if not executable(app).is_file():
        return "not built yet"
    have, want = read_stamp(app) or {}, wanted_stamp(python, venv)
    for key, why in (("format", "the bundle's layout changed"), ("python", "the Python interpreter changed"),
                     ("venv", "the Python environment moved")):
        if have.get(key) != want[key]:
            return why
    return ""


def tools_installed(runner=run):
    """Xcode command line tools present? (asking for install_name_tool without them pops up an installer)"""
    return runner(["xcode-select", "-p"], timeout=10).returncode == 0


def build(app=None, force=False, runner=run, say=None, python=None, venv=None):
    """Builds (or keeps) the bundle. -> {"built": bool, "path", "why"}. Raises BuildError: the caller
    then keeps running plain python (and says so)."""
    say = say or (lambda line: log.info("%s", line))
    app = Path(app or bundle())
    venv = Path(venv or venv_python() or "")
    if not str(venv) or not venv.exists():
        raise BuildError("the agent isn't running from a Python environment (venv)")
    python = Path(python or interpreter())
    why = "rebuild asked for" if force else needs_build(app, python, venv)
    if not why:
        if healthy(app, venv, runner):
            return {"built": False, "path": str(app), "why": "up to date"}
        why = "it didn't start"
    say(f"building {app} ({why})")
    app.parent.mkdir(parents=True, exist_ok=True)
    new = app.with_name(f".{app.name}.new")
    shutil.rmtree(new, ignore_errors=True)
    try:
        contents = new / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Resources" / "boot").mkdir(parents=True)
        exe = executable(new)
        shutil.copy2(python, exe)
        os.chmod(exe, 0o755)
        relink(exe, runner, say)
        if ICON.exists():
            shutil.copy2(ICON, contents / "Resources" / f"{NAME}.icns")
        (contents / "Resources" / "boot" / "sitecustomize.py").write_text(BOOT)
        stamp = wanted_stamp(python, venv)
        with (contents / "Info.plist").open("wb") as f:
            plistlib.dump(info_plist(stamp, app, venv), f)
        (contents / "PkgInfo").write_text("APPL????")
        r = runner(["codesign", "--force", "--deep", "--sign", "-", str(new)], timeout=120)
        if r.returncode != 0:
            raise BuildError(f"codesign failed: {(r.stderr or r.stdout).strip()[:300]}")
        if not healthy(new, venv, runner):
            raise BuildError("the new bundle's Python didn't start (import stick_agent failed)")
        old = app.with_name(f".{app.name}.old")
        shutil.rmtree(old, ignore_errors=True)
        if app.exists():
            app.rename(old)
        new.rename(app)
        shutil.rmtree(old, ignore_errors=True)
    except BuildError:
        shutil.rmtree(new, ignore_errors=True)
        raise
    except OSError as e:
        shutil.rmtree(new, ignore_errors=True)
        raise BuildError(str(e)) from e
    say(f"built {app} (Python {stamp['version']}, signed ad hoc)")
    return {"built": True, "path": str(app), "why": why}


def relink(exe, runner=run, say=None):
    """A framework build (Homebrew) links its versioned Cellar path; point the copy at the stable
    opt/ path so a `brew upgrade` doesn't break it before the next rebuild. Best effort."""
    if not sysconfig.get_config_var("PYTHONFRAMEWORK"):
        return
    base = Path(sys.base_prefix)
    real = base.resolve()
    if real == base or not (base / "Python").exists() or not tools_installed(runner):
        return
    r = runner(["install_name_tool", "-change", str(real / "Python"), str(base / "Python"), str(exe)])
    if r.returncode != 0 and say:
        say(f"kept the versioned Python path ({(r.stderr or '').strip()[:120]})")


def remove(app=None, say=None):
    app = Path(app or bundle())
    if app.exists():
        shutil.rmtree(app)
        if say:
            say(f"removed {app}")


# --- running as the app ------------------------------------------------------------------

@functools.cache
def process_path():
    """This process's executable (proc_pidpath), or ""."""
    try:
        libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
        buf = ctypes.create_string_buffer(4096)
        n = libc.proc_pidpath(os.getpid(), buf, ctypes.c_uint32(4096))
        return buf.value[:n].decode() if n > 0 else ""
    except (OSError, AttributeError):
        return ""


def running_as_app():
    """Is this process the bundle's executable (so macOS knows it as StickyBot)?"""
    return process_path().endswith(f".app/Contents/MacOS/{NAME}")


def display_name():
    """What macOS calls this process in its permission prompts and lists."""
    return NAME if running_as_app() else "Python"


def restart_argv():
    """(executable, argv) to start the agent again in this process, keeping the app identity."""
    if running_as_app():
        exe = process_path()
        venv = venv_python()
        if venv:
            os.environ["__PYVENV_LAUNCHER__"] = str(venv)  # CPython removed it at start-up
        return exe, [exe, "-m", "stick_agent", *sys.argv[1:]]
    return sys.executable, [sys.executable, *sys.argv]


def login_program():
    """The login item's program: the app's executable if it's built and starts, else plain python."""
    exe = executable()
    if os.environ.get("STICK_AGENT_NO_APP") != "1" and exe.is_file() and not needs_build() and healthy():
        return [str(exe), "-m", "stick_agent", "run"], launch_env()
    return [sys.executable, "-m", "stick_agent", "run"], {}


def ensure(say=None, runner=run):
    """Build the app if it's missing or out of date. -> (ok, message); never raises."""
    if os.environ.get("STICK_AGENT_NO_APP") == "1":
        return False, "the app bundle is skipped (STICK_AGENT_NO_APP=1): the agent runs as Python"
    try:
        r = build(runner=runner, say=say)
    except BuildError as e:
        return False, f"couldn't build {bundle()} ({e}) — the agent keeps running as Python"
    if not r["built"]:
        note = "up to date"
    elif r["why"] == "not built yet":
        note = "built: macOS asks once for Bluetooth etc. for StickyBot"
    else:
        note = f"rebuilt, {r['why']}: macOS may ask for its permissions again"
    return True, f"{NAME}.app: {r['path']} ({note})"


def opened():
    """The bundle was opened from Finder / Spotlight / the Dock and the agent isn't running
    (a running one gets a "reopen" instead, see window.py): start it (the login item), then show
    its window. Never returns."""
    import time
    import urllib.request
    try:
        from . import __main__ as cli
        up = lambda: _answers(urllib.request)
        if not up():
            if cli.launchctl("kickstart", f"{cli.domain()}/{cli.LABEL}").returncode != 0:
                cli.start()
            for _ in range(60):
                if up():
                    break
                time.sleep(0.5)
        if not request_window():
            subprocess.run(["open", f"http://localhost:{config.PORT}/"], capture_output=True)
    except Exception:  # noqa: BLE001 — nothing to show it in
        log.exception("opening StickyBot failed")
    finally:
        sys.stdout.flush()
        os._exit(0)


def _answers(urllib_request):
    try:
        with urllib_request.urlopen(f"http://127.0.0.1:{config.PORT}/api/status", timeout=2):
            return True
    except OSError:
        return False


def request_window(page=""):
    """Asks the running agent to show its window (on `page`, e.g. "install"). -> True if it did."""
    import json
    import urllib.request
    try:
        req = urllib.request.Request(f"http://127.0.0.1:{config.PORT}/api/window", method="POST",
                                     data=json.dumps({"page": page}).encode(),
                                     headers={"X-Stick-Token": config.token(), "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=5) as r:
            return bool(json.load(r).get("ok"))
    except (OSError, ValueError):
        return False
