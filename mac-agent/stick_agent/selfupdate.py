"""One-click agent update (dashboard → Update): move the install to a release tag,
reinstall, restart.

Only for installs made by install.sh — a clone in ~/.stick-agent/src that nobody
edits. There it runs:
    git -C ~/.stick-agent/src fetch --tags --force origin
    git -C ~/.stick-agent/src checkout <tag>          (a release tag: detached HEAD)
    uv pip install --python <the agent's python> -e ~/.stick-agent/src/mac-agent
then builds StickyBot.app if the Python changed (`stickybot app build`, app.py; never
fatal) and restarts the agent the usual way (Agent.restart_soon: execv, like Restart in
the menu bar). If a step fails the checkout goes back to where it was.

The repo was renamed (stick-key -> stickybot): a clone whose origin still has the old
name gets the new one first (GitHub redirects the old name, but not forever).

Anything else is left alone, with instructions instead:
  - a developer's checkout (the agent runs from a clone somewhere else): your branch,
    your call — `git pull` / `git checkout vX.Y.Z` yourself, then `stickybot restart`
  - the install clone with local changes (tracked files edited)
  - not a git checkout at all (installed from a package): re-run install.sh
"""
import asyncio
import logging
import re
import shutil
import subprocess
import sys
from pathlib import Path

from . import config

log = logging.getLogger("selfupdate")
PKG = Path(__file__).resolve().parent
OLD_ORIGIN = re.compile(r"^(?P<head>(?:https://|ssh://git@|git@)github\.com[:/])ashish-kumar-noon/stick-key"
                        r"(?P<tail>\.git)?/?$", re.IGNORECASE)


class UpdateError(Exception):
    pass


def source_root():
    """The checkout the running agent comes from (<root>/mac-agent/stick_agent), or None."""
    root = PKG.parent.parent
    return root if (root / "mac-agent" / "pyproject.toml").exists() else None


def managed_root():
    return config.HOME / "src"  # install.sh's clone


def git_out(root, *args, timeout=20):
    """`git -C root ...` -> (exit code, output). Tests replace it."""
    try:
        r = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, timeout=timeout)
        return r.returncode, (r.stdout or r.stderr).strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return 127, str(e)


def renamed_origin(url):
    """The origin URL under the repo's new name, or None if it doesn't need changing."""
    m = OLD_ORIGIN.match((url or "").strip())
    return f"{m['head']}ashish-kumar-noon/stickybot{m['tail'] or ''}" if m else None


def fix_origin(root, emit=None):
    """Points origin at ashish-kumar-noon/stickybot if it still has the old name. -> the new URL or None."""
    code, url = git_out(root, "remote", "get-url", "origin")
    new = renamed_origin(url) if code == 0 else None
    if new and git_out(root, "remote", "set-url", "origin", new)[0] == 0:
        log.info("origin: %s -> %s (the repo was renamed)", url, new)
        if emit:
            emit({"line": f"origin is now {new} (the repo was renamed)"})
        return new
    return None


def find_uv():
    return shutil.which("uv") or next((str(p) for p in (Path.home() / ".local/bin/uv", Path("/opt/homebrew/bin/uv"))
                                       if p.exists()), None)


def install_info(root=None):
    """How this agent is installed, and whether the dashboard may update it.
    -> {"kind": "release" | "dev" | "package", "path", "can_update", "why", "head", "branch", "instructions"}"""
    root = Path(root) if root else source_root()
    if root is None or not (root / ".git").exists():
        return {"kind": "package", "path": str(root or PKG), "can_update": False, "head": None, "branch": None,
                "why": "not installed from a git checkout",
                "instructions": ["Re-run the installer to update: ./install.sh (or the one-line install command)"]}
    _, head = git_out(root, "describe", "--tags", "--always")
    code, branch = git_out(root, "symbolic-ref", "--short", "-q", "HEAD")
    branch = branch if code == 0 else None
    info = {"path": str(root), "head": head or None, "branch": branch}
    try:
        managed = root.resolve() == managed_root().resolve()
    except OSError:
        managed = False
    if not managed:
        return {**info, "kind": "dev", "can_update": False,
                "why": "a developer checkout — the dashboard doesn't touch it",
                "instructions": [f"cd {root}",
                                 "git pull --ff-only            (or: git fetch --tags && git checkout vX.Y.Z)",
                                 f"{sys.executable} -m pip install -e mac-agent   (only if its dependencies changed)",
                                 "stickybot restart"]}
    code, dirty = git_out(root, "status", "--porcelain", "--untracked-files=no")
    if code != 0:
        return {**info, "kind": "release", "can_update": False, "why": f"git status failed: {dirty}",
                "instructions": [f"check {root} with git status, or re-run install.sh"]}
    if dirty:
        return {**info, "kind": "release", "can_update": False, "why": f"{root} has local changes",
                "instructions": [f"cd {root} && git status   — commit or discard them, then try again"]}
    return {**info, "kind": "release", "can_update": True, "why": "", "instructions": []}


def plan(tag, root, python=None, uv=None):
    """The commands an update runs, in order: [(what, argv)]."""
    root, python = str(root), str(python or sys.executable)
    install = ([uv, "pip", "install", "--python", python, "-e", f"{root}/mac-agent"] if uv
               else [python, "-m", "pip", "install", "-e", f"{root}/mac-agent"])
    return [("fetch the release tags", ["git", "-C", root, "fetch", "--tags", "--force", "origin"]),
            (f"check out {tag}", ["git", "-C", root, "-c", "advice.detachedHead=false", "checkout", "--quiet", tag]),
            ("install it", install)]


async def run_cmd(cmd, emit):
    """Runs one command, streaming its output lines; -> exit code. Tests replace it."""
    try:
        proc = await asyncio.create_subprocess_exec(*cmd, stdout=asyncio.subprocess.PIPE,
                                                    stderr=asyncio.subprocess.STDOUT)
    except OSError as e:
        emit({"line": str(e)})
        return 127
    async for raw in proc.stdout:
        line = raw.decode(errors="replace").rstrip()
        if line:
            emit({"line": line})
    return await proc.wait()


def checked_out_version(root):
    try:
        text = (Path(root) / "mac-agent" / "stick_agent" / "__init__.py").read_text()
    except OSError:
        return None
    m = re.search(r'^__version__\s*=\s*"([^"]+)"', text, re.MULTILINE)
    return m.group(1) if m else None


async def update(tag, emit, restart, root=None):
    """Moves the install clone to `tag`, reinstalls and calls restart(). emit() gets
    {"line"} / {"progress","stage"}. Raises UpdateError (after undoing the checkout)."""
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag or ""):
        raise UpdateError(f"not a release tag: {tag!r}")
    info = install_info(root)
    if not info["can_update"]:
        raise UpdateError(info["why"])
    root = Path(info["path"])
    code, prev = git_out(root, "rev-parse", "HEAD")
    if code != 0:
        raise UpdateError(f"git rev-parse failed: {prev}")
    fix_origin(root, emit)
    steps = plan(tag, root, uv=find_uv())
    for i, (what, cmd) in enumerate(steps):
        emit({"progress": int(100 * i / len(steps)), "stage": what})
        emit({"line": "$ " + " ".join(cmd)})
        if await run_cmd(cmd, emit) != 0:
            if i >= 1:  # the checkout may have moved: put it back (and its install)
                emit({"line": f"{what} failed — going back to {prev[:10]}"})
                await run_cmd(["git", "-C", str(root), "-c", "advice.detachedHead=false", "checkout", "--quiet", prev],
                              emit)
                if i == 2:
                    await run_cmd(steps[2][1], emit)
            raise UpdateError(f"{what} failed (see the log above)")
        if i == 1:
            have = checked_out_version(root)
            if have != tag[1:]:
                emit({"line": f"{tag} says it's version {have} — going back"})
                await run_cmd(["git", "-C", str(root), "-c", "advice.detachedHead=false", "checkout", "--quiet", prev],
                              emit)
                raise UpdateError(f"{tag} has version {have}, not {tag[1:]}")
    emit({"progress": 95, "stage": "StickyBot.app"})
    app_step = [str(sys.executable), "-m", "stick_agent", "app", "build"]  # the new code's builder
    emit({"line": "$ " + " ".join(app_step)})
    await run_cmd(app_step, emit)  # never fatal: without it the agent runs as Python
    emit({"progress": 100, "stage": "restart"})
    emit({"line": f"StickyBot {tag[1:]} installed — restarting the agent (the page reloads by itself)"})
    log.info("updated to %s (from %s); restarting", tag, prev[:10])
    restart()

