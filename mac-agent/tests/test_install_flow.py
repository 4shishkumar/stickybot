"""The setup page's API (/api/install), the gh one-time-code login, the pinned BlackHole
install (download, sha256, signature, the admin dialog's command) and the model download's
progress. Nothing real is run: no gh login, no osascript, no installer, no downloads."""
import io
import json
import plistlib
import shlex
import subprocess
import threading
import time

import pytest

from stick_agent import config, install_flow, installer, server

HOST = {"Host": f"localhost:{config.PORT}"}
ORDER = ["welcome", "model", "claude", "hooks", "mcp", "github", "wispr", "stick", "done"]


@pytest.fixture
def sandbox(home, tmp_path, monkeypatch):
    monkeypatch.setenv("STICK_AGENT_BIN", str(tmp_path / "bin"))
    monkeypatch.setenv("STICK_AGENT_CLAUDE_SETTINGS", str(tmp_path / "claude" / "settings.json"))
    monkeypatch.setenv("STICK_AGENT_NO_LOGIN_ITEM", "1")
    monkeypatch.setattr(installer.Model, "_cached", lambda self: False)
    monkeypatch.setattr(installer.GitHub, "_gh", lambda self: None)  # never the real gh
    monkeypatch.setattr(installer, "brew_path", lambda: None)
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    monkeypatch.setattr(installer, "model_cached_bytes", lambda repo: 0)
    return tmp_path


@pytest.fixture
async def client(agent, sandbox, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def events_of(client, job):
    r = await client.get(f"/api/install/jobs/{job}", headers=HOST)
    return [json.loads(b.removeprefix("data: ")) for b in (await r.text()).strip().split("\n\n")
            if b.startswith("data: ")]


# --- the listing, skip / unskip ------------------------------------------------------------

async def test_steps_come_in_order_with_states(client):
    r = await client.get("/api/install", headers=HOST)
    assert r.status == 200
    data = await r.json()
    assert [s["id"] for s in data["steps"]] == ORDER
    st = {s["id"]: s for s in data["steps"]}
    assert st["welcome"]["state"] == "done" and st["welcome"]["data"]["checks"]
    assert st["model"]["state"] == "todo" and st["model"]["kind"] == "recommended"
    assert st["github"]["state"] == "todo" and st["github"]["data"]["gh"] is False
    assert st["wispr"]["data"]["pinned"] == "0.7.1"
    assert st["hooks"]["data"]["preview"]["hooks"].keys() == {"PermissionRequest", "UserPromptSubmit", "Stop"}
    assert data["next"] == "model" and data["pending"] is True
    assert all(s["state"] in ("done", "todo", "skipped", "unavailable") for s in data["steps"])


async def test_skip_and_unskip_are_kept_in_the_config(client, agent):
    r = await client.post("/api/install/model/skip", headers=client.auth)
    data = await r.json()
    assert {s["id"]: s["state"] for s in data["steps"]}["model"] == "skipped"
    assert data["next"] != "model" and data["skipped"] == ["model"]
    assert config.load()["install_skipped"] == ["model"] and agent.cfg["install_skipped"] == ["model"]
    await client.post("/api/install/model/skip", headers=client.auth)  # twice: still once
    assert config.load()["install_skipped"] == ["model"]
    r = await client.post("/api/install/model/unskip", headers=client.auth)
    assert {s["id"]: s["state"] for s in (await r.json())["steps"]}["model"] == "todo"
    assert config.load()["install_skipped"] == []


async def test_pending_ignores_skipped_recommended_steps(client, agent):
    agent.cfg["stick"] = None
    for sid in ("model", "stick"):
        await client.post(f"/api/install/{sid}/skip", headers=client.auth)
    data = await (await client.get("/api/install", headers=HOST)).json()
    assert data["pending"] is False


async def test_unknown_steps_and_bad_actions(client):
    assert (await client.get("/api/install/nope", headers=HOST)).status == 404
    assert (await client.post("/api/install/nope/skip", headers=client.auth)).status == 404
    assert (await client.post("/api/install/nope/run", headers=client.auth)).status == 404
    assert (await client.post("/api/install/welcome/skip", headers=client.auth)).status == 400
    assert (await client.post("/api/install/stick/run", headers=client.auth)).status == 400
    assert (await client.get("/api/install/jobs/nope", headers=HOST)).status == 404


async def test_changes_need_the_token(client):
    for path in ("/api/install/model/skip", "/api/install/hooks/run", "/api/install/github/repo"):
        r = await client.post(path, headers=HOST)
        assert r.status == 403, path
    assert config.load()["install_skipped"] == []


async def test_one_step(client):
    r = await client.get("/api/install/hooks", headers=HOST)
    s = await r.json()
    assert s["id"] == "hooks" and s["state"] == "todo" and s["runnable"] and s["skippable"]


# --- running steps --------------------------------------------------------------------------

async def test_hooks_step_adds_the_hooks_only_when_asked(client, sandbox):
    settings = sandbox / "claude" / "settings.json"
    await client.get("/api/install", headers=HOST)
    assert not settings.exists()  # looking never changes anything
    r = await client.post("/api/install/hooks/run", headers=client.auth)
    events = await events_of(client, (await r.json())["job"])
    assert events[-1] == {"done": True, "ok": True, "error": None}
    assert installer.hooks_state(json.loads(settings.read_text())) == (3, 3)
    s = await (await client.get("/api/install/hooks", headers=HOST)).json()
    assert s["state"] == "done"


async def test_mcp_needs_claude(client, monkeypatch):
    monkeypatch.setattr(config, "claude_path", lambda cfg: None)
    s = await (await client.get("/api/install/mcp", headers=HOST)).json()
    assert s["state"] == "unavailable"


async def test_repo_is_validated_and_sets_alerts_and_quick_commands(client, agent):
    for bad in ("", "acme", "acme/web/x", "../etc", "acme/..", "a b/c", "acme/web;rm"):
        r = await client.post("/api/install/github/repo", json={"repo": bad}, headers=client.auth)
        assert r.status == 400, bad
    r = await client.post("/api/install/github/repo", json={"repo": "acme/web", "alerts": True}, headers=client.auth)
    assert (await r.json())["ok"]
    assert agent.cfg["github"]["primary_repo"] == "acme/web" and agent.cfg["github"]["on"] is True
    urls = [c.get("open_url", "") for c in json.loads(config.COMMANDS.read_text())["commands"]]
    assert "https://github.com/acme/web/pull/{n}" in urls
    assert "https://app.devin.ai/review/acme/web/pull/{n}" in urls
    assert not any("<org>" in u for u in urls)


def test_set_primary_repo_keeps_alerts_when_asked(home):
    cfg = config.load()
    cfg["github"] = {"on": False, "primary_repo": "", "interval_s": 300}
    config.save(cfg)
    g = installer.set_primary_repo("acme/app")
    assert g == {"on": False, "primary_repo": "acme/app", "interval_s": 300}
    installer.set_primary_repo("other/app", alerts=True)
    urls = [c.get("open_url", "") for c in json.loads(config.COMMANDS.read_text())["commands"]]
    assert "https://github.com/other/app/pull/{n}" in urls  # a second change replaces the first
    assert config.load()["github"]["on"] is True


# --- the gh login (one-time code) -------------------------------------------------------------

GH_PIPE = """! First copy your one-time code: 1A2B-3C4D
Open this URL to continue in your web browser: https://github.com/login/device
"""
GH_TTY = "\x1b[0;33m!\x1b[0m First copy your one-time code: \x1b[1mWXYZ-0987\x1b[0m\n" \
         "\x1b[1mPress Enter\x1b[0m to open https://github.com/login/device in your browser... "


def test_device_code_parsing():
    assert installer.parse_device_code(GH_PIPE) == ("1A2B-3C4D", "https://github.com/login/device")
    assert installer.parse_device_code(GH_TTY) == ("WXYZ-0987", "https://github.com/login/device")
    assert installer.parse_device_code("! First copy your one-time code: 1A2B") == (None, None)
    assert installer.parse_device_code("") == (None, None)
    assert installer.gh_login_args("gh") == ["gh", "auth", "login", "--hostname", "github.com",
                                             "--git-protocol", "https", "--web"]


def fake_gh(tmp_path, needs_enter=False):
    """A `gh` that 'logs in' a moment after printing its code (and waits for Enter if asked)."""
    marker = tmp_path / "gh-logged-in"
    script = tmp_path / "gh"
    wait = "read _line" if needs_enter else ":"
    script.write_text(f"""#!/bin/sh
if [ "$1 $2" = "auth status" ]; then [ -f {marker} ]; exit $?; fi
if [ "$1 $2" = "auth login" ]; then
  [ "$GH_PROMPT_DISABLED" = 1 ] || exit 9
  printf '! First copy your one-time code: QWER-7890\\n'
  printf 'Press Enter to open https://github.com/login/device in your browser... '
  {wait}
  sleep 0.2
  printf '\\n✓ Authentication complete.\\n'
  touch {marker}
  exit 0
fi
exit 1
""")
    script.chmod(0o755)
    return str(script)


@pytest.mark.parametrize("needs_enter", [False, True])
async def test_gh_login_job_shows_the_code_and_finishes(client, tmp_path, monkeypatch, needs_enter):
    gh = fake_gh(tmp_path, needs_enter)
    monkeypatch.setattr(installer.GitHub, "_gh", lambda self: gh)
    r = await client.post("/api/install/github/run", headers=client.auth)
    events = await events_of(client, (await r.json())["job"])
    assert {"code": "QWER-7890", "url": "https://github.com/login/device"} in events
    assert not any("QWER-7890" in e.get("line", "") for e in events)  # shown big, not in the log
    assert events[-1]["done"] and events[-1]["ok"], events
    s = await (await client.get("/api/install/github", headers=HOST)).json()
    assert s["data"]["logged_in"] and s["data"]["login"] is None and s["state"] == "todo"  # next: the repo


async def test_gh_login_that_fails(client, tmp_path, monkeypatch):
    script = tmp_path / "gh"
    script.write_text("#!/bin/sh\necho 'error connecting to github.com'\nexit 1\n")
    script.chmod(0o755)
    monkeypatch.setattr(installer.GitHub, "_gh", lambda self: str(script))
    r = await client.post("/api/install/github/run", headers=client.auth)
    last = (await events_of(client, (await r.json())["job"]))[-1]
    assert last["done"] and not last["ok"] and "gh auth login" in last["error"]


# --- BlackHole: pinned, checked, the macOS dialog ---------------------------------------------

SIG_GOOD = """Package "BlackHole2ch-0.7.1.pkg":
   Status: signed by a developer certificate issued by Apple for distribution
   Notarization: trusted by the Apple notary service
   Signed with a trusted timestamp on: 2026-07-03 18:01:42 +0000
   Certificate Chain:
    1. Developer ID Installer: Existential Audio Inc. (Q5C99V536K)
       Expires: 2027-02-01 22:12:15 +0000
    2. Developer ID Certification Authority
    3. Apple Root CA
"""


def test_signature_check():
    assert installer.signature_ok(SIG_GOOD) == (True, "")
    assert not installer.signature_ok(SIG_GOOD.replace("Existential Audio Inc. (Q5C99V536K)", "Evil Ltd (X1)"))[0]
    assert not installer.signature_ok(SIG_GOOD.replace("Notarization: trusted by the Apple notary service\n", ""))[0]
    assert not installer.signature_ok('Package "x.pkg":\n   Status: no signature\n')[0]
    assert not installer.signature_ok("")[0]


def test_the_pin():
    pin = installer.BLACKHOLE_PIN
    assert pin["url"].startswith("https://existential.audio/") and pin["version"] in pin["url"]
    assert len(pin["sha256"]) == 64


class FakeResponse(io.BytesIO):
    pass


def opener_for(payload):
    return lambda url: FakeResponse(payload)


def test_download_checks_the_sha256(tmp_path):
    import hashlib
    data = b"pkg bytes"
    good = hashlib.sha256(data).hexdigest()
    dest = tmp_path / "a.pkg"
    installer.download_verified("https://example.test/a.pkg", good, dest, opener=opener_for(data))
    assert dest.read_bytes() == data
    with pytest.raises(installer.NotAvailable, match="checksum"):
        installer.download_verified("https://example.test/a.pkg", "0" * 64, dest, opener=opener_for(data))
    assert not dest.exists()  # a bad download is deleted
    with pytest.raises(installer.NotAvailable, match="HTTPS"):
        installer.download_verified("http://example.test/a.pkg", good, dest, opener=opener_for(data))


def test_admin_command_quotes_the_path():
    tricky = "/tmp/it's a \"pkg\" $(touch x)/BlackHole.pkg"
    argv = installer.admin_install_args(tricky)
    assert argv[:2] == ["/usr/bin/osascript", "-e"] and len(argv) == 3
    script = argv[2]
    assert script.startswith('do shell script "') and "with administrator privileges" in script
    # undo the AppleScript string literal, then check the shell sees the path as one word
    literal = script[len("do shell script "):script.index(" with administrator privileges")]
    assert literal[0] == literal[-1] == '"'
    sh = literal[1:-1].replace('\\"', '"').replace("\\\\", "\\")
    words = shlex.split(sh)
    assert words[:4] == ["/usr/sbin/installer", "-pkg", tricky, "-target"]
    assert "killall" in sh and "coreaudiod" in sh  # same call: one password prompt
    assert "password" not in sh.lower()


class Runner:
    def __init__(self, sig=SIG_GOOD, osa_rc=0, osa_err="", on_install=None):
        self.calls, self.sig, self.osa_rc, self.osa_err, self.on_install = [], sig, osa_rc, osa_err, on_install

    def __call__(self, argv, timeout=30, **kw):
        self.calls.append(argv)
        if argv[0].endswith("pkgutil"):
            return subprocess.CompletedProcess(argv, 0, self.sig, "")
        if argv[0].endswith("osascript"):
            if self.osa_rc == 0 and self.on_install:
                self.on_install()
            return subprocess.CompletedProcess(argv, self.osa_rc, "", self.osa_err)
        raise AssertionError(argv)


@pytest.fixture
def pinned(monkeypatch):
    import hashlib
    data = b"the pinned pkg"
    monkeypatch.setitem(installer.BLACKHOLE_PIN, "sha256", hashlib.sha256(data).hexdigest())
    monkeypatch.setattr(installer.time, "sleep", lambda s: None)
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)  # (whatever this Mac has)
    return opener_for(data)


def test_install_blackhole_happy_path(home, tmp_path, monkeypatch, pinned):
    state = {"installed": False}
    monkeypatch.setattr(installer, "blackhole_installed", lambda: state["installed"])
    monkeypatch.setattr(installer, "blackhole_version", lambda: "0.7.1")
    runner = Runner(on_install=lambda: state.update(installed=True))
    lines = []
    installer.install_blackhole(installer.UI(emit=lines.append), runner=runner, opener=pinned, workdir=tmp_path)
    assert [c[0] for c in runner.calls] == ["/usr/sbin/pkgutil", "/usr/bin/osascript"]
    assert "-pkg" in runner.calls[1][2]
    assert any("0.7.1 installed" in e.get("line", "") for e in lines)
    assert not list(tmp_path.glob("blackhole-*"))  # the download is cleaned up


def test_install_blackhole_refuses_a_bad_signature(home, tmp_path, monkeypatch, pinned):
    runner = Runner(sig=SIG_GOOD.replace("Existential Audio Inc. (Q5C99V536K)", "Someone Else (AAAA)"))
    with pytest.raises(installer.NotAvailable, match="not installing"):
        installer.install_blackhole(installer.UI(emit=lambda e: None), runner=runner, opener=pinned, workdir=tmp_path)
    assert [c[0] for c in runner.calls] == ["/usr/sbin/pkgutil"]  # never the admin dialog


def test_install_blackhole_refuses_a_wrong_checksum(home, tmp_path, monkeypatch, pinned):
    runner = Runner()
    with pytest.raises(installer.NotAvailable, match="checksum"):
        installer.install_blackhole(installer.UI(emit=lambda e: None), runner=runner,
                                    opener=opener_for(b"something else"), workdir=tmp_path)
    assert runner.calls == []


def test_install_blackhole_cancelled_dialog(home, tmp_path, pinned):
    runner = Runner(osa_rc=1, osa_err="execution error: User canceled. (-128)")
    with pytest.raises(installer.NotAvailable, match="cancelled"):
        installer.install_blackhole(installer.UI(emit=lambda e: None), runner=runner, opener=pinned, workdir=tmp_path)


def test_blackhole_already_installed_other_version(home, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: True)
    monkeypatch.setattr(installer, "blackhole_version", lambda: "0.6.0")
    runner = Runner()
    lines = []
    installer.install_blackhole(installer.UI(emit=lines.append), runner=runner)
    assert runner.calls == []  # no reinstall, no downgrade
    assert lines[0]["line"] == "BlackHole 0.6.0 found (tested with 0.7.1)"


def test_blackhole_version_from_the_driver(tmp_path, monkeypatch):
    drv = tmp_path / "BlackHole2ch.driver"
    (drv / "Contents").mkdir(parents=True)
    with (drv / "Contents/Info.plist").open("wb") as f:
        plistlib.dump({"CFBundleShortVersionString": "0.7.1"}, f)
    monkeypatch.setattr(installer, "BLACKHOLE_DRIVERS", [tmp_path / "missing.driver", drv])
    assert installer.blackhole_version() == "0.7.1"
    monkeypatch.setattr(installer, "BLACKHOLE_DRIVERS", [tmp_path / "missing.driver"])
    assert installer.blackhole_version() is None


async def test_wispr_step_turns_the_mic_on(client, agent, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: True)
    monkeypatch.setattr(installer, "blackhole_version", lambda: "0.7.1")
    r = await client.post("/api/install/wispr/run", headers=client.auth)
    events = await events_of(client, (await r.json())["job"])
    assert events[-1]["ok"], events
    assert agent.cfg["wireless_mic"] is True
    s = await (await client.get("/api/install/wispr", headers=HOST)).json()
    assert s["state"] == "done" and "BlackHole 0.7.1 found" in s["detail"]


# --- the speech model's progress --------------------------------------------------------------

def test_progress_text():
    assert installer.progress_text(1.2e9, 2.5e9, 0) == "1.2 GB of 2.5 GB"
    assert installer.progress_text(1.2e9, 2.5e9, 13e6) == "1.2 GB of 2.5 GB · 13 MB/s · about 2 min left"
    assert installer.progress_text(2.4e9, 2.5e9, 10e6) == "2.4 GB of 2.5 GB · 10 MB/s · 10 s left"


def test_download_watcher_reports_bytes(tmp_path, monkeypatch):
    blobs = tmp_path / "blobs"
    blobs.mkdir()
    monkeypatch.setattr(installer, "model_cache_dir", lambda repo: tmp_path)
    monkeypatch.setattr(installer, "model_cached_bytes", installer.model_cached_bytes)
    got, stop = [], threading.Event()
    t = threading.Thread(target=installer.watch_download, args=("r", 1000, installer.UI(emit=got.append), stop, 0.02))
    t.start()
    for i in range(5):
        (blobs / f"f{i}.incomplete").write_bytes(b"x" * 100)
        time.sleep(0.05)
    stop.set()
    t.join()
    assert got and got[-1]["bytes"] == 500 and got[-1]["total"] == 1000 and got[-1]["progress"] == 50
    assert "of" in got[-1]["stage"]


def test_model_install_reports_progress(monkeypatch, tmp_path):
    import huggingface_hub
    m = installer.Model()
    monkeypatch.setattr(m, "_cached", lambda: False)
    monkeypatch.setattr(installer, "model_total_bytes", lambda repo: 2000)
    monkeypatch.setattr(huggingface_hub, "snapshot_download", lambda repo, **kw: None)
    got = []
    m.install(installer.UI(emit=got.append))
    assert {"progress": 100, "stage": "download", "bytes": 2000, "total": 2000} in got


# --- small helpers ---------------------------------------------------------------------------

def test_only_selects_just_those():
    assert [c.id for c in installer.selection(only=["core"])] == ["core"]
    assert installer.parse(["--yes", "--only", "core"])["only"] == ["core"]
    with pytest.raises(SystemExit):
        installer.selection(only=["nope"])


def test_quick_pending(home, monkeypatch):
    monkeypatch.setattr(installer.Model, "_cached", lambda self: True)
    assert install_flow.quick_pending({"stick": None})
    assert not install_flow.quick_pending({"stick": None, "install_skipped": ["stick"]})
    assert not install_flow.quick_pending({"stick": {"name": "s", "address": "a"}})
    monkeypatch.setattr(installer.Model, "_cached", lambda self: False)
    assert install_flow.quick_pending({"stick": {"name": "s", "address": "a"}})
    assert not install_flow.quick_pending({"stick": {"name": "s"}, "install_skipped": ["model", "bogus"]})


def test_installer_flag_is_used_once(home, monkeypatch):
    from stick_agent import __main__ as cli
    flag = home / "installer-opens-page"
    monkeypatch.setattr(cli, "INSTALLER_FLAG", flag)
    assert not cli.installer_opens_page()
    flag.write_text("")
    assert cli.installer_opens_page() and not flag.exists()
    flag.write_text("")
    old = time.time() - 3600
    import os
    os.utime(flag, (old, old))
    assert not cli.installer_opens_page()  # stale: the agent opens the page itself
