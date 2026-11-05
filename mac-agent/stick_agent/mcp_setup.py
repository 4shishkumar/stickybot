"""Giving Claude Code the stick tools (mcp_server.py).

- Voice requests: brain.py passes `--mcp-config <brain_config()>` (a file in ~/.stick-agent, 0600).
- Terminal sessions: `stick-agent mcp install` runs Claude Code's own command
      claude mcp add --transport http --scope user stick http://127.0.0.1:8660/mcp \
          --header "Authorization: Bearer <token>" --header "X-Stick-Cwd: ${PWD}"
  so Claude Code writes its own config; we never edit ~/.claude.json or ~/.claude/settings.json.
  `${PWD}` is left for Claude Code to expand when it starts (env var expansion in MCP headers:
  https://code.claude.com/docs/en/mcp), so the stick can say which project is asking.
  `stick-agent mcp uninstall` = `claude mcp remove --scope user stick`; `status` = `claude mcp get stick`.

Command forms from `claude mcp add --help` (Claude Code 2.1.317) and https://code.claude.com/docs/en/mcp
("Option 1: Add a remote HTTP server", "MCP installation scopes"). --header is variadic, so it goes
after the name and URL.

  stick-agent mcp install [--yes]     add the stick tools to Claude Code (user scope, all projects)
  stick-agent mcp uninstall [--yes]   remove them again
  stick-agent mcp status              installed? (and the command that would run)
"""
import json
import subprocess
import sys

from . import config

NAME = "stick"


def url():
    return f"http://127.0.0.1:{config.PORT}/mcp"


def brain_config():
    """The --mcp-config file for the agent's own voice session (holds the token: 0600)."""
    path = config.HOME / "mcp-voice.json"
    cfg = {"mcpServers": {NAME: {"type": "http", "url": url(),
                                 "headers": {"Authorization": f"Bearer {config.token()}", "X-Stick-Client": "voice"}}}}
    text = json.dumps(cfg, indent=2)
    try:
        if path.read_text() == text:
            return path
    except OSError:
        pass
    path.touch(mode=0o600, exist_ok=True)
    path.chmod(0o600)
    path.write_text(text)
    return path


def add_args(claude, token):
    return [claude, "mcp", "add", "--transport", "http", "--scope", "user", NAME, url(),
            "--header", f"Authorization: Bearer {token}", "--header", "X-Stick-Cwd: ${PWD}"]


def remove_args(claude):
    return [claude, "mcp", "remove", "--scope", "user", NAME]


def get_args(claude):
    return [claude, "mcp", "get", NAME]


def shown(args):
    """The command for people to read: the token is replaced (it's in ~/.stick-agent/token)."""
    out = []
    for a in args[1:]:
        a = a.replace(config.token(), "<token>") if a.startswith("Authorization:") else a
        out.append(f'"{a}"' if any(c in a for c in " ${}") else a)
    return "claude " + " ".join(out)


def _claude():
    return config.claude_path(config.load())


def _run(args, timeout=60):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def status():
    """-> {"installed": bool | None, "detail", "command"} (None: Claude Code not found / didn't answer)."""
    claude = _claude()
    if not claude:
        return {"installed": None, "detail": "Claude Code not found", "command": shown(add_args("claude", "x"))}
    cmd = shown(add_args(claude, config.token()))
    try:
        r = _run(get_args(claude), timeout=30)
    except (OSError, subprocess.TimeoutExpired) as e:
        return {"installed": None, "detail": f"`claude mcp get` didn't answer: {e}", "command": cmd}
    out = (r.stdout or "") + (r.stderr or "")
    if r.returncode != 0:
        return {"installed": False, "detail": "not added to Claude Code", "command": cmd}
    if url() not in out:
        return {"installed": False, "detail": f"a server named {NAME!r} exists but points elsewhere", "command": cmd,
                "conflict": True}
    scope = "user" if "user" in out.lower() else "local/project"
    return {"installed": True, "detail": f"added ({scope} scope)", "command": cmd}


def install(confirm=None, say=print):
    """Runs `claude mcp add …`. confirm(text) -> bool is asked first (None = don't ask). -> (ok, message)."""
    claude = _claude()
    if not claude:
        return False, "Claude Code isn't installed (https://code.claude.com)"
    st = status()
    if st["installed"]:
        return True, "already added to Claude Code"
    if st.get("conflict"):  # someone else's server with our name (or another port): never replace it
        return False, (f"Claude Code already has an MCP server named {NAME!r} that isn't {url()} — "
                       f"remove it first (claude mcp remove {NAME}) if it's an old one of ours")
    args = add_args(claude, config.token())
    say(f"  {shown(args)}")
    if confirm is not None and not confirm("Run this?"):
        return False, "cancelled"
    r = _run(args)
    if r.returncode != 0:
        return False, (r.stderr or r.stdout).strip()[-300:] or f"exit {r.returncode}"
    return True, "added: new Claude Code sessions have the stick tools (mcp__stick__*)"


def uninstall(confirm=None, say=print):
    claude = _claude()
    if not claude:
        return False, "Claude Code isn't installed"
    st = status()
    if st["installed"] is False and not st.get("conflict"):
        return True, "wasn't added — nothing to remove"
    if st.get("conflict"):
        return False, f"the {NAME!r} server in Claude Code isn't ours ({url()}): left alone"
    args = remove_args(claude)
    say(f"  {shown(args)}")
    if confirm is not None and not confirm("Run this?"):
        return False, "cancelled"
    r = _run(args)
    if r.returncode != 0:
        return False, (r.stderr or r.stdout).strip()[-300:] or f"exit {r.returncode}"
    return True, "removed from Claude Code"


def cli(argv):
    """stick-agent mcp install|uninstall|status [--yes]"""
    what = argv[0] if argv else "status"
    yes = "--yes" in argv or "-y" in argv

    def confirm(q):
        if yes:
            return True
        if not sys.stdin.isatty():
            print("(not a terminal: add --yes to run it)")
            return False
        return input(f"{q} [y/N] ").strip().lower() in ("y", "yes")

    if what == "status":
        st = status()
        state = {True: "installed", False: "not installed", None: "unknown"}[st["installed"]]
        print(f"stick tools for Claude Code: {state} — {st['detail']}")
        print(f"server: {url()} ({'on' if config.load().get('mcp_server', True) else 'off in the dashboard'})")
        if not st["installed"]:
            print(f"install: stick-agent mcp install   (runs: {st['command']})")
        return 0
    if what in ("install", "uninstall"):
        print(f"Will run (Claude Code's own command; the token comes from {config.TOKEN}):")
        ok, msg = (install if what == "install" else uninstall)(confirm=confirm)
        print(msg)
        return 0 if ok else 1
    print(__doc__)
    return 2
