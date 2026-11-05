"""The one-click agent update: which installs it may touch (install.sh's clone only;
a developer checkout gets instructions), the exact commands, rollback, the restart, the
API job, and the firmware-updater hook (where Bluetooth OTA plugs in). git and uv are
never run: selfupdate.git_out / run_cmd are fakes here (conftest refuses the real ones)."""
import asyncio
import json
import sys

import pytest

from stick_agent import __version__, config, flasher, selfupdate, server, updates

HOST = {"Host": f"localhost:{config.PORT}"}
TAG = "v9.0.0"


class FakeGit:
    """git_out + run_cmd: answers from a table, records every command, can fail one step."""

    def __init__(self, root, dirty="", fail=None, version="9.0.0"):
        self.root, self.dirty, self.fail, self.version = root, dirty, fail, version
        self.ran = []

    def git_out(self, root, *args, timeout=20):
        assert root == self.root, f"git against {root}, not the test clone"
        if args[0] == "describe":
            return 0, "v1.7.2"
        if args[0] == "symbolic-ref":
            return 1, ""
        if args[0] == "status":
            return 0, self.dirty
        if args[0] == "rev-parse":
            return 0, "abc1234def5678"
        raise AssertionError(args)

    async def run_cmd(self, cmd, emit):
        self.ran.append(cmd)
        emit({"line": "ok"})
        if "checkout" in cmd and cmd[-1] == TAG:
            init = self.root / "mac-agent" / "stick_agent" / "__init__.py"
            init.write_text(f'__version__ = "{self.version}"\n')
        return 1 if self.fail and self.fail in " ".join(cmd) else 0


def make_checkout(path):
    (path / ".git").mkdir(parents=True)
    (path / "mac-agent" / "stick_agent").mkdir(parents=True)
    (path / "mac-agent" / "pyproject.toml").write_text("[project]\n")
    (path / "mac-agent" / "stick_agent" / "__init__.py").write_text(f'__version__ = "{__version__}"\n')
    return path


@pytest.fixture
def clone(home, monkeypatch):
    """install.sh's clone: ~/.stick-agent/src (here: the test home)."""
    root = make_checkout(home / "src")
    fake = FakeGit(root)
    monkeypatch.setattr(selfupdate, "git_out", fake.git_out)
    monkeypatch.setattr(selfupdate, "run_cmd", fake.run_cmd)
    monkeypatch.setattr(selfupdate, "source_root", lambda: root)
    monkeypatch.setattr(selfupdate, "find_uv", lambda: "/Users/me/.local/bin/uv")
    return fake


# --- which installs -------------------------------------------------------------------------

def test_the_install_clone_can_update(clone):
    info = selfupdate.install_info()
    assert info["kind"] == "release" and info["can_update"] and info["head"] == "v1.7.2" and info["branch"] is None


def test_local_changes_block_it(clone):
    clone.dirty = " M mac-agent/stick_agent/core.py"
    info = selfupdate.install_info()
    assert not info["can_update"] and "local changes" in info["why"] and "git status" in info["instructions"][0]


def test_a_developer_checkout_gets_instructions(home, tmp_path, monkeypatch):
    dev = make_checkout(tmp_path / "code" / "stick-key")
    monkeypatch.setattr(selfupdate, "git_out", FakeGit(dev).git_out)
    info = selfupdate.install_info(dev)
    assert info["kind"] == "dev" and not info["can_update"]
    assert info["instructions"][0] == f"cd {dev}" and any("git pull --ff-only" in x for x in info["instructions"])
    assert info["instructions"][-1] == "stick-agent restart"


def test_not_a_checkout(tmp_path):
    info = selfupdate.install_info(tmp_path / "site-packages")
    assert info["kind"] == "package" and not info["can_update"] and "install.sh" in info["instructions"][0]


def test_this_repo_is_a_dev_checkout_for_the_tests():
    assert selfupdate.source_root() is not None  # the tests run from a checkout (never ~/.stick-agent/src)
    assert selfupdate.source_root().resolve() != selfupdate.managed_root().resolve()


# --- the commands -----------------------------------------------------------------------------

def test_plan_with_uv_and_without(tmp_path):
    root = tmp_path / "src"
    steps = selfupdate.plan(TAG, root, python="/venv/bin/python", uv="/bin/uv")
    assert [cmd for _, cmd in steps] == [
        ["git", "-C", str(root), "fetch", "--tags", "--force", "origin"],
        ["git", "-C", str(root), "-c", "advice.detachedHead=false", "checkout", "--quiet", TAG],
        ["/bin/uv", "pip", "install", "--python", "/venv/bin/python", "-e", f"{root}/mac-agent"],
    ]
    assert selfupdate.plan(TAG, root, python="/venv/bin/python")[2][1] == [
        "/venv/bin/python", "-m", "pip", "install", "-e", f"{root}/mac-agent"]
    assert selfupdate.plan(TAG, root)[2][1][0] == sys.executable


async def test_update_runs_the_steps_then_restarts(clone):
    events, restarted = [], []
    await selfupdate.update(TAG, events.append, restart=lambda: restarted.append(1))
    root = str(clone.root)
    assert clone.ran == [c for _, c in selfupdate.plan(TAG, root, uv="/Users/me/.local/bin/uv")]
    assert restarted == [1]
    assert {"progress": 100, "stage": "restart"} in events
    assert any(e.get("line", "").startswith("$ git -C") for e in events)


async def test_a_failed_install_goes_back(clone):
    clone.fail = "pip install"
    restarted = []
    with pytest.raises(selfupdate.UpdateError, match="install it failed"):
        await selfupdate.update(TAG, lambda e: None, restart=lambda: restarted.append(1))
    assert restarted == []
    back = ["git", "-C", str(clone.root), "-c", "advice.detachedHead=false", "checkout", "--quiet", "abc1234def5678"]
    assert clone.ran[-2] == back and "pip" in clone.ran[-1]  # the old checkout, installed again


async def test_a_failed_fetch_changes_nothing(clone):
    clone.fail = "fetch"
    with pytest.raises(selfupdate.UpdateError, match="fetch"):
        await selfupdate.update(TAG, lambda e: None, restart=lambda: None)
    assert len(clone.ran) == 1


async def test_a_tag_with_the_wrong_version_goes_back(clone):
    clone.version = "1.0.0"
    with pytest.raises(selfupdate.UpdateError, match="has version 1.0.0"):
        await selfupdate.update(TAG, lambda e: None, restart=lambda: None)
    assert clone.ran[-1][-1] == "abc1234def5678" and not any("pip" in c for c in clone.ran)


async def test_refuses_odd_tags_and_dev_checkouts(clone, tmp_path, monkeypatch):
    with pytest.raises(selfupdate.UpdateError, match="not a release tag"):
        await selfupdate.update("main; rm -rf /", lambda e: None, restart=lambda: None)
    dev = make_checkout(tmp_path / "dev")
    monkeypatch.setattr(selfupdate, "source_root", lambda: dev)
    monkeypatch.setattr(selfupdate, "git_out", FakeGit(dev).git_out)
    with pytest.raises(selfupdate.UpdateError, match="developer checkout"):
        await selfupdate.update(TAG, lambda e: None, restart=lambda: None)
    assert clone.ran == []


# --- the API --------------------------------------------------------------------------------

@pytest.fixture
async def client(agent, aiohttp_client):
    c = await aiohttp_client(server.make_app(agent))
    c.agent = agent
    c.auth = {**HOST, "X-Stick-Token": config.token()}
    agent.updates.latest = {"tag": TAG, "version": TAG[1:], "notes": "", "firmware": None}
    return c


async def events_of(client, job):
    text = await (await client.get(f"/api/update/jobs/{job}", headers=HOST)).text()
    return [json.loads(line[6:]) for line in text.splitlines() if line.startswith("data: ")]


async def test_api_updates_the_install_clone_and_restarts(client, clone, monkeypatch):
    restarts = []
    monkeypatch.setattr(client.agent, "restart_soon", lambda delay=0.3: restarts.append(delay))
    assert (await client.post("/api/update/agent", headers=HOST)).status == 403  # the token
    r = await client.post("/api/update/agent", headers=client.auth, json={})
    job = (await r.json())["job"]
    evs = await events_of(client, job)
    assert evs[-1] == {"done": True, "ok": True, "error": None}
    assert restarts == [1.0] and clone.ran[1][-1] == TAG


async def test_api_shows_instructions_for_a_dev_checkout(client, tmp_path, monkeypatch):
    dev = make_checkout(tmp_path / "dev")
    monkeypatch.setattr(selfupdate, "source_root", lambda: dev)
    monkeypatch.setattr(selfupdate, "git_out", FakeGit(dev).git_out)
    r = await client.post("/api/update/agent", headers=client.auth, json={})
    d = await r.json()
    assert r.status == 409 and d["install"]["kind"] == "dev" and d["install"]["instructions"]


async def test_api_without_a_release(client):
    client.agent.updates.latest = None
    r = await client.post("/api/update/agent", headers=client.auth, json={})
    assert r.status == 400 and "check for updates" in (await r.json())["error"]


async def test_a_failed_update_job_reports_it(client, clone, monkeypatch):
    monkeypatch.setattr(client.agent, "restart_soon", lambda delay=0.3: None)
    clone.fail = "pip install"
    job = (await (await client.post("/api/update/agent", headers=client.auth, json={})).json())["job"]
    evs = await events_of(client, job)
    assert evs[-1]["done"] and not evs[-1]["ok"] and "install it failed" in evs[-1]["error"]


# --- the firmware-updater hook (Bluetooth OTA plugs in here) -----------------------------------

@pytest.fixture
def ota(monkeypatch):
    monkeypatch.setattr(updates, "FIRMWARE_UPDATERS", {})
    ran = []

    async def run(agent, fw, emit):
        ran.append(fw["files"])
        emit({"progress": 50, "stage": "send"})
        await asyncio.sleep(0)
    state = {"why": ""}
    updates.register_firmware_updater("ota", "Update over Bluetooth", lambda agent: state["why"], run)
    return ran, state


async def test_a_registered_firmware_updater_is_listed_and_runs(client, ota, monkeypatch):
    ran, state = ota
    monkeypatch.setattr(selfupdate, "install_info", lambda root=None: {"kind": "dev", "can_update": False})
    fw = {"version": "1.8.0", "source": "release", "release": "v1.8.0", "files": [("0x10000", "/x/app.bin")]}
    monkeypatch.setattr(flasher, "bundled", lambda: fw)
    d = await (await client.get("/api/update", headers=HOST)).json()
    assert d["firmware_updaters"] == [{"id": "ota", "title": "Update over Bluetooth", "available": True, "why": ""}]
    job = (await (await client.post("/api/update/firmware/ota", headers=client.auth)).json())["job"]
    evs = await events_of(client, job)
    assert evs[-1]["ok"] and {"progress": 50, "stage": "send"} in evs and ran == [fw["files"]]
    state["why"] = "the stick isn't connected"
    r = await client.post("/api/update/firmware/ota", headers=client.auth)
    assert r.status == 409 and (await r.json())["error"] == "the stick isn't connected"
    assert (await client.post("/api/update/firmware/nope", headers=client.auth)).status == 404


async def test_a_firmware_updater_needs_firmware(client, ota, monkeypatch):
    monkeypatch.setattr(flasher, "bundled", lambda: None)
    r = await client.post("/api/update/firmware/ota", headers=client.auth)
    assert r.status == 409 and "make -C m5-stick-s3 release" in (await r.json())["error"]
