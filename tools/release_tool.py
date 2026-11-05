#!/usr/bin/env python3
"""Helpers for cutting a release (tools/release.sh) and for the release workflow.

  release_tool.py current                  the version in mac-agent (stick_agent.__version__)
  release_tool.py firmware                 the firmware's FW_VERSION (m5-stick-s3/src/config.h)
  release_tool.py check VERSION            exit 1 unless VERSION is X.Y.Z or X.Y.Z-pre
  release_tool.py bump VERSION             write VERSION into __init__.py and pyproject.toml
  release_tool.py changelog VERSION DATE   date VERSION's entry in CHANGELOG.md (or turn
                                           [Unreleased] into it); keeps the links at the bottom
  release_tool.py notes VERSION            print VERSION's changelog entry (the release notes)
  release_tool.py check-tag TAG            the workflow's guard: TAG == v<version in mac-agent>
                                           and CHANGELOG.md has an entry for it

Standard library only (runs with the system python3).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "mac-agent"))
from stick_agent.releases import parse_version  # noqa: E402 — stdlib only

INIT = ROOT / "mac-agent" / "stick_agent" / "__init__.py"
PYPROJECT = ROOT / "mac-agent" / "pyproject.toml"
CHANGELOG = ROOT / "CHANGELOG.md"
CONFIG_H = ROOT / "m5-stick-s3" / "src" / "config.h"
REPO_URL = "https://github.com/ashish-kumar-noon/stickybot"
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


def bump(version):
    check(version)
    for path, pattern, line in [(INIT, r'^__version__\s*=\s*"[^"]*"', f'__version__ = "{version}"'),
                                (PYPROJECT, r'^version\s*=\s*"[^"]*"', f'version = "{version}"')]:
        text = path.read_text()
        new, n = re.subn(pattern, line, text, count=1, flags=re.M)
        if n != 1:
            raise Fail(f"no version line in {path}")
        path.write_text(new)


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
    notes(version)
    return version


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
