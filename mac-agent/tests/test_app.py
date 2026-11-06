"""StickyBot.app (app.py): what the builder writes and runs (codesign, the health check) with a fake
runner, when it rebuilds (only when the Python changes), the fallback to plain python, the login
item pointing at the bundle, restarts that keep the app identity, and the dashboard window's
callbacks (window.py) never raising into Objective-C. Nothing is signed or started for real."""
import os
import plistlib
import subprocess
import sys
from pathlib import Path

import pytest

from stick_agent import __main__ as cli
from stick_agent import app, config, server, window

HOST = {"Host": f"localhost:{config.PORT}"}


class Runner:
    """app.run stand-in: records every command; codesign / the health check can fail."""

    def __init__(self, codesign=0, health=0):
        self.codesign, self.health = codesign, health
        self.ran = []

    def __call__(self, cmd, env=None, timeout=60):
        self.ran.append((cmd, env))
        if cmd[0] == "codesign":
            return subprocess.CompletedProcess(cmd, self.codesign, "", "" if not self.codesign else "no identity")
        if cmd[1:3] == ["-c", app.HEALTH]:
            return subprocess.CompletedProcess(cmd, self.health, "", "")
        return subprocess.CompletedProcess(cmd, 0, "", "")

    def cmds(self, name):
        return [c for c, _ in self.ran if c[0] == name or c[0].endswith("/" + name)]


@pytest.fixture
def py(tmp_path, monkeypatch):
    """A fake interpreter binary and venv (no framework build: nothing to relink)."""
    monkeypatch.setattr(app.sysconfig, "get_config_var", lambda name: None)
    python = tmp_path / "python3.14"
    python.write_bytes(b"\xcf\xfa\xed\xfe fake interpreter v1")
    venv = tmp_path / "venv" / "bin" / "python"
    venv.parent.mkdir(parents=True)
    venv.write_text("")
    return python, venv


def build(tmp_path, py, runner, **kw):
    python, venv = py
    return app.build(app=tmp_path / "Apps" / "StickyBot.app", runner=runner, python=python, venv=venv, **kw)


def test_builds_a_signed_bundle_around_a_copy_of_the_interpreter(tmp_path, py):
    runner = Runner()
    r = build(tmp_path, py, runner)
    bundle = tmp_path / "Apps" / "StickyBot.app"
    assert r == {"built": True, "path": str(bundle), "why": "not built yet"}
    exe = bundle / "Contents" / "MacOS" / "StickyBot"
    assert exe.read_bytes() == py[0].read_bytes() and os.access(exe, os.X_OK)  # the real binary, copied
    info = plistlib.loads((bundle / "Contents" / "Info.plist").read_bytes())
    assert info["CFBundleIdentifier"] == "com.stickybot.app" == app.BUNDLE_ID
    assert info["CFBundleName"] == info["CFBundleDisplayName"] == info["CFBundleExecutable"] == "StickyBot"
    assert info["CFBundleIconFile"] == "StickyBot" and info["LSUIElement"] is True
    assert "Bluetooth" in info["NSBluetoothAlwaysUsageDescription"]
    assert info["NSAudioCaptureUsageDescription"] == \
        "StickyBot taps only its own sound to make the stick's Bluetooth mic."
    assert info["LSEnvironment"]["__PYVENV_LAUNCHER__"] == str(py[1])  # stdlib + the venv's site-packages
    assert info["LSEnvironment"]["PYTHONPATH"] == str(bundle / "Contents" / "Resources" / "boot")
    assert info["StickyBotBuild"]["python"] == app.sha256(py[0]) and info["StickyBotBuild"]["format"] == app.FORMAT
    assert (bundle / "Contents" / "Resources" / "StickyBot.icns").read_bytes() == app.ICON.read_bytes()
    assert "app.opened()" in (bundle / "Contents" / "Resources" / "boot" / "sitecustomize.py").read_text()
    # signed ad hoc (the staging copy, then moved into place), then checked that it starts as the venv's Python
    assert runner.cmds("codesign") == [["codesign", "--force", "--deep", "--sign", "-",
                                        str(bundle.with_name(".StickyBot.app.new"))]]
    health = [(c, env) for c, env in runner.ran if c[1:3] == ["-c", app.HEALTH]]
    assert health and health[-1][1]["__PYVENV_LAUNCHER__"] == str(py[1])
    assert not bundle.with_name(".StickyBot.app.new").exists()


def test_rebuilds_only_when_the_python_changes(tmp_path, py):
    build(tmp_path, py, Runner())
    runner = Runner()
    assert build(tmp_path, py, runner)["built"] is False  # code updates (.py) never touch it
    assert runner.cmds("codesign") == []
    py[0].write_bytes(b"\xcf\xfa\xed\xfe fake interpreter v2")  # a new Python
    r = build(tmp_path, py, runner)
    assert r["built"] and r["why"] == "the Python interpreter changed"


def test_a_new_bundle_format_rebuilds(tmp_path, py, monkeypatch):
    build(tmp_path, py, Runner())
    monkeypatch.setattr(app, "FORMAT", app.FORMAT + 1)
    assert build(tmp_path, py, Runner())["why"] == "the bundle's layout changed"


def test_one_that_does_not_start_is_rebuilt(tmp_path, py):
    build(tmp_path, py, Runner())
    runner = Runner()
    bundle = tmp_path / "Apps" / "StickyBot.app"
    calls = {"n": 0}

    def flaky(cmd, env=None, timeout=60):
        if cmd[1:3] == ["-c", app.HEALTH]:
            calls["n"] += 1
            return subprocess.CompletedProcess(cmd, 1 if calls["n"] == 1 else 0, "", "")
        return runner(cmd, env, timeout)
    r = app.build(app=bundle, runner=flaky, python=py[0], venv=py[1])
    assert r["built"] and r["why"] == "it didn't start"


def test_failed_signing_keeps_the_old_bundle(tmp_path, py):
    build(tmp_path, py, Runner())
    bundle = tmp_path / "Apps" / "StickyBot.app"
    before = (bundle / "Contents" / "Info.plist").read_bytes()
    with pytest.raises(app.BuildError, match="codesign failed"):
        build(tmp_path, py, Runner(codesign=1), force=True)
    assert (bundle / "Contents" / "Info.plist").read_bytes() == before
    assert not bundle.with_name(".StickyBot.app.new").exists()


def test_a_bundle_that_does_not_start_is_not_installed(tmp_path, py):
    with pytest.raises(app.BuildError, match="didn't start"):
        build(tmp_path, py, Runner(health=1))
    assert not (tmp_path / "Apps" / "StickyBot.app").exists()


def test_ensure_falls_back_to_python_and_says_so(monkeypatch):
    def fail(**_):
        raise app.BuildError("codesign failed: no")
    monkeypatch.setattr(app, "build", fail)
    ok, msg = app.ensure()
    assert not ok and "keeps running as Python" in msg
    monkeypatch.setenv("STICK_AGENT_NO_APP", "1")
    assert app.ensure()[0] is False


def test_framework_python_is_relinked_to_the_stable_path(tmp_path, monkeypatch):
    """Homebrew: the copy links .../Cellar/python@3.14/3.14.5/...; opt/ survives `brew upgrade`."""
    cellar = tmp_path / "Cellar" / "3.14.5" / "Versions" / "3.14"
    cellar.mkdir(parents=True)
    (cellar / "Python").write_text("")
    opt = tmp_path / "opt" / "Versions"
    opt.mkdir(parents=True)
    (opt / "3.14").symlink_to(cellar)
    monkeypatch.setattr(app.sysconfig, "get_config_var", lambda name: "Python" if name == "PYTHONFRAMEWORK" else None)
    monkeypatch.setattr(sys, "base_prefix", str(opt / "3.14"))
    runner = Runner()
    app.relink(tmp_path / "exe", runner)
    assert runner.cmds("install_name_tool") == [["install_name_tool", "-change", str(cellar / "Python"),
                                                 str(opt / "3.14" / "Python"), str(tmp_path / "exe")]]
    # the interpreter to copy is Python.app's (bin/python3.X only spawns it: that's the "Python" identity)
    real = cellar / "Resources" / "Python.app" / "Contents" / "MacOS" / "Python"
    real.parent.mkdir(parents=True)
    real.write_text("")
    assert app.interpreter() == real.resolve()


# --- the login item ------------------------------------------------------------------------

@pytest.fixture
def plist(tmp_path, monkeypatch):
    path = tmp_path / "LaunchAgents" / "com.stickkey.agent.test.plist"
    path.parent.mkdir()
    monkeypatch.setattr(cli, "plist_path", lambda: path)
    return path


def test_the_login_item_runs_the_app(plist, monkeypatch):
    exe = "/Users/me/Applications/StickyBot.app/Contents/MacOS/StickyBot"
    env = {"__PYVENV_LAUNCHER__": "/Users/me/.stick-agent/venv/bin/python", "STICKYBOT_APP": exe}
    monkeypatch.setattr(app, "login_program", lambda: ([exe, "-m", "stick_agent", "run"], env))
    p = plistlib.loads(cli.write_plist().read_bytes())
    assert p["ProgramArguments"] == [exe, "-m", "stick_agent", "run"]  # launchd starts it directly
    assert p["EnvironmentVariables"]["__PYVENV_LAUNCHER__"] == env["__PYVENV_LAUNCHER__"]
    assert p["AssociatedBundleIdentifiers"] == ["com.stickybot.app"]
    assert p["KeepAlive"] == {"SuccessfulExit": False} and p["Label"] == cli.LABEL


def test_without_the_app_the_login_item_runs_python(plist, monkeypatch):
    # conftest's STICK_AGENT_APP has no bundle in it
    p = plistlib.loads(cli.write_plist().read_bytes())
    assert p["ProgramArguments"] == [sys.executable, "-m", "stick_agent", "run"]
    assert "AssociatedBundleIdentifiers" not in p and "__PYVENV_LAUNCHER__" not in p["EnvironmentVariables"]


def test_login_program_uses_a_built_healthy_bundle(monkeypatch, tmp_path):
    bundle = tmp_path / "StickyBot.app"
    monkeypatch.setenv("STICK_AGENT_APP", str(bundle))
    exe = app.executable()
    exe.parent.mkdir(parents=True)
    exe.write_text("")
    monkeypatch.setattr(app, "needs_build", lambda: "")
    monkeypatch.setattr(app, "healthy", lambda: True)
    program, env = app.login_program()
    assert program == [str(exe), "-m", "stick_agent", "run"] and env["STICKYBOT_APP"] == str(exe)
    monkeypatch.setattr(app, "healthy", lambda: False)  # broken: plain python, never a crash loop
    assert app.login_program()[0][0] == sys.executable


def test_the_login_item_switches_to_the_app_through_a_detached_helper(plist, monkeypatch):
    plist.write_bytes(plistlib.dumps({"ProgramArguments": [sys.executable, "-m", "stick_agent", "run"]}))
    monkeypatch.setattr(cli, "LOGIN_PLIST", plist)
    monkeypatch.setattr(app, "ensure", lambda say=None: (True, "built"))
    exe = "/A/StickyBot.app/Contents/MacOS/StickyBot"
    monkeypatch.setattr(app, "login_program", lambda: ([exe, "-m", "stick_agent", "run"], {"STICKYBOT_APP": exe}))
    spawned = []
    monkeypatch.setattr(cli.subprocess, "Popen", lambda cmd, **kw: spawned.append((cmd, kw)))
    cli.switch_to_app()
    cmd, kw = spawned[0]
    assert kw["start_new_session"] is True  # launchd stops the job's process group, not this helper
    assert "launchctl bootout" in cmd[2] and "launchctl bootstrap" in cmd[2] and cmd[-1] == str(plist)
    assert plistlib.loads(plist.read_bytes())["ProgramArguments"][0] == exe
    spawned.clear()
    cli.switch_to_app()  # already the app: nothing to do
    assert spawned == []


def test_a_failed_build_leaves_the_login_item_alone(plist, monkeypatch):
    plist.write_bytes(plistlib.dumps({"ProgramArguments": [sys.executable]}))
    monkeypatch.setattr(cli, "LOGIN_PLIST", plist)
    monkeypatch.setattr(app, "ensure", lambda say=None: (False, "couldn't build it"))
    monkeypatch.setattr(cli.subprocess, "Popen", lambda *a, **kw: pytest.fail("restarted"))
    cli.switch_to_app()
    assert plistlib.loads(plist.read_bytes())["ProgramArguments"] == [sys.executable]


# --- running as the app --------------------------------------------------------------------

def test_a_restart_keeps_the_app_identity(monkeypatch):
    exe = "/A/StickyBot.app/Contents/MacOS/StickyBot"
    monkeypatch.setattr(app, "process_path", lambda: exe)
    monkeypatch.setattr(sys, "argv", ["/x/stick_agent/__main__.py", "run"])
    monkeypatch.delenv("__PYVENV_LAUNCHER__", raising=False)
    venv = Path("/home/.stick-agent/venv/bin/python")  # as installed (the release build runs outside a venv)
    monkeypatch.setattr(app, "venv_python", lambda: venv)
    assert app.running_as_app() and app.display_name() == "StickyBot"
    got_exe, argv = app.restart_argv()
    assert got_exe == exe and argv == [exe, "-m", "stick_agent", "run"]
    assert os.environ["__PYVENV_LAUNCHER__"] == str(venv)  # CPython dropped it at start-up
    monkeypatch.setattr(app, "process_path", lambda: sys.executable)
    assert app.display_name() == "Python"
    assert app.restart_argv() == (sys.executable, [sys.executable, "/x/stick_agent/__main__.py", "run"])


def test_app_status_command(monkeypatch, capsys):
    monkeypatch.setattr("sys.argv", ["stickybot", "app"])
    with pytest.raises(SystemExit) as e:
        cli.main()
    assert e.value.code == 0 and "not built yet" in capsys.readouterr().out


def test_uninstall_removes_the_bundle(tmp_path):
    bundle = tmp_path / "StickyBot.app"
    (bundle / "Contents").mkdir(parents=True)
    said = []
    app.remove(bundle, say=said.append)
    assert not bundle.exists() and said == [f"removed {bundle}"]


# --- the window and its API ------------------------------------------------------------------

def test_navigation_stays_on_the_dashboard_and_sends_the_rest_out():
    opened = []
    assert window.navigation_policy(f"http://localhost:{config.PORT}/#settings", opened.append) == window.ALLOW
    assert window.navigation_policy("about:blank", opened.append) == window.ALLOW
    assert window.navigation_policy("https://github.com/x", opened.append) == window.CANCEL
    assert window.navigation_policy("http://localhost:9999/", opened.append) == window.CANCEL
    assert opened == ["https://github.com/x", "http://localhost:9999/"]


def test_window_callbacks_never_raise_into_objective_c():
    """An exception that reaches Objective-C aborts the process: every callback is guarded."""
    def boom(*_):
        raise RuntimeError("boom")
    assert window.navigation_policy("https://example.com/", boom) == window.CANCEL  # opening failed: still answers
    answers = []
    window.answer_once("nav", answers.append, boom, window.CANCEL)
    assert answers == [window.CANCEL]  # the handler is called exactly once, with the fallback
    window.answer_once("nav", boom, lambda: window.ALLOW, window.CANCEL)  # a raising handler: swallowed
    done = []
    window.answer_once("alert", lambda *a: done.append(a), lambda: None, None)
    assert done == [()]  # an alert's handler takes no value
    assert window.guarded("x", boom, fallback="fb") == "fb"


@pytest.fixture
async def client(agent, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.agent = agent
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def test_api_window(client):
    r = await client.post("/api/window", headers=client.auth, json={"page": "install"})
    assert (await r.json()) == {"ok": False}  # no window here (no menu bar): the caller opens a browser
    shown = []
    client.agent.window = lambda page: shown.append(page) or True
    r = await client.post("/api/window", headers=client.auth, json={"page": "install"})
    assert (await r.json()) == {"ok": True} and shown == ["install"]
    r = await client.post("/api/window", headers=client.auth, json={"page": "x\"; alert(1)"})
    assert (await r.json()) == {"ok": False} and shown == ["install"]
    r = await client.post("/api/window", headers=HOST, json={})
    assert r.status == 403  # needs the token like every other change


async def test_status_says_how_macos_knows_us(client):
    s = await (await client.get("/api/status", headers=HOST)).json()
    assert s["app"]["name"] in ("StickyBot", "Python") and s["app"]["window"] is False


def test_the_icon_ships_with_the_package():
    assert app.ICON.exists() and app.ICON.read_bytes()[:4] == b"icns"
    assert Path(app.__file__).parent / "icon" == app.ICON.parent
