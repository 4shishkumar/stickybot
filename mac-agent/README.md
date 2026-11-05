# StickyBot — a voice assistant for your Mac, on an M5StickS3

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

### Pick a brain: Claude Code or OpenRouter
Settings → **Brain** picks who answers (everything above that isn't a quick command):
- **Claude Code** (default) — your Claude subscription, no key. It can search the web and run commands on your Mac (with your approval).
- **OpenRouter** — your own [OpenRouter](https://openrouter.ai) API key, **pay per use**; pick any model (the list shows
  each model's price per million tokens; a fast, cheap one that can use tools is suggested). It answers from the model's
  own knowledge (no web search or shell) and can use the stick tools — opening an app or a URL, or running a quick
  command, asks on the stick first (A / B). The dashboard shows each request's time and cost; **Test** sends one short request.
  - **Privacy:** what you say, and the conversation so far, goes to OpenRouter and the model's provider.
  - **The key** is kept in your Mac's login **Keychain** (service `com.stickkey.agent`, account `openrouter`), never in
    `config.json`; the dashboard can save or remove it but never reads it back (it shows "saved (…abcd)").
    For development, the environment variable `STICK_AGENT_OPENROUTER_KEY` overrides it.
  - Settings: `openrouter_model`, `openrouter_max_tokens` (400). Follow-ups and "new chat" work as with Claude.

Switching applies from the next request, no restart needed.

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
- **Terminal sessions:** run `stickybot mcp install` once (or dashboard → Settings → *Stick tools for Claude
  Code* → Install). It shows and then runs Claude Code's own command —
  `claude mcp add --transport http --scope user stick http://127.0.0.1:8660/mcp --header "Authorization: Bearer <token>" --header "X-Stick-Cwd: ${PWD}"` —
  so every new Claude Code session has `mcp__stick__*`. Then ask e.g. *"run the tests and notify me on the stick when
  done"*. `stickybot mcp status` / `stickybot mcp uninstall` (`--yes` skips the question). Nothing of ours edits
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

One command (no GitHub login needed):

```sh
curl -fsSL https://raw.githubusercontent.com/ashish-kumar-noon/stickybot/main/install.sh | sh
```

or, from a clone of the repo, `./install.sh`.

What it does: checks for an Apple Silicon Mac, installs [uv](https://docs.astral.sh/uv/)
(a standalone Python manager — no Homebrew or system Python needed) if it's
missing, makes `~/.stick-agent/venv` with Python 3.14, installs StickyBot, builds
**StickyBot.app** (below), starts it (login item), and opens the **setup page** — in the
StickyBot window, or your browser if the app couldn't be built:
**http://localhost:8660/#install**.

The setup page does the rest, one step per screen, each with a button and live progress:

| Step | | What happens when you click |
|---|---|---|
| Welcome | | checks Apple Silicon, macOS 13.5+, free disk space (4 GB for the model) |
| Speech model | recommended | downloads it (~2.5 GB) with size, speed and time left |
| Claude Code | optional | runs Anthropic's installer if it's missing; then says to run `claude` once in Terminal to log in, and notices by itself when you have |
| Approvals on the stick | optional | shows the 3 hooks first, then adds them to `~/.claude/settings.json` (backup first) |
| Stick tools for Claude Code | optional | runs `claude mcp add …` (see below) |
| GitHub alerts | optional | shows `brew install gh` if `gh` is missing; logs in with a one-time code shown on the page (enter it at github.com/login/device); asks for your main repo |
| Wireless Wispr mic | optional | macOS 14.2+: turns the mic on and checks the Apple audio tap (no driver; macOS asks once for *System Audio Recording*). Older macOS, or *Use BlackHole instead*: installs BlackHole 2ch — a pinned, tested version (0.7.1), sha256 and signature checked, with the Mac's own password dialog |
| Your stick | recommended | the setup wizard: pair, pick, name, firmware (USB or Bluetooth), try it |

Skip any optional step; skipped steps can be done later from the last screen or
**Settings → Components → Open the setup page**. Reopen the page with
**`stickybot install-page`** or the menu bar's **Finish setup…** (shown while the model or
the stick isn't set up).

**In the terminal instead** (scripts, AI agents, SSH): any of `--yes`, `--with`,
`--without`, `--dry-run`, `--uninstall` or `--no-browser` — and an SSH session, or nobody
logged in at the screen — runs **`stickybot setup`** as before, which installs these
components:

| Component | Default | What it does |
|---|---|---|
| `core` | always | the agent, StickyBot.app (`~/Applications`), the `stickybot` command (`~/.local/bin`; `stick-agent` too), its login item, the dashboard |
| `model` | on | downloads the speech model (~2.5 GB, once) |
| `claude` | on | checks Claude Code is installed and logged in; adds our 3 hooks to `~/.claude/settings.json` (backup first, other hooks kept) |
| `github` | off | checks `gh`, logs in if needed, sets your main repo, turns on GitHub alerts |
| `firmware` | off | downloads the latest release's firmware (sha256-checked) and flashes it to a stick plugged in by USB (keeps its settings) |
| `wireless_mic` | off | the stick's mic for Wispr without a cable: turns `wireless_mic` on (macOS 14.2+: carried by the Apple audio tap, no driver; older macOS: checks for BlackHole 2ch and tells you the `brew` command if it's missing — it needs your password, so it never runs it); then rank “<stick>-bt-mic” first in Wispr once |

Options (for `install.sh` and `stickybot setup`): `--yes` (defaults, no
questions), `--with github,firmware`, `--without model`, `--dry-run`,
`--uninstall [all | a,b]` (`--purge` also deletes `~/.stick-agent`).
Anytime later: `stickybot setup --status`, `stickybot setup --with github`,
`stickybot uninstall`. The dashboard's **Settings → Components** shows the same
list with Install / Remove buttons.

macOS asks once for Bluetooth access for StickyBot — allow it. If the login item can't be written, fix it once with
`sudo chown $(whoami) ~/Library/LaunchAgents` and run `stickybot setup` again.

Requirements: a Mac with Apple Silicon, [Claude Code](https://claude.com/claude-code)
with your own login (the installer helps), the stick paired with the Mac
(System Settings → Bluetooth), and `gh` for GitHub alerts (optional).

### StickyBot.app
The agent runs as **StickyBot.app** in `~/Applications`, so macOS shows *StickyBot* (with
its icon) in permission prompts, Privacy & Security, Activity Monitor and Force Quit —
not *Python*. The installer builds it on this Mac: the bundle's executable is a copy of
the venv's own Python interpreter, signed ad hoc (no Apple Developer ID needed; built
locally, so Gatekeeper doesn't ask), and the login item starts it directly. Opening the
app from Finder or Spotlight shows the StickyBot window (and starts the agent if it isn't
running).

- **Rebuilt only when Python itself changes** (a new interpreter, e.g. after
  `brew upgrade python@3.14`): code updates don't touch it, so macOS keeps its permissions.
  After a rebuild macOS asks once more for Bluetooth and System Audio Recording.
- `stickybot app` says whether it's built and up to date; `stickybot app rebuild` builds it
  again and restarts the login item.
- If it can't be built, the agent keeps running as plain Python (as before) and the
  installer says why. `STICK_AGENT_NO_APP=1` skips it on purpose.
- `stickybot uninstall` removes it. How it works: [ARCHITECTURE.md](../docs/ARCHITECTURE.md#stickybotapp-why-the-agent-has-its-own-app-bundle).

**Coming from stick-agent** (before the rename): re-run the installer (or update from the
dashboard — the new version builds the app and switches the login item when it starts).
macOS then asks again: **Bluetooth** (*“StickyBot” would like to use Bluetooth* — Allow) and,
with the wireless mic on the Apple audio tap, **System Audio Recording** for StickyBot.
The old *Python* entries in Privacy & Security can be removed. The mics have new names
(below): rank them again in Wispr once. `stick-agent` still works as a command.

### Claude Code hooks (in `~/.claude/settings.json`)
| Hook | Script | What for |
|---|---|---|
| `PermissionRequest` (timeout 45) | `stick_agent/hook.py` | approvals on the stick / dashboard |
| `UserPromptSubmit` + `Stop` (timeout 5) | `stick_agent/claude_hooks.py` | "Claude finished" alerts |

Command for each: `~/.stick-agent/venv/bin/python <path to the script>`. They never
block Claude Code: if the agent isn't running they do nothing.

---

### Wireless Wispr mic
Plugged in by USB the stick is a USB microphone, **<stick>-usb-mic** — named after the stick, e.g. *stickybot-usb-mic* (firmware 1.9.0+; 1.8.1–1.8.x: *<stick>-mic-usb*; 1.7.1–1.8.0: *stick-mic-usb*;
older firmware: *TinyUSB UAC1*). Without the cable, the agent makes a microphone called
**<stick>-bt-mic** (e.g. *stickybot-bt-mic*; before the rename *<stick>-mic-bluetooth*) appear on the Mac **only while you hold A**, and Wispr — which always
uses the highest-ranked mic that is present — switches to it for that dictation. The rest
of the time it isn't there and Wispr uses your usual mic (keyboard shortcut included).

1. **Turn it on:** the setup page (`stickybot install-page` → *Wireless Wispr mic* →
   **Turn on**), `stickybot setup --with wireless_mic`, or dashboard → **Settings →
   Wireless Wispr mic** (or **Components → Wireless Wispr mic → Install**). Firmware 1.7.1+.
2. **macOS 14.2 or newer: no driver.** The audio is carried by an *Apple audio tap* of the
   agent's own sound. macOS asks once to allow **System Audio Recording** for **StickyBot**
   (it only taps its own sound, muted; never other apps). Press
   *Allow*; if you missed it: System Settings → Privacy & Security → Screen & System Audio
   Recording → *System Audio Recording Only* → turn on StickyBot, then *Check again*
   (Settings → Wireless Wispr mic, or the setup page). The agent checks the tap with an
   inaudible test before using it, and never plays the stick through the speakers.
   **Older macOS, or by choice** (Settings → *Carried by* → BlackHole 2ch, or *Use BlackHole
   instead* on the setup page): **install BlackHole 2ch** once (asks for your Mac password) —
   one click on the setup page, or `brew install --cask blackhole-2ch`; the hidden carrier of
   the audio; Wispr is never set to it. *Automatic* (the default) uses the tap once it's
   allowed, else BlackHole if it's installed.
3. **Rank it once:** hold **A** once (or dashboard → Settings → *Show “<stick>-bt-mic”
   for 60 s*); in **Wispr Flow → Settings → Microphone**, drag **<stick>-bt-mic
   (Aggregate)** to the top of the ranking. Put **<stick>-usb-mic** wherever you like — Wispr
   only sees it while the stick is plugged in. (Coming from the old setup? Pick your usual
   mic again instead of BlackHole 2ch. Renamed mics — including the StickyBot rename — are new devices to Wispr: rank again.)
4. Hold **A** and talk. The stick's screen says *Wispr hears the stick (BT)* (or *(USB)*
   on the cable); the dashboard shows the last hold's timings.

How a hold goes (away from USB, agent connected): A down → the stick starts streaming and
sends `prepare`; the agent creates the device (~100–170 ms; with the tap: tap + device), waits `wispr_switch_ms`
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
default). Remove: `stickybot setup --uninstall wireless_mic` (turns it off; BlackHole, if
you installed it, stays — `brew uninstall --cask blackhole-2ch` removes it). Switching the
carrier keeps the device's name and UID, so Wispr's ranking stays.

`wispr_mic_name` (config) renames the device if “bluetooth” in its name ever causes
trouble in Wispr (e.g. `"my-stick-mic"`); Wispr then needs the ranking again. Empty means automatic, and so do
the old automatic names (`stick-mic-bluetooth`, `<stick>-mic-bluetooth`); a name you typed wins.

## Using it
- **Menu bar** icon: state at a glance, stick volume, sounds on/off, pause, restart, **Open StickyBot** (the dashboard
  in its own window; a Dock icon while it's open, closing it doesn't quit) and **Open in browser**.
- **Dashboard:** http://localhost:8660 — activity, history, quick commands, sounds, events, settings, logs. Only reachable from this Mac; every change needs the token in `~/.stick-agent/token` (built into the page).
- **Terminal:** `stickybot status | start | stop | restart | logs | open [--browser]`, `stickybot app [rebuild]`,
  `stickybot mcp install | status | uninstall` (`stick-agent …` works the same).
- **Firmware:** dashboard → Stick → Firmware. **Update over Bluetooth** (stick 1.8.0+, connected; no cable, ~1–2 min,
  the stick shows the progress and B cancels) or **Update via USB** (any firmware; also the recovery path).
  Terminal: `stickybot flash --ble [--force]` (through the running agent) or `stickybot flash [--port P] [--backup]`.
  Both use the bundle in `stick_agent/firmware/` and keep the stick's settings; see
  [PROTOCOL.md](../PROTOCOL.md#firmware-update-over-bluetooth-sticks3-180) and [BUILDING.md](../docs/BUILDING.md#over-bluetooth-no-cable-stick-firmware-180).

Files: everything lives in `~/.stick-agent/` — `config.json`, `commands.json`,
`history.jsonl`, `events.jsonl`, `agent.log`, `sounds/` (Kenney CC0 packs),
`workspace/CLAUDE.md` (the instructions Claude gets for voice requests).

## How it works
| Module | |
|---|---|
| `link.py` | Bluetooth to the chosen stick (bleak): JSON messages, mic audio in, sound clips and firmware out |
| `flasher.py`, `ota.py`, `firmware_api.py` | firmware updates: USB (esptool) and over Bluetooth (the app image, acks / resume, rollback check) |
| `stt.py` | speech to text — Parakeet (MLX), English-only v2 by default (`stt_model`) |
| `commands.py` | quick commands: phrase matching, spoken numbers, app lookup |
| `brain.py` | Claude Code headless: a persistent stream-json session (or `claude -p` per request), follow-ups, interrupt/cancel, timings |
| `brains.py`, `openai_brain.py`, `brain_api.py` | the brain picked in Settings; the OpenRouter brain (OpenAI-compatible chat, streaming, the stick tools); Settings → Brain API |
| `keychain.py` | API keys in the macOS Keychain (Security framework via ctypes) |
| `speech.py` | replies read aloud: sentence splitting, `say` per sentence, a queue to the stick speaker or the Mac |
| `core.py` | the agent: talk → text → command or Claude → reply; approvals; sounds; speech |
| `wispr_mic.py` | the wireless Wispr mic: stick audio → jitter buffer → the default output (Apple audio tap) or BlackHole 2ch (sounddevice) |
| `stick_mic_device.py` | the "<stick>-bt-mic" device around each A hold: a CoreAudio aggregate with the agent's process tap or on BlackHole (ctypes), the tap check, the carrier choice, the prepare → ready → tail flow |
| `events.py`, `github.py`, `claude_hooks.py` | alerts: GitHub (via `gh`), Claude finished |
| `home.py`, `feed.py`, `weather.py` | the stick's home screen card, facts/news, weather (Open-Meteo) |
| `server.py`, `dashboard.html`, `menubar.py` | dashboard API (localhost + token) and menu bar |
| `app.py`, `window.py` | StickyBot.app (build, sign, rebuild rules, Finder launch) and the dashboard window (WKWebView) |
| `mcp_server.py`, `mcp_setup.py` | the stick tools: the MCP endpoint (`/mcp`) and `stickybot mcp install` (`claude mcp add`) |

## Not built yet
Next meeting on the home screen (calendar), wake word, Mac-control tools for
Claude beyond AppleScript.
