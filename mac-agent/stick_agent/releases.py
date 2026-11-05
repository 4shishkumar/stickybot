"""Releases of stick-key on GitHub: versions, picking the latest per channel, and
downloading a release's firmware into ~/.stick-agent/firmware/<firmware version>/.

Releases are listed with the `gh` CLI when it's installed and logged in (works for a
private repo, with your own login), else over plain HTTPS from api.github.com (a
public repo, no login). Assets come the same way: `gh release download`, or the
asset's download URL. Every firmware part is checked against the sha256 in the
release's manifest.json (and GitHub's own asset digest, when it gives one) before
it lands in place; a download goes to a temporary folder first, so a half-finished
one is never used.

Kept to the standard library: tools/release_tool.py imports it without the venv.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

SEMVER = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$")
REPO = os.environ.get("STICK_AGENT_REPO", "ashish-kumar-noon/stick-key")
API = "https://api.github.com"
CHANNELS = ("stable", "beta")
MANIFEST = "manifest.json"
SAFE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,80}$")  # asset / folder names we'll write
KEEP = 3  # firmware versions kept in ~/.stick-agent/firmware (plus the local build)
NO_RELEASE = "no release yet — build the firmware locally: make -C m5-stick-s3 release"


# --- versions -----------------------------------------------------------------------------

def parse_version(v):
    """'v1.8.0-beta.2' -> (1, 8, 0, ("beta", 2)); None if it isn't a version."""
    m = SEMVER.match(str(v or "").strip())
    if not m:
        return None
    pre = tuple(int(p) if p.isdigit() else p for p in m.group(4).split(".")) if m.group(4) else ()
    return int(m.group(1)), int(m.group(2)), int(m.group(3)), pre


def version_key(v):
    """A sort key with SemVer precedence: 1.8.0-beta.2 < 1.8.0-beta.10 < 1.8.0-rc.1 < 1.8.0.
    Unparsable versions sort first."""
    p = parse_version(v)
    if p is None:
        return (0, ())
    major, minor, patch, pre = p
    # a release (no pre-release part) sorts after all of its pre-releases; within one,
    # numbers sort before words, numbers by value, and a longer list wins a tie
    ids = tuple((0, x, "") if isinstance(x, int) else (1, 0, x) for x in pre)
    return (1, (major, minor, patch, 1 if not pre else 0, ids))


def is_prerelease(v):
    p = parse_version(v)
    return bool(p and p[3])


def newer(a, b):
    """a is a newer version than b (False if a isn't a version; anything beats a missing b)."""
    if parse_version(a) is None:
        return False
    return parse_version(b) is None or version_key(a) > version_key(b)


# --- talking to GitHub ----------------------------------------------------------------------

class ReleaseError(Exception):
    pass


class NoRelease(ReleaseError):
    """The repo has no (matching) release yet."""


def run(cmd, timeout=60):
    """subprocess.run with text output; never raises for a missing program (tests replace it)."""
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        return subprocess.CompletedProcess(cmd, 127, "", str(e))


def urlopen(req, timeout=30):
    return urllib.request.urlopen(req, timeout=timeout)  # tests replace this


def gh_path():
    return shutil.which("gh") or next((p for p in ("/opt/homebrew/bin/gh", "/usr/local/bin/gh") if Path(p).exists()),
                                      None)


class Gh:
    """The GitHub CLI with the user's own login (needed while the repo is private)."""
    name = "gh"

    def __init__(self, gh, repo=REPO):
        self.gh, self.repo = gh, repo

    def json(self, path):
        r = run([self.gh, "api", "-H", "Accept: application/vnd.github+json", path])
        if r.returncode != 0:
            raise ReleaseError(f"gh api {path}: {(r.stderr or r.stdout).strip()[-200:] or f'exit {r.returncode}'}")
        try:
            return json.loads(r.stdout)
        except ValueError as e:
            raise ReleaseError(f"gh api {path}: not JSON ({e})") from None

    def download(self, release, name, dest):
        r = run([self.gh, "release", "download", release["tag"], "--repo", self.repo, "--pattern", name,
                 "--dir", str(Path(dest).parent), "--clobber"], timeout=300)
        if r.returncode != 0:
            raise ReleaseError(f"gh release download {name}: {(r.stderr or r.stdout).strip()[-200:]}")
        if Path(dest).name != name:
            (Path(dest).parent / name).replace(dest)


class Https:
    """api.github.com without a login: works once the repo is public."""
    name = "https"

    def __init__(self, repo=REPO):
        self.repo = repo

    def _get(self, url, accept):
        req = urllib.request.Request(url, headers={"Accept": accept, "User-Agent": "stick-agent",
                                                   "X-GitHub-Api-Version": "2022-11-28"})
        try:
            return urlopen(req)
        except urllib.error.HTTPError as e:
            raise ReleaseError(f"{url}: HTTP {e.code}" + (" (private repo? log in with `gh auth login`)"
                                                           if e.code == 404 else "")) from None
        except (urllib.error.URLError, OSError) as e:
            raise ReleaseError(f"{url}: {getattr(e, 'reason', e)}") from None

    def json(self, path):
        with self._get(f"{API}/{path}", "application/vnd.github+json") as r:
            try:
                return json.loads(r.read())
            except ValueError as e:
                raise ReleaseError(f"{path}: not JSON ({e})") from None

    def download(self, release, name, dest):
        asset = release["assets"].get(name) or {}
        url = asset.get("download")
        if not url:
            raise ReleaseError(f"release {release['tag']} has no {name}")
        limit = int(asset["size"]) if asset.get("size") else 64 << 20
        with self._get(url, "application/octet-stream") as r, open(dest, "wb") as f:
            got = 0
            while chunk := r.read(64 << 10):
                got += len(chunk)
                if got > limit:
                    raise ReleaseError(f"{name} is bigger than the release says")
                f.write(chunk)


def sources(repo=REPO):
    """The ways to reach GitHub, best first: gh (when installed), then plain HTTPS."""
    gh = gh_path()
    return ([Gh(gh, repo)] if gh else []) + [Https(repo)]


def list_releases(repo=REPO):
    """-> (raw releases from the API, the source that worked). Raises ReleaseError."""
    errors = []
    for src in sources(repo):
        try:
            data = src.json(f"repos/{repo}/releases?per_page=30")
        except ReleaseError as e:
            errors.append(f"{src.name}: {e}")
            continue
        if not isinstance(data, list):
            errors.append(f"{src.name}: unexpected answer")
            continue
        return data, src
    raise ReleaseError("; ".join(errors) or "no way to reach GitHub")


def normalize(r):
    """A release from the API -> the fields we use (None for drafts and odd tags)."""
    if not isinstance(r, dict) or r.get("draft"):
        return None
    tag = r.get("tag_name") or ""
    version = tag[1:] if tag.startswith("v") else tag
    if parse_version(version) is None:
        return None
    assets = {}
    for a in r.get("assets") or []:
        name = a.get("name") or ""
        if SAFE_NAME.match(name):
            digest = a.get("digest") or ""
            assets[name] = {"size": a.get("size"), "url": a.get("url"), "download": a.get("browser_download_url"),
                            "sha256": digest[7:] if digest.startswith("sha256:") else None}
    return {"tag": tag, "version": version, "prerelease": bool(r.get("prerelease")) or is_prerelease(version),
            "name": r.get("name") or tag, "notes": r.get("body") or "", "url": r.get("html_url"),
            "published": r.get("published_at"), "assets": assets}


def pick(raw, channel="stable"):
    """The newest release for the channel: "stable" = no prereleases, "beta" = everything."""
    found = [n for n in (normalize(r) for r in raw or []) if n]
    if channel != "beta":
        found = [r for r in found if not r["prerelease"]]
    return max(found, key=lambda r: version_key(r["version"]), default=None)


def latest(channel="stable", repo=REPO):
    """-> (release, source). Raises NoRelease if there's none for the channel yet."""
    raw, src = list_releases(repo)
    rel = pick(raw, channel)
    if rel is None:
        raise NoRelease(NO_RELEASE if not raw else f"no {channel} release yet")
    return rel, src


# --- firmware from a release ------------------------------------------------------------------

def firmware_root():
    from . import config  # (late: tools/release_tool.py imports this module without the agent's config)
    return config.HOME / "firmware"


def check_manifest(m, release=None):
    """Raises ReleaseError unless the manifest is one we can trust to write files from."""
    if not isinstance(m, dict) or not SAFE_NAME.match(str(m.get("version") or "")):
        raise ReleaseError("the release's manifest.json has no usable firmware version")
    parts = m.get("parts")
    if not isinstance(parts, list) or not parts:
        raise ReleaseError("the release's manifest.json lists no firmware parts")
    for p in parts:
        name = str(p.get("file") or "")
        if not (SAFE_NAME.match(name) and name.endswith(".bin") and name != MANIFEST):
            raise ReleaseError(f"odd firmware part name in the manifest: {name!r}")
        if not re.fullmatch(r"0x[0-9a-fA-F]+", str(p.get("offset") or "")) or not isinstance(p.get("size"), int):
            raise ReleaseError(f"{name}: bad offset or size in the manifest")
        if not re.fullmatch(r"[0-9a-f]{64}", str(p.get("sha256") or "")):
            raise ReleaseError(f"{name}: no sha256 in the manifest")
        if release is not None and name not in release["assets"]:
            raise ReleaseError(f"release {release['tag']} lists {name} but doesn't have it")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def verify_dir(d, manifest=None):
    """The folder holds a manifest and every part it lists, with the right size and sha256."""
    d = Path(d)
    try:
        m = json.loads((d / MANIFEST).read_text())
        check_manifest(m)
    except (OSError, ValueError, ReleaseError):
        return False
    if manifest is not None and m.get("sha256") != manifest.get("sha256"):
        return False
    for p in m["parts"]:
        f = d / p["file"]
        if not f.is_file() or f.stat().st_size != p["size"] or sha256(f) != p["sha256"]:
            return False
    return True


def _fetch(src, release, name, dest):
    src.download(release, name, dest)
    want = release["assets"].get(name, {}).get("sha256")
    if want and sha256(dest) != want:  # GitHub's own digest of the asset
        raise ReleaseError(f"{name}: doesn't match GitHub's sha256 for the asset")


def download_firmware(release, src, root=None, say=None):
    """Downloads the release's firmware into <root>/<firmware version>/ (verified), unless it's
    already there. -> (folder, manifest). Raises ReleaseError."""
    say = say or (lambda line: None)
    root = Path(root or firmware_root())
    if MANIFEST not in release["assets"]:
        raise ReleaseError(f"release {release['tag']} has no firmware (no {MANIFEST})")
    root.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix=".download-", dir=root))
    try:
        _fetch(src, release, MANIFEST, tmp / MANIFEST)
        try:
            manifest = json.loads((tmp / MANIFEST).read_text())
        except ValueError as e:
            raise ReleaseError(f"the release's manifest.json isn't JSON ({e})") from None
        check_manifest(manifest, release)
        target = root / manifest["version"]
        if verify_dir(target, manifest):
            say(f"firmware {manifest['version']} ({release['tag']}) is already here")
            return target, manifest
        for p in manifest["parts"]:
            say(f"downloading {p['file']} ({p['size'] // 1024} KB) from {release['tag']} via {src.name}")
            _fetch(src, release, p["file"], tmp / p["file"])
            got = tmp / p["file"]
            if got.stat().st_size != p["size"] or sha256(got) != p["sha256"]:
                raise ReleaseError(f"{p['file']}: sha256 doesn't match the manifest — not using this download")
        say(f"firmware {manifest['version']}: all {len(manifest['parts'])} parts match the manifest's sha256")
        old = None
        if target.exists():
            old = root / f".old-{manifest['version']}-{os.getpid()}"
            target.replace(old)
        tmp.replace(target)
        if old:
            shutil.rmtree(old, ignore_errors=True)
        prune(root)
        return target, manifest
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def prune(root=None, keep=KEEP):
    """Keeps the newest `keep` downloaded firmware versions (never the local build)."""
    root = Path(root or firmware_root())
    found = sorted((d for d in root.iterdir() if d.is_dir() and parse_version(d.name)),
                   key=lambda d: version_key(d.name), reverse=True)
    for d in found[keep:]:
        shutil.rmtree(d, ignore_errors=True)


def ensure_latest_firmware(channel="stable", say=None, repo=REPO):
    """The latest release's firmware, downloaded if needed. -> (release, folder, manifest).
    Raises NoRelease / ReleaseError (the caller says "build it locally")."""
    rel, src = latest(channel, repo)
    folder, manifest = download_firmware(rel, src, say=say)
    return rel, folder, manifest
