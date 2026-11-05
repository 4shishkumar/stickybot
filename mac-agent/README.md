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
- **Follow-ups** within 10 minutes continue the conversation ("and tomorrow?"); say **"new chat"** or **"start over"** to begin again
- The answer appears on the stick **while Claude writes it**, and with *Read replies aloud* on it's spoken sentence by sentence
  (Settings → Claude: *Claude Code runs as one session* keeps it ready — no start-up per request; *once per request* is the old way)
- Web search and reading run without asking; commands that change things ask on the stick (A yes / B no, 30 s, then Claude Code asks in the terminal).
- **Click B** on any agent screen to cancel and go back to Home. **Shake** does the same.

### The other buttons
- **A (hold):** Wispr Flow dictation (the key combo; over USB the stick is also the mic, and with the
  [wireless Wispr mic](#wireless-wispr-mic) it is the mic without a cable too).
- **B (double press):** types your Mac password to unlock it.
- **B (triple press):** the stick's menu.

### It tells you things (stick screen + sound)
| | When |
|---|---|
| 🎉 celebrate (confetti + jingle) | your PR merged · CI green again after a failure |
| ✅ nice (toast + chime) | PR created · PR approved · Claude Code finished a task that took ≥ 2 min · Devin review posted |
| ⚠️ heads-up (orange, until you press a button) | review requested from you (primary repo) · changes requested · CI failed |

Levels per event, quiet hours and the GitHub settings are in the dashboard → **Events**.

### The stick as a pager for any Claude session (MCP)
The agent runs a small MCP server, **stick**, at `http://127.0.0.1:8660/mcp` (this Mac only,
needs the agent's token). Claude can use it to reach you on the stick:

| Tool | Does | Asks you first? |
|---|---|---|
| `notify(text, level=info\|nice\|celebrate\|heads_up, sound?)` | a toast / alert on the stick | no |
| `ask(question, options=[A, B], timeout_s=60)` | the question on the stick; A or B answers (shake dismisses) → the label, `timeout`, `dismissed` or `busy: …` | no |
| `celebrate(text?)` | confetti + fanfare | no |
| `progress(text, done?, total?)` | a progress line on the home card (gone 10 min after the last update) | no |
| `pin_home(text, minutes=30)` | pins a line on the home card (`""` removes it) | no |
| `stick_status()` | connected, battery, USB, what's on screen, last button press, "at the desk?" | no |
| `speak(text)` | says it on the stick (or the Mac, per *Speech* settings); silent in quiet hours | no |
| `open_url(url)`, `open_app(name)`, `quick_command(phrase)` | change the Mac (http(s) only / installed apps / your quick commands) | **yes** — Claude Code's permission prompt, on the stick |

- **Voice requests** have them already (the agent's session gets `--mcp-config`, the feedback tools pre-allowed).
- **Terminal sessions:** run `stick-agent mcp install` once (or dashboard → Settings → *Stick tools for Claude
  Code* → Install). It shows and then runs Claude Code's own command —
  `claude mcp add --transport http --scope user stick http://127.0.0.1:8660/mcp --header "Authorization: Bearer <token>" --header "X-Stick-Cwd: ${PWD}"` —
  so every new Claude Code session has `mcp__stick__*`. Then ask e.g. *"run the tests and notify me on the stick when
  done"*. `stick-agent mcp status` / `stick-agent mcp uninstall` (`--yes` skips the question). Nothing of ours edits
  `~/.claude.json`.
- Alerts are limited to one a second (20 a minute); without the stick every tool answers `stick not connected`.
  Turn the server off in Settings, or only the alerts in **Events** (*claude_tool* → off). Every call is in **Activity**.
- Questions wait behind an approval prompt; one question at a time. Firmware 1.7.2 shows the option labels on the
  buttons; older firmware shows the question as an approval (A yes / B no) with the options in the text.

### At a glance on the stick
- **Home:** clock, weather, and a card that rotates through approvals, your last answer, open Claude Code sessions and (optional) facts / Hacker News / Wikipedia.
- **Agent page:** your last question and answer. **Status page:** battery, connections, last-hour battery chart.
- **Charging:** a full-screen desk clock (dims 22:00–06:00).

---

## Install

One command, from a clone of the repo (it's private, so for now you need access to
`ashish-kumar-noon/stick-key`):

```sh
./install.sh
```

or, without cloning first (uses your `gh` login):

```sh
gh api repos/ashish-kumar-noon/stick-key/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
```

Once the setup site is up this becomes `curl -fsSL https://<site>/install.sh | sh`.

What it does: checks for an Apple Silicon Mac, installs [uv](https://docs.astral.sh/uv/)
(a standalone Python manager — no Homebrew or system Python needed) if it's
missing, makes `~/.stick-agent/venv` with Python 3.14, installs stick-agent, and
then runs **`stick-agent setup`**, which asks what else to set up:

| Component | Default | What it does |
|---|---|---|
| `core` | always | the agent, the `stick-agent` command (`~/.local/bin`), its login item, the dashboard |
| `model` | on | downloads the speech model (~2.5 GB, once) |
| `claude` | on | checks Claude Code is installed and logged in; adds our 3 hooks to `~/.claude/settings.json` (backup first, other hooks kept) |
| `github` | off | checks `gh`, logs in if needed, sets your main repo, turns on GitHub alerts |
| `firmware` | off | downloads the latest release's firmware (sha256-checked) and flashes it to a stick plugged in by USB (keeps its settings) |
| `wireless_mic` | off | the stick's mic for Wispr without a cable: checks for BlackHole 2ch (tells you the `brew` command if it's missing — it needs your password, so it never runs it) and turns `wireless_mic` on; then rank “stick-mic-bluetooth” first in Wispr once |

Options (for `install.sh` and `stick-agent setup`): `--yes` (defaults, no
questions), `--with github,firmware`, `--without model`, `--dry-run`,
`--uninstall [all | a,b]` (`--purge` also deletes `~/.stick-agent`).
Anytime later: `stick-agent setup --status`, `stick-agent setup --with github`,
`stick-agent uninstall`. The dashboard's **Settings → Components** shows the same
list with Install / Remove buttons.

Then, in the dashboard (it opens on the first start): **Settings → Find sticks
nearby → Use this one**, and set your repo in **Quick commands**. macOS asks once
for Bluetooth access — allow it. If the login item can't be written, fix it once with
`sudo chown $(whoami) ~/Library/LaunchAgents` and run `stick-agent setup` again.

Requirements: a Mac with Apple Silicon, [Claude Code](https://claude.com/claude-code)
with your own login (the installer helps), the stick paired with the Mac
(System Settings → Bluetooth), and `gh` for GitHub alerts (optional).

### Claude Code hooks (in `~/.claude/settings.json`)
| Hook | Script | What for |
|---|---|---|
| `PermissionRequest` (timeout 45) | `stick_agent/hook.py` | approvals on the stick / dashboard |
| `UserPromptSubmit` + `Stop` (timeout 5) | `stick_agent/claude_hooks.py` | "Claude finished" alerts |

Command for each: `~/.stick-agent/venv/bin/python <path to the script>`. They never
block Claude Code: if the agent isn't running they do nothing.

---

### Wireless Wispr mic
Plugged in by USB the stick is a USB microphone, **stick-mic-usb** (firmware 1.7.1+;
older firmware: *TinyUSB UAC1*). Without the cable, the agent makes a microphone called
**stick-mic-bluetooth** appear on the Mac **only while you hold A**, and Wispr — which always
uses the highest-ranked mic that is present — switches to it for that dictation. The rest
of the time it isn't there and Wispr uses your usual mic (keyboard shortcut included).

1. **Install BlackHole 2ch** (once; asks for your Mac password):
   `brew install --cask blackhole-2ch` — the hidden carrier of the audio; Wispr is never set to it.
2. **Turn it on:** `stick-agent setup --with wireless_mic`, or dashboard → **Settings →
   Wireless Wispr mic** (or **Components → Wireless Wispr mic → Install**). Firmware 1.7.1+.
3. **Rank it once:** hold **A** once (or dashboard → Settings → *Show “stick-mic-bluetooth”
   for 60 s*); in **Wispr Flow → Settings → Microphone**, drag **stick-mic-bluetooth
   (Aggregate)** to the top of the ranking. Put **stick-mic-usb** wherever you like — Wispr
   only sees it while the stick is plugged in. (Coming from the old setup? Pick your usual
   mic again instead of BlackHole 2ch. Renamed mics are new devices to Wispr: rank again.)
4. Hold **A** and talk. The stick's screen says *Wispr hears the stick (BT)* (or *(USB)*
   on the cable); the dashboard shows the last hold's timings.

How a hold goes (away from USB, agent connected): A down → the stick starts streaming and
sends `prepare`; the agent creates the device (~100–170 ms), waits `wispr_switch_ms`
(250 ms) for Wispr to switch, and answers `ready`; only then does the stick press Wispr's
shortcut (it presses anyway after 800 ms). Let go before that = no key at all (a quick tap
does nothing). On release the key-up waits 250 ms for the audio on its way, the agent keeps
the device `wispr_tail_ms` (800 ms) — and as long as Wispr still reads it, up to 4 s more,
so Wispr never stops with *MicDisconnected* — then removes it. A new hold in that time
reuses it at once. Both times are sliders under dashboard → Settings → *Advanced timings*.

The device exists only while the stick is connected by Bluetooth and **not** plugged into a
computer: on USB the stick is its USB mic exactly as before (the key at once, no device, no
Bluetooth audio), and the agent removes the device at once if the stick is plugged in or
disconnects. It is also removed when the agent quits or is stopped (a leftover from a crash
is removed at the next start). The stick has its own switch (menu → *Wireless mic*, on by
default). Remove: `stick-agent setup --uninstall wireless_mic` (turns it off; BlackHole
stays — `brew uninstall --cask blackhole-2ch` removes it).

`wispr_mic_name` (config) renames the device if “bluetooth” in its name ever causes
trouble in Wispr (e.g. `"stick-mic-bt"`); Wispr then needs the ranking again.

## Using it
- **Menu bar** icon: state at a glance, stick volume, sounds on/off, pause, restart, dashboard.
- **Dashboard:** http://localhost:8660 — activity, history, quick commands, sounds, events, settings, logs. Only reachable from this Mac; every change needs the token in `~/.stick-agent/token` (built into the page).
- **Terminal:** `stick-agent status | start | stop | restart | logs | open`, `stick-agent mcp install | status | uninstall`.

Files: everything lives in `~/.stick-agent/` — `config.json`, `commands.json`,
`history.jsonl`, `events.jsonl`, `agent.log`, `sounds/` (Kenney CC0 packs),
`workspace/CLAUDE.md` (the instructions Claude gets for voice requests).

## How it works
| Module | |
|---|---|
| `link.py` | Bluetooth to the chosen stick (bleak): JSON messages, mic audio in, sound clips out |
| `stt.py` | speech to text — Parakeet (MLX), English-only v2 by default (`stt_model`) |
| `commands.py` | quick commands: phrase matching, spoken numbers, app lookup |
| `brain.py` | Claude Code headless: a persistent stream-json session (or `claude -p` per request), follow-ups, interrupt/cancel, timings |
| `speech.py` | replies read aloud: sentence splitting, `say` per sentence, a queue to the stick speaker or the Mac |
| `core.py` | the agent: talk → text → command or Claude → reply; approvals; sounds; speech |
| `wispr_mic.py` | the wireless Wispr mic: stick audio → jitter buffer → BlackHole 2ch (sounddevice) |
| `stick_mic_device.py` | the "stick-mic-bluetooth" device around each A hold: a CoreAudio aggregate on BlackHole (ctypes), the prepare → ready → tail flow |
| `events.py`, `github.py`, `claude_hooks.py` | alerts: GitHub (via `gh`), Claude finished |
| `home.py`, `feed.py`, `weather.py` | the stick's home screen card, facts/news, weather (Open-Meteo) |
| `server.py`, `dashboard.html`, `menubar.py` | dashboard API (localhost + token) and menu bar |
| `mcp_server.py`, `mcp_setup.py` | the stick tools: the MCP endpoint (`/mcp`) and `stick-agent mcp install` (`claude mcp add`) |

## Not built yet
Next meeting on the home screen (calendar), wake word, Mac-control tools for
Claude beyond AppleScript.
