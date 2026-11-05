"""Releases of stick-key on GitHub: versions, picking the latest per channel, and
downloading a release's firmware into ~/.stick-agent/firmware/<version>/.

Kept to the standard library: tools/release_tool.py imports it without the venv.
"""
import re

SEMVER = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$")


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
