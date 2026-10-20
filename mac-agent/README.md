# stick-agent — a voice assistant for your Mac, on an M5StickS3

Hold **B** on the stick and speak. The Mac turns your speech into text (locally,
nothing leaves the machine), runs a quick command if one matches ("PR 1234"),
and otherwise hands the request to **Claude Code** with your own login. Anything
that changes something asks first — **A** approves, **B** denies, on the stick
or in the dashboard. Replies show on the stick's screen (and short ones can be
read aloud on its speaker).

It also keeps an eye on things for you — GitHub PRs, long Claude Code tasks —
and tells you on the stick: confetti when your PR merges, a heads-up when a
review is requested.

The stick runs the firmware in [../m5-stick-s3](../m5-stick-s3); they talk over
the Bluetooth link described in [../PROTOCOL.md](../PROTOCOL.md) (section 5).

---

## What it can do

### Hold B and ask — quick commands (instant, no AI)
| Say | Does |
|---|---|
| "PR 1234", "pull request 1234" | opens that PR in Brave |
| "Devin review 1234" | opens the Devin review of that PR |
| "Open Slack / Figma / VS Code / Linear …" | opens any installed app |

Add your own in the dashboard → **Quick commands** (a URL, an app or a shell
command; `{n}` = a number, `{app}` / `{q}` = words). Tip: "pull request 1234" is
easier for the speech model than "P R 1234".

### Hold B and ask — everything else goes to Claude Code
- **Questions & the web:** "what's the weather", "what's the latest on …", "summarise <url>"
- **Your Mac:** "how much disk space do I have", "what's using my CPU", "find my notes about X"
- **Apps (AppleScript):** "play my focus playlist in Spotify", "set volume to 30%"
- **Follow-ups** within 10 minutes continue the conversation ("and tomorrow?")
- Web search and reading run without asking; commands that change things ask on the stick (A yes / B no, 30 s, then Claude Code asks in the terminal).
- **Click B** on any agent screen to cancel and go back to Home. **Shake** does the same.

### The other buttons
- **A (hold):** Wispr Flow dictation (the key combo; over USB the stick is also the mic).
- **B (double press):** types your Mac password to unlock it.
- **B (triple press):** the stick's menu.

### It tells you things (stick screen + sound)
| | When |
|---|---|
| 🎉 celebrate (confetti + jingle) | your PR merged · CI green again after a failure |
| ✅ nice (toast + chime) | PR created · PR approved · Claude Code finished a task that took ≥ 2 min · Devin review posted |
| ⚠️ heads-up (orange, until you press a button) | review requested from you (primary repo) · changes requested · CI failed |

Levels per event, quiet hours and the GitHub settings are in the dashboard → **Events**.

### At a glance on the stick
- **Home:** clock, weather, and a card that rotates through approvals, your last answer, open Claude Code sessions and (optional) facts / Hacker News / Wikipedia.
- **Agent page:** your last question and answer. **Status page:** battery, connections, last-hour battery chart.
- **Charging:** a full-screen desk clock (dims 22:00–06:00).

---

## Install

Requirements: a Mac with Apple Silicon, Python 3.11+ (`brew install python@3.14`),
[Claude Code](https://claude.com/claude-code) installed and logged in, the stick
paired with the Mac (System Settings → Bluetooth), and `gh` logged in for GitHub
alerts (optional).

```sh
/opt/homebrew/bin/python3.14 -m venv ~/.stick-agent/venv
~/.stick-agent/venv/bin/pip install -e .          # from this folder
ln -sf ~/.stick-agent/venv/bin/stick-agent ~/.local/bin/stick-agent
stick-agent start                                  # also starts at every login
```

The first start downloads the speech model (~2.5 GB, once) and opens the
dashboard: **Settings → Find sticks nearby → Use this one**, then set your repo in
**Quick commands**. macOS asks once for Bluetooth access — allow it.

If `stick-agent start` says the LaunchAgents folder isn't writable, fix it once:
`sudo chown $(whoami) ~/Library/LaunchAgents`.

### Claude Code hooks (in `~/.claude/settings.json`)
| Hook | Script | What for |
|---|---|---|
| `PermissionRequest` (timeout 45) | `stick_agent/hook.py` | approvals on the stick / dashboard |
| `UserPromptSubmit` + `Stop` (timeout 5) | `stick_agent/claude_hooks.py` | "Claude finished" alerts |

Command for each: `~/.stick-agent/venv/bin/python <path to the script>`. They never
block Claude Code: if the agent isn't running they do nothing.

---

## Using it
- **Menu bar** icon: state at a glance, stick volume, sounds on/off, pause, restart, dashboard.
- **Dashboard:** http://localhost:8660 — activity, history, quick commands, sounds, events, settings, logs. Only reachable from this Mac; every change needs the token in `~/.stick-agent/token` (built into the page).
- **Terminal:** `stick-agent status | start | stop | restart | logs | open`.

Files: everything lives in `~/.stick-agent/` — `config.json`, `commands.json`,
`history.jsonl`, `events.jsonl`, `agent.log`, `sounds/` (Kenney CC0 packs),
`workspace/CLAUDE.md` (the instructions Claude gets for voice requests).

## How it works
| Module | |
|---|---|
| `link.py` | Bluetooth to the chosen stick (bleak): JSON messages, mic audio in, sound clips out |
| `stt.py` | speech to text — Parakeet (MLX), English-only v2 by default (`stt_model`) |
| `commands.py` | quick commands: phrase matching, spoken numbers, app lookup |
| `brain.py` | Claude Code headless (`claude -p`, stream-json), follow-ups, cancel |
| `core.py` | the agent: talk → text → command or Claude → reply; approvals; sounds; speech |
| `events.py`, `github.py`, `claude_hooks.py` | alerts: GitHub (via `gh`), Claude finished |
| `home.py`, `feed.py`, `weather.py` | the stick's home screen card, facts/news, weather (Open-Meteo) |
| `server.py`, `dashboard.html`, `menubar.py` | dashboard API (localhost + token) and menu bar |

## Not built yet
Next meeting on the home screen (calendar), wake word, Mac-control tools for
Claude beyond AppleScript, wireless Wispr (via BlackHole).
