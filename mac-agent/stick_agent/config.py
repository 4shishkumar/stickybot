"""Settings and file locations. Everything lives in ~/.stick-agent/."""
import json
import os
import secrets
import shutil
from pathlib import Path

HOME = Path(os.environ.get("STICK_AGENT_HOME", Path.home() / ".stick-agent"))
CONFIG = HOME / "config.json"
COMMANDS = HOME / "commands.json"
HISTORY = HOME / "history.jsonl"
LOG = HOME / "agent.log"
TOKEN = HOME / "token"
WORKSPACE = HOME / "workspace"  # Claude Code's working folder (has its CLAUDE.md)
DEFAULTS = Path(__file__).parent / "defaults"
PORT = int(os.environ.get("STICK_AGENT_PORT", 8660))

DEFAULT_CONFIG = {
    "stick": None,            # {"address": CoreBluetooth id, "name": "stick-key"} — set in the dashboard
    "model": "sonnet",        # Claude model for voice requests (fast); "opus" for harder tasks
    # Speech model: v2 is English-only (won't mistake a short "PR 1234" for Russian);
    # v3 understands 25 languages but guesses the language on its own.
    "stt_model": "mlx-community/parakeet-tdt-0.6b-v2",
    "claude_path": "",        # "" = find it automatically
    "speak_replies": False,   # read short replies aloud; longer ones stay on the screen
    "speak_output": "stick",  # "stick" (its speaker) or "mac" (the Mac's speakers)
    "speak_max_words": 40,    # "short" = at most this many words
    "speak_voice": "",        # macOS voice name, "" = the system voice
    "speak_quick": False,     # also speak quick-command confirmations ("Opened Slack.")
    "followup_minutes": 10,   # a request within this time continues the same conversation ("oneshot" mode)
    # How Claude Code runs (brain.py): "session" = one long-lived process, no start-up per request;
    # "oneshot" = `claude -p` per request (the original way, the fallback)
    "brain_mode": "session",
    "brain_prewarm": True,    # start the session when the stick connects, before the first request
    "brain_idle_min": 10,     # session unused this long -> stopped; the next request starts a new chat
    "stream_replies": True,   # show the reply on the stick while Claude writes it
    "stream_interval_ms": 300,  # at most one screen update per this many ms (Bluetooth)
    "speak_stream": True,     # speak sentence by sentence as the reply is written (with speak_replies)
    "safe_tools": ["WebSearch", "WebFetch", "Read", "Glob", "Grep"],  # run without asking
    "weather": True,          # weather on the stick's home screen (Open-Meteo, free)
    "weather_location": "auto",  # "auto" = from this Mac's internet connection, or a city name
    "ticker": True,           # the facts / news ticker on the home screen (sources below)
    # experimental: home screen reading when nothing needs you
    "idle_content": ["facts", "hn", "onthisday", "wikipedia"],
    "onboarding": {},         # wizard steps the user ticked by hand (see firmware_api.py)
    "stick_approvals": True,  # Claude Code permission prompts go to the stick + dashboard first
    # The stick tools for Claude sessions (mcp_server.py): an MCP server at http://127.0.0.1:8660/mcp,
    # given to voice requests; terminal sessions: `stick-agent mcp install`
    "mcp_server": True,
    # Wireless Wispr mic: an A hold away from USB streams the stick's mic over Bluetooth; the agent
    # plays it into BlackHole 2ch and, just for the hold, shows it as the "stick-mic-bluetooth" mic
    # (rank it first in Wispr's microphone list once). `stick-agent setup --with wireless_mic`
    "wireless_mic": False,
    "wispr_mic_name": "stick-mic-bluetooth",  # the device's name (changing it: rank it in Wispr again)
    "wispr_switch_ms": 250,   # after the device appears: time for Wispr to switch to it before the key goes down
    "wispr_tail_ms": 800,     # after the key-up: keep the device while Wispr finishes (then the usual mic again)
    # Events on the stick (see events.py): kind -> "celebrate" | "nice" | "headsup" | "off";
    # kinds not listed use events.DEFAULT_RULES
    "event_rules": {},
    "quiet_hours": {"on": False, "from": "19:00", "to": "09:00"},  # events show, but silently
    "claude_done_min_s": 120,  # "Claude finished" only for tasks at least this long
    "github": {"on": True, "interval_s": 120, "primary_repo": "wvrdz/loom"},  # PR watcher (gh CLI)
}


def ensure_home():
    HOME.mkdir(parents=True, exist_ok=True)
    WORKSPACE.mkdir(exist_ok=True)
    if not COMMANDS.exists():
        shutil.copy(DEFAULTS / "commands.json", COMMANDS)
    if not (WORKSPACE / "CLAUDE.md").exists():
        shutil.copy(DEFAULTS / "CLAUDE.md", WORKSPACE / "CLAUDE.md")
    if not TOKEN.exists():
        TOKEN.write_text(secrets.token_urlsafe(24))
        TOKEN.chmod(0o600)


def load():
    cfg = dict(DEFAULT_CONFIG)
    try:
        cfg.update(json.loads(CONFIG.read_text()))
    except (OSError, ValueError):
        pass
    cfg.pop("wispr_fallback_mic", None)  # the smart mic's fallback (removed)
    return cfg


def save(cfg):
    CONFIG.write_text(json.dumps(cfg, indent=2))


def token():
    return TOKEN.read_text().strip()


def claude_path(cfg):
    if cfg.get("claude_path"):
        return cfg["claude_path"]
    for p in (shutil.which("claude"), Path.home() / ".local/bin/claude", "/opt/homebrew/bin/claude",
              "/usr/local/bin/claude"):
        if p and Path(p).exists():
            return str(p)
    return None
