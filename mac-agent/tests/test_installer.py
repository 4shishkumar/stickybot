"""The installer's components (`stickybot setup`), the Claude Code hook merge,
flags, dry-run, and the dashboard's /api/components. Everything happens in
throwaway folders: no real login item, no real ~/.claude/settings.json."""
import json
import os
import pathlib
import subprocess

import pytest

from stick_agent import config, installer, server

HOST = {"Host": f"localhost:{config.PORT}"}
RUNKIT = {"type": "command", "command": "/bin/sh -c 'run-kit agent hook idle'"}


@pytest.fixture
def sandbox(home, tmp_path, monkeypatch):
    """Test paths for everything the installer touches."""
    monkeypatch.setenv("STICK_AGENT_BIN", str(tmp_path / "bin"))
    monkeypatch.setenv("STICK_AGENT_CLAUDE_SETTINGS", str(tmp_path / "claude" / "settings.json"))
    monkeypatch.setenv("STICK_AGENT_NO_LOGIN_ITEM", "1")
    return tmp_path


def settings_file(sandbox):
    return sandbox / "claude" / "settings.json"


def ours(event):
    return installer.our_hooks()[event]


# --- the hook merge --------------------------------------------------------------------

def test_merge_into_empty_settings_adds_three_hooks():
    s, changed = installer.merge_hooks({})
    assert changed
    assert set(s["hooks"]) == {"PermissionRequest", "UserPromptSubmit", "Stop"}
    assert installer.hooks_state(s) == (3, 3)
    assert s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] == 45
    assert s["hooks"]["Stop"][0]["hooks"][0]["timeout"] == 5


def test_merge_is_idempotent():
    once, _ = installer.merge_hooks({})
    twice, changed = installer.merge_hooks(once)
    assert not changed and twice == once


def test_merge_keeps_other_hooks_and_settings():
    before = {"theme": "dark", "hooks": {
        "Stop": [{"hooks": [RUNKIT]}],
        "PreToolUse": [{"matcher": "Bash", "hooks": [RUNKIT]}],
    }}
    s, changed = installer.merge_hooks(before)
    assert changed and s["theme"] == "dark"
    assert s["hooks"]["Stop"][0]["hooks"] == [RUNKIT, ours("Stop")]  # joined the existing group
    assert s["hooks"]["PreToolUse"] == before["hooks"]["PreToolUse"]  # untouched
    assert before["hooks"]["Stop"][0]["hooks"] == [RUNKIT]  # the input isn't modified


def test_merge_doesnt_join_a_group_with_a_matcher():
    s, _ = installer.merge_hooks({"hooks": {"PermissionRequest": [{"matcher": "Bash", "hooks": [RUNKIT]}]}})
    groups = s["hooks"]["PermissionRequest"]
    assert groups[0] == {"matcher": "Bash", "hooks": [RUNKIT]}
    assert groups[1] == {"hooks": [ours("PermissionRequest")]}


def test_merge_updates_a_hook_from_another_install():
    old = {"type": "command", "command": "/old/venv/bin/python /old/repo/mac-agent/stick_agent/hook.py", "timeout": 45}
    s, changed = installer.merge_hooks({"hooks": {"PermissionRequest": [{"hooks": [old]}]}})
    assert changed
    assert s["hooks"]["PermissionRequest"] == [{"hooks": [ours("PermissionRequest")]}]  # replaced, not duplicated


def test_hooks_state_counts_outdated_ones():
    old = {"type": "command", "command": "/x/stick_agent/claude_hooks.py", "timeout": 5}
    assert installer.hooks_state({"hooks": {"Stop": [{"hooks": [old]}]}}) == (0, 1)


def test_remove_takes_only_ours():
    merged, _ = installer.merge_hooks({"model": "x", "hooks": {"Stop": [{"hooks": [RUNKIT]}]}})
    s, changed = installer.remove_hooks(merged)
    assert changed
    assert s == {"model": "x", "hooks": {"Stop": [{"hooks": [RUNKIT]}]}}
    _, again = installer.remove_hooks(s)
    assert not again


def test_remove_everything_drops_the_hooks_key():
    merged, _ = installer.merge_hooks({"theme": "dark"})
    s, _ = installer.remove_hooks(merged)
    assert s == {"theme": "dark"}


# --- the claude component (settings file) -------------------------------------------------

def test_claude_install_writes_hooks_with_a_backup(sandbox):
    f = settings_file(sandbox)
    f.parent.mkdir(parents=True)
    f.write_text(json.dumps({"theme": "dark", "hooks": {"Stop": [{"hooks": [RUNKIT]}]}}))
    installer.Claude().install(installer.UI())
    s = json.loads(f.read_text())
    assert installer.hooks_state(s) == (3, 3) and s["theme"] == "dark"
    backups = list(f.parent.glob("settings.json.bak-*"))
    assert len(backups) == 1 and json.loads(backups[0].read_text())["theme"] == "dark"
    assert installer.Claude().status()[0] == "installed"  # hooks current; claude_path (/usr/bin/false) exists


def test_claude_install_twice_makes_no_second_backup(sandbox):
    installer.Claude().install(installer.UI())
    installer.Claude().install(installer.UI())
    assert list(settings_file(sandbox).parent.glob("settings.json.bak-*")) == []  # no file existed the first time


def test_claude_uninstall_keeps_other_hooks(sandbox):
    f = settings_file(sandbox)
    f.parent.mkdir(parents=True)
    f.write_text(json.dumps({"hooks": {"Stop": [{"hooks": [RUNKIT]}]}}))
    c = installer.Claude()
    c.install(installer.UI())
    c.uninstall(installer.UI())
    assert json.loads(f.read_text()) == {"hooks": {"Stop": [{"hooks": [RUNKIT]}]}}


def test_claude_never_overwrites_a_broken_settings_file(sandbox):
    f = settings_file(sandbox)
    f.parent.mkdir(parents=True)
    f.write_text("{not json")
    with pytest.raises(ValueError):
        installer.Claude().install(installer.UI())
    assert f.read_text() == "{not json"
    assert installer.Claude().status()[0] == "partial"


def test_claude_status_with_a_working_claude(sandbox, tmp_path, monkeypatch):
    fake = tmp_path / "fake-claude"
    fake.write_text('#!/bin/sh\nif [ "$1" = auth ]; then echo \'{"loggedIn": true}\'; else echo "2.1.0"; fi\n')
    fake.chmod(0o755)
    cfg = config.load()
    cfg["claude_path"] = str(fake)
    config.save(cfg)
    c = installer.Claude()
    assert c.status()[0] == "missing"
    c.install(installer.UI())
    state, detail = c.status()
    assert state == "installed" and "3/3" in detail
    assert c._logged_in(str(fake))


# --- core / model / github ------------------------------------------------------------------

@pytest.fixture
def venv(sandbox, monkeypatch):
    """A venv with both entry points (`stickybot` and the older `stick-agent`)."""
    bin_ = sandbox / "venv" / "bin"
    bin_.mkdir(parents=True)
    for name in ("python", "stickybot", "stick-agent"):
        (bin_ / name).write_text("#!/bin/sh\n")
    monkeypatch.setattr(installer, "venv_python", lambda: bin_ / "python")
    return bin_


def test_core_install_links_both_commands_and_uninstall_removes_them(sandbox, venv):
    core = installer.Core()
    core.install(installer.UI())
    for name in ("stickybot", "stick-agent"):
        link = sandbox / "bin" / name
        assert link.is_symlink() and link.resolve() == (venv / name).resolve()
    assert core.status()[0] == "installed"  # login item skipped in tests
    core.install(installer.UI())  # again: fine
    app = pathlib.Path(os.environ["STICK_AGENT_APP"])
    (app / "Contents" / "MacOS").mkdir(parents=True)  # as if the login item's install had built it
    lines = []
    core.uninstall(installer.UI(emit=lines.append))
    assert not (sandbox / "bin" / "stickybot").exists() and not (sandbox / "bin" / "stick-agent").exists()
    assert not app.exists() and any(str(app) in x.get("line", "") for x in lines)  # the app goes too


def test_core_links_only_the_commands_the_venv_has(sandbox, venv):
    (venv / "stickybot").unlink()  # a venv from before the alias (reinstalled by install.sh)
    installer.Core().install(installer.UI())
    assert (sandbox / "bin" / "stick-agent").is_symlink() and not (sandbox / "bin" / "stickybot").exists()


def test_core_with_a_login_item_builds_the_app_first(sandbox, venv, monkeypatch):
    from stick_agent import __main__ as cli
    from stick_agent import app
    monkeypatch.delenv("STICK_AGENT_NO_LOGIN_ITEM")
    order = []
    monkeypatch.setattr(app, "ensure", lambda say=None: order.append("app") or (True, "StickyBot.app: built"))
    monkeypatch.setattr(cli, "write_plist", lambda: order.append("plist") or cli.LOCAL_PLIST)
    monkeypatch.setattr(cli, "launchctl", lambda *a: order.append(a[0]) or
                        subprocess.CompletedProcess(a, 1 if a[0] == "print" else 0, "", ""))  # print: gone
    core = installer.Core()
    monkeypatch.setattr(core, "_dashboard_up", lambda: True)
    lines = []
    core.install(installer.UI(emit=lines.append))
    assert order == ["app", "plist", "bootout", "print", "bootstrap"]
    assert any("StickyBot.app: built" in x.get("line", "") for x in lines)


def test_model_skips_the_download_when_cached(monkeypatch):
    m = installer.Model()
    monkeypatch.setattr(m, "_cached", lambda: True)
    lines = []
    m.install(installer.UI(emit=lines.append))
    assert "already downloaded" in lines[0]["line"]
    assert m.status()[0] == "installed"


def test_model_downloads_when_missing(monkeypatch):
    import huggingface_hub
    m = installer.Model()
    monkeypatch.setattr(m, "_cached", lambda: False)
    got = []
    monkeypatch.setattr(huggingface_hub, "snapshot_download", lambda repo, **kw: got.append(repo))
    m.install(installer.UI(emit=lambda e: None))
    assert got == ["mlx-community/parakeet-tdt-0.6b-v2"]


def test_github_needs_gh(monkeypatch):
    g = installer.GitHub()
    monkeypatch.setattr(g, "_gh", lambda: None)
    assert g.status()[0] == "missing"
    with pytest.raises(installer.NotAvailable):
        g.install(installer.UI())


def test_github_install_turns_the_watcher_on(home, monkeypatch):
    g = installer.GitHub()
    monkeypatch.setattr(g, "_gh", lambda: "/usr/bin/gh")
    monkeypatch.setattr(installer, "run", lambda cmd, **kw: subprocess.CompletedProcess(cmd, 0, "", ""))
    cfg = config.load()
    cfg["github"] = {"on": False, "primary_repo": "acme/app", "interval_s": 120}
    config.save(cfg)
    g.install(installer.UI())
    assert config.load()["github"] == {"on": True, "primary_repo": "acme/app", "interval_s": 120}
    assert g.status()[0] == "installed"
    g.uninstall(installer.UI())
    assert config.load()["github"]["on"] is False


def test_github_not_logged_in(monkeypatch):
    g = installer.GitHub()
    monkeypatch.setattr(g, "_gh", lambda: "/usr/bin/gh")
    monkeypatch.setattr(installer, "run", lambda cmd, **kw: subprocess.CompletedProcess(cmd, 1, "", ""))
    assert g.status()[0] == "partial"
    with pytest.raises(installer.NotAvailable):
        g.install(installer.UI())


def test_wireless_mic_without_blackhole(home, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: False)
    w = installer.BY_ID["wireless_mic"]
    assert w.status()[0] == "missing" and "brew install --cask blackhole-2ch" in w.status()[1]
    lines = []
    with pytest.raises(installer.NotAvailable) as e:
        w.install(installer.UI(emit=lines.append))
    assert "brew install --cask blackhole-2ch" in str(e.value)  # says what to run; never runs it (sudo)
    assert config.load()["wireless_mic"] is True                 # on: works as soon as BlackHole appears
    assert w.status()[0] == "partial"


def test_wireless_mic_install_and_uninstall(home, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: True)
    w = installer.BY_ID["wireless_mic"]
    assert w.status() == ("missing", "off · BlackHole 2ch found")
    lines = []
    w.install(installer.UI(emit=lines.append))
    assert config.load()["wireless_mic"] is True
    assert w.status()[0] == "installed"
    assert any("BlackHole 2ch" in x.get("line", "") for x in lines)
    w.uninstall(installer.UI(emit=lines.append))
    assert config.load()["wireless_mic"] is False
    assert w.status()[0] == "missing"


def test_blackhole_is_found_by_its_driver(tmp_path, monkeypatch):
    driver = tmp_path / "BlackHole2ch.driver"
    monkeypatch.setattr(installer, "BLACKHOLE_DRIVERS", [driver])
    assert not installer.blackhole_installed()
    driver.mkdir()
    assert installer.blackhole_installed()


# --- selection, flags, main ----------------------------------------------------------------

def test_defaults_and_with_without():
    assert [c.id for c in installer.selection()] == ["core", "model", "claude"]
    assert [c.id for c in installer.selection(["github"], ["model"])] == ["core", "claude", "github"]
    assert "core" in [c.id for c in installer.selection(without=["core"])]  # core is always there
    with pytest.raises(SystemExit):
        installer.selection(["nope"])


def test_parse_flags(monkeypatch):
    monkeypatch.delenv("STICK_AGENT_NO_LOGIN_ITEM", raising=False)
    o = installer.parse(["--yes", "--with", "github,firmware", "--without=model", "--dry-run", "--json",
                         "--no-login-item"])
    assert o["yes"] and o["dry_run"] and o["json"]
    assert o["with"] == ["github", "firmware"] and o["without"] == ["model"]
    assert installer.no_login_item()
    assert installer.parse(["--uninstall"])["uninstall"] == list(installer.BY_ID)
    assert installer.parse(["--uninstall", "all", "--purge"])["uninstall"] == list(installer.BY_ID)
    assert installer.parse(["--uninstall", "claude,github"])["uninstall"] == ["claude", "github"]
    with pytest.raises(SystemExit):
        installer.parse(["--bogus"])


def test_dry_run_changes_nothing(sandbox, capsys):
    assert installer.main(["--yes", "--dry-run", "--with", "github"]) == 0
    assert not (sandbox / "bin").exists()
    assert not settings_file(sandbox).exists()
    out = capsys.readouterr().out
    assert "dry run" in out and "Claude Code integration" in out


def test_yes_installs_the_chosen_components(sandbox, monkeypatch, capsys):
    monkeypatch.setattr(installer.Model, "install", lambda self, ui: ui.say("model: fake"))
    assert installer.main(["--yes", "--without", "github"]) == 0
    assert (sandbox / "bin" / "stick-agent").is_symlink()
    assert installer.hooks_state(json.loads(settings_file(sandbox).read_text())) == (3, 3)
    assert "Done:" in capsys.readouterr().out


def test_uninstall_all(sandbox, monkeypatch):
    monkeypatch.setattr(installer.Model, "uninstall", lambda self, ui: ui.say("model: fake"))
    installer.main(["--yes", "--without", "model"])
    assert installer.main(["--uninstall", "all", "--yes"]) == 0
    assert not (sandbox / "bin" / "stick-agent").exists()
    assert "hooks" not in json.loads(settings_file(sandbox).read_text())


def test_status_json(sandbox, capsys, monkeypatch):
    monkeypatch.setattr(installer.Model, "status", lambda self: ("missing", "not downloaded"))
    assert installer.main(["--status", "--json"]) == 0
    data = json.loads(capsys.readouterr().out)
    assert [c["id"] for c in data] == ["core", "model", "claude", "github", "firmware", "wireless_mic", "mcp"]
    assert {"status", "detail", "needs", "from_dashboard", "removable"} <= set(data[0])


# --- the dashboard API ----------------------------------------------------------------------

@pytest.fixture
async def client(agent, sandbox, aiohttp_client, monkeypatch):
    monkeypatch.setattr(installer.Model, "status", lambda self: ("missing", "not downloaded"))
    c = await aiohttp_client(server.make_app(agent))
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    return c


async def test_components_listing(client):
    r = await client.get("/api/components", headers=HOST)
    assert r.status == 200
    ids = [c["id"] for c in await r.json()]
    assert ids == ["core", "model", "claude", "github", "firmware", "wireless_mic", "mcp"]


async def test_component_changes_need_the_token(client):
    r = await client.post("/api/components/claude/install", headers=HOST)
    assert r.status == 403


async def test_component_job_streams_and_installs(client, sandbox):
    r = await client.post("/api/components/claude/install", headers=client.auth)
    job = (await r.json())["job"]
    r = await client.get(f"/api/components/jobs/{job}", headers=HOST)
    events = [json.loads(b.removeprefix("data: ")) for b in (await r.text()).strip().split("\n\n")
              if b.startswith("data: ")]
    assert events[-1] == {"done": True, "ok": True, "error": None}
    assert any("hooks added" in e.get("line", "") for e in events)
    assert installer.hooks_state(json.loads(settings_file(sandbox).read_text())) == (3, 3)


async def test_component_errors(client):
    r = await client.post("/api/components/core/install", headers=client.auth)
    assert r.status == 400  # it's what runs the dashboard
    r = await client.post("/api/components/firmware/uninstall", headers=client.auth)
    assert r.status == 400
    r = await client.post("/api/components/nope/install", headers=client.auth)
    assert r.status == 404
    r = await client.get("/api/components/jobs/nope", headers=HOST)
    assert r.status == 404


async def test_wireless_mic_from_the_dashboard_updates_the_agent(client, agent, monkeypatch):
    monkeypatch.setattr(installer, "blackhole_installed", lambda: True)
    r = await client.post("/api/components/wireless_mic/install", headers=client.auth)
    job = (await r.json())["job"]
    r = await client.get(f"/api/components/jobs/{job}", headers=HOST)
    events = [json.loads(b.removeprefix("data: ")) for b in (await r.text()).strip().split("\n\n")
              if b.startswith("data: ")]
    assert {"done": True, "ok": True, "error": None} in events, events
    assert agent.cfg["wireless_mic"] is True and agent.wispr.enabled


async def test_component_not_available_is_reported(client, monkeypatch):
    monkeypatch.setattr(installer.GitHub, "_gh", lambda self: None)
    r = await client.post("/api/components/github/install", headers=client.auth)
    job = (await r.json())["job"]
    r = await client.get(f"/api/components/jobs/{job}", headers=HOST)
    last = json.loads((await r.text()).strip().split("\n\n")[-1].removeprefix("data: "))
    assert last["done"] and not last["ok"] and "gh" in last["error"]


def test_reload_waits_for_the_old_service_and_retries_bootstrap(monkeypatch):
    from stick_agent import __main__ as cli
    calls, answers = [], {"print": [0, 0, 1], "bootstrap": [5, 0]}  # still there twice; bootstrap fails once
    def fake(*a):
        calls.append(a[0])
        codes = answers.get(a[0])
        return subprocess.CompletedProcess(a, codes.pop(0) if codes else 0, "", "Bootstrap failed: 5")
    monkeypatch.setattr(cli, "launchctl", fake)
    r = cli.reload_login_item("/tmp/x.plist", sleep=lambda s: None)
    assert r.returncode == 0
    assert calls == ["bootout", "print", "print", "print", "bootstrap", "bootstrap"]
