#!/usr/bin/env python3
"""Helpers for cutting a release (tools/release.sh) and for the release workflow.

  release_tool.py current                  the version in mac-agent (stick_agent.__version__)
  release_tool.py firmware                 the firmware's FW_VERSION (m5-stick-s3/src/config.h)
  release_tool.py check VERSION            exit 1 unless VERSION is X.Y.Z or X.Y.Z-pre
  release_tool.py bump VERSION             write VERSION into __init__.py, pyproject.toml and the
                                           Rust app (desktop/Cargo.toml + Cargo.lock)
  release_tool.py desktop                  the Rust app's version (desktop/Cargo.toml)
  release_tool.py changelog VERSION DATE   date VERSION's entry in CHANGELOG.md (or turn
                                           [Unreleased] into it); keeps the links at the bottom
  release_tool.py notes VERSION            print VERSION's changelog entry (the release notes)
  release_tool.py check-tag TAG            the workflow's guard: TAG == v<version in mac-agent>
                                           == v<desktop version>, and CHANGELOG.md has an entry
  release_tool.py add-app MANIFEST ZIP PLATFORM VERSION
                                           list a Mac app download (the release asset ZIP) in
                                           the release's manifest.json, under "apps"
  release_tool.py latest-json OUT --version X.Y.Z --tag vX.Y.Z --sig FILE.sig --asset NAME
                                  [--repo OWNER/NAME] [--pub-date ISO]
                                           write latest.json, the file the Mac app's updater
                                           (Tauri's static update JSON) reads: the version, its
                                           changelog entry as notes, and where to get the signed
                                           StickyBot.app.tar.gz and its signature

Standard library only (runs with the system python3).
"""
import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "mac-agent"))
from stick_agent.releases import parse_version  # noqa: E402 — stdlib only

INIT = ROOT / "mac-agent" / "stick_agent" / "__init__.py"
PYPROJECT = ROOT / "mac-agent" / "pyproject.toml"
CHANGELOG = ROOT / "CHANGELOG.md"
CONFIG_H = ROOT / "m5-stick-s3" / "src" / "config.h"
DESKTOP_CARGO = ROOT / "desktop" / "Cargo.toml"
DESKTOP_LOCK = ROOT / "desktop" / "Cargo.lock"
DESKTOP_CRATES = ("stickybot-server", "stickybot-app")  # `version.workspace = true`
DESKTOP_VERSION = re.compile(r'(\[workspace\.package\][^\[]*?^version\s*=\s*")([^"]*)(")', re.M | re.S)
REPO_URL = "https://github.com/4shishkumar/stickybot"
HEADING = re.compile(r"^## \[([^\]]+)\](.*)$", re.M)


class Fail(Exception):
    pass


def current():
    m = re.search(r'^__version__\s*=\s*"([^"]+)"', INIT.read_text(), re.M)
    if not m:
        raise Fail(f"no __version__ in {INIT}")
    return m.group(1)


def firmware():
    m = re.search(r'#define\s+FW_VERSION\s+"([^"]+)"', CONFIG_H.read_text())
    return m.group(1) if m else "unknown"


def check(version):
    if parse_version(version) is None or version.startswith("v"):
        raise Fail(f"{version!r} isn't a version like 1.8.0 or 1.8.0-beta.1")


def desktop_version():
    """The Rust app's version: desktop/Cargo.toml [workspace.package] (CARGO_PKG_VERSION)."""
    m = DESKTOP_VERSION.search(DESKTOP_CARGO.read_text())
    if not m:
        raise Fail(f"no [workspace.package] version in {DESKTOP_CARGO}")
    return m.group(2)


def bump(version):
    """The release version into the Python app (__init__.py, pyproject.toml) and the Rust app
    (desktop/Cargo.toml's [workspace.package] and the Cargo.lock entries of the crates that use it,
    so StickyBot.app, /api/rust and the update check say this version)."""
    check(version)
    edits = [(INIT, re.compile(r'^(__version__\s*=\s*")[^"]*(")', re.M)),
             (PYPROJECT, re.compile(r'^(version\s*=\s*")[^"]*(")', re.M)),
             (DESKTOP_CARGO, DESKTOP_VERSION)]
    edits += [(DESKTOP_LOCK, re.compile(rf'^(name = "{re.escape(c)}"\nversion = ")[^"]*(")', re.M))
              for c in DESKTOP_CRATES]
    out = {}
    for path, pattern in edits:
        text = out.get(path, path.read_text())
        new, n = pattern.subn(lambda m: m.group(1) + version + m.group(m.lastindex), text, count=1)
        if n != 1:
            raise Fail(f"no version line for {pattern.pattern!r} in {path}")
        out[path] = new
    for path, text in out.items():  # all or nothing
        path.write_text(text)


def sections(text):
    """[(version, start of heading, start of body, end)] for every '## [x]' section."""
    heads = list(HEADING.finditer(text))
    out = []
    for i, m in enumerate(heads):
        end = heads[i + 1].start() if i + 1 < len(heads) else len(text)
        # the link references at the bottom aren't part of the last section
        refs = re.search(r"^\[[^\]]+\]: ", text[m.end():end], re.M)
        if refs:
            end = m.end() + refs.start()
        out.append((m.group(1), m.start(), m.end(), end))
    return out


def notes(version, text=None):
    text = CHANGELOG.read_text() if text is None else text
    for v, _start, body, end in sections(text):
        if v == version:
            return text[body:end].strip("\n").strip()
    raise Fail(f"CHANGELOG.md has no entry for {version}")


def changelog(version, date, text=None):
    """-> the new CHANGELOG text: VERSION's entry dated DATE."""
    check(version)
    text = CHANGELOG.read_text() if text is None else text
    secs = {v: (start, body, end) for v, start, body, end in sections(text)}
    if version in secs:
        start, body, _ = secs[version]
        text = text[:start] + f"## [{version}] - {date}" + text[body:]
    elif "Unreleased" in secs:
        start, body, end = secs["Unreleased"]
        if not text[body:end].strip():
            raise Fail("nothing under [Unreleased] in CHANGELOG.md — describe the changes first")
        text = text[:start] + f"## [Unreleased]\n\n## [{version}] - {date}" + text[body:]
    else:
        raise Fail(f"CHANGELOG.md has neither [Unreleased] nor [{version}]")
    # links: [Unreleased] compares from this release; this release has its own
    text = re.sub(r"^\[Unreleased\]: .*$", f"[Unreleased]: {REPO_URL}/compare/v{version}...HEAD", text, flags=re.M)
    if not re.search(rf"^\[{re.escape(version)}\]: ", text, re.M):
        link = f"[{version}]: {REPO_URL}/releases/tag/v{version}"
        m = re.search(r"^\[Unreleased\]: .*$", text, re.M)
        text = text[:m.end()] + "\n" + link + text[m.end():] if m else text.rstrip("\n") + "\n\n" + link + "\n"
    return text


def check_tag(tag):
    version = tag[1:] if tag.startswith("v") else ""
    check(version)
    have = current()
    if have != version:
        raise Fail(f"tag {tag} but mac-agent says {have} — cut releases with tools/release.sh")
    rust = desktop_version()
    if rust != version:
        raise Fail(f"tag {tag} but desktop/Cargo.toml says {rust} — cut releases with tools/release.sh")
    notes(version)
    return version


def add_app(manifest_path, zip_path, platform, version):
    """Lists ZIP (a release asset next to the manifest) in manifest.json's "apps": file, size,
    sha256, platform, version. The firmware part of the manifest is left as it is: agents read
    only "version", "parts", "sha256" and "flash" and ignore other keys."""
    check(version)
    manifest_path, zip_path = Path(manifest_path), Path(zip_path)
    m = json.loads(manifest_path.read_text())
    data = zip_path.read_bytes()
    entry = {"name": "StickyBot", "platform": platform, "version": version, "file": zip_path.name,
             "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    apps = [a for a in m.get("apps") or [] if a.get("platform") != platform]
    m["apps"] = apps + [entry]
    manifest_path.write_text(json.dumps(m, indent=2) + "\n")
    return entry


UPDATER_PLATFORM = "darwin-aarch64"  # Tauri's name for an Apple silicon Mac (the only one StickyBot.app is built for)
NAME_PART = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
BASE64 = re.compile(r"^[A-Za-z0-9+/]+={0,2}$")


def release_url(repo, tag, asset):
    """https://github.com/<repo>/releases/download/<tag>/<asset>, or Fail if any part isn't a plain name."""
    owner, _, name = repo.partition("/")
    if not (NAME_PART.match(owner) and NAME_PART.match(name)):
        raise Fail(f"{repo!r} isn't a repo like owner/name")
    if not NAME_PART.match(tag):
        raise Fail(f"{tag!r} isn't a plain tag name")
    if not NAME_PART.match(asset) or not asset.endswith(".app.tar.gz"):
        raise Fail(f"{asset!r} isn't the name of a StickyBot-<version>-macos-arm64.app.tar.gz release asset")
    return f"https://github.com/{repo}/releases/download/{tag}/{asset}"


def rfc3339_utc(value=None):
    """now, or VALUE (an ISO 8601 time with a zone), as 2026-11-20T10:00:00Z."""
    if value is None:
        when = datetime.now(timezone.utc)
    else:
        try:
            when = datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            raise Fail(f"{value!r} isn't a time like 2026-11-20T10:00:00Z") from None
        if when.tzinfo is None:
            raise Fail(f"{value!r} has no time zone (use Z for UTC)")
    return when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def latest_json(out, version, tag, sig_path, asset, repo="4shishkumar/stickybot", pub_date=None):
    """Writes OUT: Tauri's static update JSON for the Mac app's updater. `signature` is the .sig file's
    text (base64, as the Tauri signer wrote it); `url` is the release asset. The updater checks the
    signature against the public key built into the app, so a wrong or empty one is refused here
    instead of shipping a release nobody can install."""
    check(version)
    if tag != f"v{version}":
        raise Fail(f"tag {tag} isn't v{version}")
    signature = Path(sig_path).read_text().strip()
    if not signature:
        raise Fail(f"{sig_path} is empty — the update isn't signed (is TAURI_SIGNING_PRIVATE_KEY set?)")
    if not BASE64.match(signature):
        raise Fail(f"{sig_path} doesn't look like a Tauri .sig file (one line of base64)")
    data = {"version": version, "notes": notes(version), "pub_date": rfc3339_utc(pub_date),
            "platforms": {UPDATER_PLATFORM: {"signature": signature, "url": release_url(repo, tag, asset)}}}
    Path(out).write_text(json.dumps(data, indent=2) + "\n")
    return data


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd, args = argv[0], argv[1:]
    try:
        if cmd == "current":
            print(current())
        elif cmd == "firmware":
            print(firmware())
        elif cmd == "desktop":
            print(desktop_version())
        elif cmd == "check":
            check(args[0])
        elif cmd == "bump":
            bump(args[0])
        elif cmd == "changelog":
            CHANGELOG.write_text(changelog(args[0], args[1]))
        elif cmd == "notes":
            print(notes(args[0]))
        elif cmd == "check-tag":
            print(check_tag(args[0]))
        elif cmd == "add-app":
            print(json.dumps(add_app(args[0], args[1], args[2], args[3])))
        elif cmd == "latest-json":
            ap = argparse.ArgumentParser(prog="release_tool.py latest-json")
            ap.add_argument("out")
            ap.add_argument("--version", required=True)
            ap.add_argument("--tag", required=True)
            ap.add_argument("--sig", required=True)
            ap.add_argument("--asset", required=True)
            ap.add_argument("--repo", default="4shishkumar/stickybot")
            ap.add_argument("--pub-date")
            a = ap.parse_args(args)
            latest_json(a.out, a.version, a.tag, a.sig, a.asset, a.repo, a.pub_date)
        else:
            print(__doc__)
            return 2
    except Fail as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    except IndexError:
        print(f"error: {cmd} needs an argument (see --help)", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
