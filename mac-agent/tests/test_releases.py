"""Releases on GitHub: picking stable / beta, listing through gh or plain HTTPS (both
faked — no network), downloading a release's firmware with sha256 checks, the
~/.stick-agent/firmware/ layout, and which bundle flasher.bundled() hands out."""
import hashlib
import io
import json
import subprocess
from pathlib import Path

import pytest

from stick_agent import config, flasher, installer, releases

REPO = releases.REPO
PARTS = [("0x0", "bootloader.bin", 2000), ("0x8000", "partitions.bin", 300), ("0xe000", "boot_app0.bin", 800),
         ("0x10000", "app.bin", 5000)]


def build(fw="1.8.0", tag="v1.8.0", fill=b"\xe9"):
    """The assets of a release: {name: bytes} (4 parts + manifest.json)."""
    files, parts = {}, []
    for offset, name, size in PARTS:
        data = fill + bytes([len(name)]) * (size - 1)
        files[name] = data
        parts.append({"offset": offset, "file": name, "size": size, "sha256": hashlib.sha256(data).hexdigest()})
    manifest = {"version": fw, "board": "m5-sticks3", "release": tag, "agent_version": tag[1:], "built": "today",
                "sha256": hashlib.sha256(b"".join(files[n] for _, n, _ in PARTS)).hexdigest(),
                "size": sum(len(v) for v in files.values()), "parts": parts,
                "flash": {"chip": "esp32s3", "mode": "dio", "freq": "80m", "size": "8MB"}}
    files["manifest.json"] = json.dumps(manifest).encode()
    return files


def api_release(tag, files=None, prerelease=False, draft=False, digests=True):
    files = files if files is not None else build(tag=tag)
    return {"tag_name": tag, "name": f"stick-key {tag}", "draft": draft, "prerelease": prerelease,
            "body": f"notes for {tag}", "html_url": f"https://github.com/{REPO}/releases/tag/{tag}",
            "published_at": "2026-11-05T10:00:00Z",
            "assets": [{"name": n, "size": len(b), "url": f"https://api.github.com/repos/{REPO}/releases/assets/{i}",
                        "browser_download_url": f"https://github.com/{REPO}/releases/download/{tag}/{n}",
                        **({"digest": "sha256:" + hashlib.sha256(b).hexdigest()} if digests else {})}
                       for i, (n, b) in enumerate(files.items())]}


class FakeGitHub:
    """Answers both `gh api` / `gh release download` (releases.run) and HTTPS (releases.urlopen)."""

    def __init__(self, releases_json, assets):
        self.releases = releases_json
        self.assets = assets      # tag -> {name: bytes}
        self.gh_ok = True
        self.https_ok = True
        self.calls = []

    def run(self, cmd, timeout=60):
        self.calls.append(("gh", cmd[1:3]))
        if not self.gh_ok:
            return subprocess.CompletedProcess(cmd, 1, "", "gh: To get started, please run: gh auth login")
        if cmd[1] == "api":
            assert cmd[-1] == f"repos/{REPO}/releases?per_page=30"
            return subprocess.CompletedProcess(cmd, 0, json.dumps(self.releases), "")
        if cmd[1:3] == ["release", "download"]:
            tag, name, d = cmd[3], cmd[cmd.index("--pattern") + 1], Path(cmd[cmd.index("--dir") + 1])
            assert cmd[cmd.index("--repo") + 1] == REPO
            (d / name).write_bytes(self.assets[tag][name])
            return subprocess.CompletedProcess(cmd, 0, "", "")
        raise AssertionError(cmd)

    def urlopen(self, req, timeout=30):
        url = req.full_url
        self.calls.append(("https", url))
        if not self.https_ok:
            raise releases.urllib.error.URLError("offline")
        if url == f"{releases.API}/repos/{REPO}/releases?per_page=30":
            return io.BytesIO(json.dumps(self.releases).encode())
        prefix = f"https://github.com/{REPO}/releases/download/"
        assert url.startswith(prefix), url
        tag, name = url[len(prefix):].split("/")
        return io.BytesIO(self.assets[tag][name])


@pytest.fixture
def github(home, monkeypatch):
    fake = FakeGitHub([], {})
    monkeypatch.setattr(releases, "run", fake.run)
    monkeypatch.setattr(releases, "urlopen", fake.urlopen)
    monkeypatch.setattr(releases, "gh_path", lambda: "/usr/bin/gh")

    def publish(tag, files=None, **kw):
        files = files if files is not None else build(tag=tag)
        fake.assets[tag] = files
        fake.releases.insert(0, api_release(tag, files, **kw))
        return files
    fake.publish = publish
    return fake


# --- picking a release -------------------------------------------------------------------

def test_pick_stable_and_beta():
    raw = [api_release("v1.7.2"), api_release("v1.8.0-beta.1", prerelease=True), api_release("v1.7.10"),
           api_release("v9.0.0", draft=True), api_release("nightly"), api_release("v1.8.0-beta.10", prerelease=True),
           api_release("v1.7.11", prerelease=True)]  # marked prerelease on GitHub without a suffix: beta only
    assert releases.pick(raw, "stable")["tag"] == "v1.7.10"
    assert releases.pick(raw, "beta")["tag"] == "v1.8.0-beta.10"
    assert releases.pick([], "stable") is None
    assert releases.pick([api_release("v2.0.0-rc.1", prerelease=True)], "stable") is None
    r = releases.pick(raw, "stable")
    assert r["version"] == "1.7.10" and r["notes"] == "notes for v1.7.10" and not r["prerelease"]
    assert set(r["assets"]) == {"manifest.json", "bootloader.bin", "partitions.bin", "boot_app0.bin", "app.bin"}
    assert len(r["assets"]["app.bin"]["sha256"]) == 64


def test_a_final_release_beats_its_betas_on_the_beta_channel():
    raw = [api_release("v1.8.0-beta.3", prerelease=True), api_release("v1.8.0"), api_release("v1.8.1-beta.1",
                                                                                              prerelease=True)]
    assert releases.pick(raw[:2], "beta")["tag"] == "v1.8.0"
    assert releases.pick(raw, "beta")["tag"] == "v1.8.1-beta.1"


def test_odd_asset_names_are_dropped():
    r = api_release("v1.8.0")
    r["assets"].append({"name": "../../evil.bin", "size": 1, "browser_download_url": "x"})
    assert "../../evil.bin" not in releases.normalize(r)["assets"]


# --- listing: gh first, then HTTPS --------------------------------------------------------

def test_lists_with_gh_when_it_works(github):
    github.publish("v1.8.0")
    rel, src = releases.latest("stable")
    assert rel["tag"] == "v1.8.0" and src.name == "gh"
    assert [c[0] for c in github.calls] == ["gh"]


def test_falls_back_to_https_when_gh_is_logged_out(github):
    github.publish("v1.8.0")
    github.gh_ok = False
    rel, src = releases.latest("stable")
    assert rel["tag"] == "v1.8.0" and src.name == "https"


def test_https_only_without_gh(github, monkeypatch):
    monkeypatch.setattr(releases, "gh_path", lambda: None)
    github.publish("v1.8.0")
    _rel, src = releases.latest()
    assert src.name == "https" and all(c[0] == "https" for c in github.calls)


def test_both_failing_is_one_error(github):
    github.gh_ok = github.https_ok = False
    with pytest.raises(releases.ReleaseError, match="gh: .*https: .*offline"):
        releases.latest()


def test_private_repo_404_hints_at_gh_login(github, monkeypatch):
    def notfound(req, timeout=30):
        raise releases.urllib.error.HTTPError(req.full_url, 404, "Not Found", {}, None)
    monkeypatch.setattr(releases, "urlopen", notfound)
    monkeypatch.setattr(releases, "gh_path", lambda: None)
    with pytest.raises(releases.ReleaseError, match="gh auth login"):
        releases.latest()


def test_no_release_yet(github):
    with pytest.raises(releases.NoRelease, match="build the firmware locally"):
        releases.latest()
    github.publish("v1.8.0-beta.1", prerelease=True)
    with pytest.raises(releases.NoRelease, match="no stable release"):
        releases.latest("stable")
    assert releases.latest("beta")[0]["tag"] == "v1.8.0-beta.1"


# --- downloading firmware ------------------------------------------------------------------

@pytest.mark.parametrize("via", ["gh", "https"])
def test_download_verifies_and_lays_out_the_cache(github, home, monkeypatch, via):
    if via == "https":
        monkeypatch.setattr(releases, "gh_path", lambda: None)
    files = github.publish("v1.8.0")
    lines = []
    rel, folder, manifest = releases.ensure_latest_firmware("stable", say=lines.append)
    assert folder == home / "firmware" / "1.8.0" and manifest["release"] == "v1.8.0"
    assert sorted(p.name for p in folder.iterdir()) == sorted(files)
    for name, data in files.items():
        assert (folder / name).read_bytes() == data
    assert releases.verify_dir(folder)
    assert not [p for p in (home / "firmware").iterdir() if p.name.startswith(".")]  # no temp folders left
    assert any("match the manifest" in line for line in lines)
    # again: nothing new is downloaded
    n = len(github.calls)
    releases.ensure_latest_firmware("stable", say=lines.append)
    assert "already here" in lines[-1]
    assert len([c for c in github.calls[n:] if "download" in str(c) or "/download/" in str(c)]) == 1  # the manifest


def test_a_tampered_part_is_refused(github, home):
    files = build(tag="v1.8.0")
    files["app.bin"] = files["app.bin"][:-1] + b"X"  # same size, different bytes than the manifest says
    github.publish("v1.8.0", files, digests=False)
    with pytest.raises(releases.ReleaseError, match="app.bin: sha256 doesn't match the manifest"):
        releases.ensure_latest_firmware()
    assert not (home / "firmware" / "1.8.0").exists()
    assert list((home / "firmware").iterdir()) == []
    assert flasher.bundled() is None


def test_githubs_asset_digest_is_checked_too(github, home):
    files = github.publish("v1.8.0")
    rel = releases.pick(github.releases)
    rel["assets"]["manifest.json"]["sha256"] = "0" * 64
    with pytest.raises(releases.ReleaseError, match="GitHub's sha256"):
        releases.download_firmware(rel, releases.Gh("/usr/bin/gh"))
    assert files  # (published fine; only the digest was wrong)


def test_a_manifest_cant_write_outside_its_folder(github, home):
    files = build(tag="v1.8.0")
    m = json.loads(files["manifest.json"])
    m["parts"][0]["file"] = "../../evil.bin"
    files["manifest.json"] = json.dumps(m).encode()
    github.publish("v1.8.0", files)
    with pytest.raises(releases.ReleaseError, match="odd firmware part name"):
        releases.ensure_latest_firmware()
    assert not (home / "evil.bin").exists()


def test_a_release_without_firmware(github):
    github.publish("v1.8.0", {"notes.txt": b"hi"})
    with pytest.raises(releases.ReleaseError, match="no firmware"):
        releases.ensure_latest_firmware()


def test_prune_keeps_the_newest_and_the_local_build(home):
    root = home / "firmware"
    for v in ("1.7.0", "1.7.2", "1.8.0-beta.1", "1.8.0", "1.6.9", "local"):
        (root / v).mkdir(parents=True)
    releases.prune(root, keep=3)
    assert sorted(p.name for p in root.iterdir()) == ["1.7.2", "1.8.0", "1.8.0-beta.1", "local"]


# --- which firmware the flasher uses -----------------------------------------------------------

def put_bundle(folder, fw, built="today"):
    folder.mkdir(parents=True, exist_ok=True)
    files = build(fw=fw)
    m = json.loads(files.pop("manifest.json"))
    m["built"] = built
    for name, data in files.items():
        (folder / name).write_bytes(data)
    (folder / "manifest.json").write_text(json.dumps(m))
    return folder


def test_bundled_picks_the_newest_and_a_local_build_wins_a_tie(home, monkeypatch, tmp_path):
    monkeypatch.setattr(flasher, "BUNDLE", tmp_path / "none")
    assert flasher.bundled() is None
    put_bundle(home / "firmware" / "1.7.2", "1.7.2")
    fw = flasher.bundled()
    assert fw["version"] == "1.7.2" and fw["source"] == "release" and fw["dir"] == str(home / "firmware" / "1.7.2")
    assert fw["files"] == [(o, str(home / "firmware" / "1.7.2" / n)) for o, n, _ in PARTS]  # the OTA / flash shape
    put_bundle(home / "firmware" / "local", "1.7.2", built="mine")
    assert flasher.bundled()["source"] == "local" and flasher.bundled()["built"] == "mine"
    put_bundle(home / "firmware" / "1.8.0", "1.8.0")
    assert flasher.bundled()["version"] == "1.8.0"
    assert [b["source"] for b in flasher.bundles()] == ["release", "local", "release"]


def test_an_incomplete_bundle_is_ignored(home, monkeypatch, tmp_path):
    monkeypatch.setattr(flasher, "BUNDLE", tmp_path / "none")
    put_bundle(home / "firmware" / "1.7.2", "1.7.2")
    broken = put_bundle(home / "firmware" / "1.9.0", "1.9.0")
    (broken / "app.bin").write_bytes(b"\xe9short")  # wrong size: a copy that went wrong
    assert flasher.bundled()["version"] == "1.7.2"
    (broken / "app.bin").unlink()
    assert flasher.bundled()["version"] == "1.7.2"


def test_flashing_without_any_firmware_says_how_to_get_some(home, monkeypatch, tmp_path, capsys):
    monkeypatch.setattr(flasher, "BUNDLE", tmp_path / "none")
    assert flasher.cli("/dev/cu.usbmodemX") == 1
    out = capsys.readouterr().out
    assert "no release downloaded" in out and "make -C m5-stick-s3 release" in out


# --- the installer's firmware component ------------------------------------------------------

def test_installer_firmware_downloads_the_latest_release_first(github, home, monkeypatch, tmp_path):
    monkeypatch.setattr(flasher, "BUNDLE", tmp_path / "none")
    github.publish("v1.8.0")
    monkeypatch.setattr(flasher, "espressif_ports", lambda: [{"port": "/dev/cu.usbmodemS", "serial": "", "product": "",
                                                              "pid": 1}])
    flashed = []
    monkeypatch.setattr(flasher, "cli", lambda port, backup: flashed.append((port, flasher.bundled()["version"])) or 0)
    lines = []
    installer.Firmware().install(installer.UI(emit=lambda e: lines.append(e.get("line"))))
    assert flashed == [("/dev/cu.usbmodemS", "1.8.0")]
    assert any("from release v1.8.0" in (line or "") for line in lines)
    assert installer.Firmware().status()[1].startswith("firmware 1.8.0 (release v1.8.0)")


def test_installer_firmware_without_a_release(github, home, monkeypatch, tmp_path):
    monkeypatch.setattr(flasher, "BUNDLE", tmp_path / "none")
    with pytest.raises(installer.NotAvailable, match="make -C m5-stick-s3 release"):
        installer.Firmware().install(installer.UI(emit=lambda e: None))
    assert "downloads the latest release" in installer.Firmware().status()[1]


def test_uses_the_beta_channel_from_the_config(github, home, monkeypatch, tmp_path):
    monkeypatch.setattr(flasher, "BUNDLE", tmp_path / "none")
    github.publish("v1.7.2")
    github.publish("v1.8.0-beta.1", build(fw="1.8.0-beta.1", tag="v1.8.0-beta.1"), prerelease=True)
    cfg = config.load()
    cfg["update_channel"] = "beta"
    config.save(cfg)
    installer.fetch_firmware(installer.UI(emit=lambda e: None))
    assert flasher.bundled()["version"] == "1.8.0-beta.1"
