# Architecture

How the stick, the Mac agent and Claude fit together — what runs where, what
talks to what, and where every file lives. For the wire formats see
[../PROTOCOL.md](../PROTOCOL.md); for building and developing see
[BUILDING.md](BUILDING.md) and [DEVELOPMENT.md](DEVELOPMENT.md).

```
 ┌──────────── M5StickS3 (firmware, C++) ────────────┐        ┌──────────────────── your Mac ─────────────────────────────┐
 │                                                   │  BLE   │                                                           │
 │  buttons · mic · speaker · screen · IMU · battery │◀──────▶│  macOS Bluetooth ── keyboard (HID): A = Wispr combo,       │
 │                                                   │  HID   │                     B = Enter, BB = unlock                │
 │  Mac link (Nordic UART service, same connection): │        │                                                           │
 │    RX  ← JSON from the agent (status, screen,     │  BLE   │  stick-agent  (Python, login item, one process)           │
 │          sounds, events, approvals)               │◀──────▶│    link ─ stt ─ commands ─ brain ─ events ─ server/menu  │
 │    TX  → JSON to the agent (talk start/stop,      │  NUS   │      │      │        │         │                         │
 │          approve/deny, acks)                      │        │      │   Parakeet   open/    claude -p ──▶ Claude (cloud)  │
 │    AUDIO    → mic audio (ADPCM, hold B / A)       │        │      │   (local)    osascript   │  uses your Claude Code    │
 │    AUDIO_IN ← sound clips / speech (ADPCM)        │        │      │                          │  login                    │
 │                                                   │  USB   │      └─ hooks ◀── Claude Code (PermissionRequest,         │
 │  USB: serial port + USB microphone (hold A)       │◀──────▶│                    UserPromptSubmit, Stop)                │
 └───────────────────────────────────────────────────┘        │  dashboard: http://localhost:8660  ·  menu-bar icon       │
                                                               └───────────────────────────────────────────────────────────┘
```

## The pieces

| Piece | What it is | Runs where | Source |
|---|---|---|---|
| Stick firmware | Arduino/C++ on the ESP32-S3: a BLE keyboard + the Mac link + screens/sounds | on the stick | `m5-stick-s3/src/` |
| stick-agent | one Python process: Bluetooth link, speech-to-text, quick commands, Claude runner, events, dashboard server, menu bar | your Mac, started at login by launchd | `mac-agent/stick_agent/` |
| Claude Code | Anthropic's CLI, run headless (`claude -p`) once per request — the "brain" | your Mac (the model itself runs in the cloud) | installed separately |
| Hooks | tiny scripts Claude Code runs at certain moments; they forward to the agent | started by Claude Code | `mac-agent/stick_agent/hook.py`, `claude_hooks.py` |
| Dashboard | a web page served by the agent | your browser → `localhost:8660` | `mac-agent/stick_agent/dashboard.html` |

### Inside the stick-agent process
| Thread / task | Does |
|---|---|
| main thread | the menu-bar icon (rumps / AppKit) |
| `agent` thread → asyncio loop | everything else, cooperatively: Bluetooth (bleak), the web server (aiohttp), the talk pipeline, events, GitHub polling, weather, feed |
| `wispr-monitor`, `wispr-router` threads | the smart Wispr mic: watching CoreAudio for Wispr recording; opening / closing the real mic (PortAudio's own threads run the audio callbacks) |
| `stt` thread | Parakeet speech-to-text (MLX keeps its GPU streams per thread, so the model always runs on this one) |
| subprocesses | `claude -p …` per Claude request, `say` for speech, `open`/`osascript` for quick commands, `gh api` for GitHub |

## Where things live

| Path | What |
|---|---|
| `~/Library/LaunchAgents/com.stickkey.agent.plist` | the login item (`stick-agent start` writes it) |
| `~/.stick-agent/venv/` | the agent's Python + libraries (`stick-agent` command is in `bin/`) |
| `~/.stick-agent/config.json` | settings (model, stick, sounds/speech, weather, events, GitHub…) |
| `~/.stick-agent/commands.json` | your quick commands |
| `~/.stick-agent/workspace/CLAUDE.md` | the instructions Claude gets for voice requests (its working folder) |
| `~/.stick-agent/history.jsonl`, `events.jsonl` | what you asked / what it told you |
| `~/.stick-agent/agent.log` | the log (`stick-agent logs`, dashboard → Logs) |
| `~/.stick-agent/token` | secret for the dashboard API (the page has it built in) |
| `~/.stick-agent/sounds/` | Kenney sound packs (CC0) for the library |
| `~/.stick-agent/github_state.json`, `claude_sessions.json` | what the watchers last saw |
| `~/.cache/huggingface/` | the speech models (~2.5 GB each) |
| `~/.claude/settings.json` | Claude Code settings — our three hooks are registered here |
| on the stick (flash, NVS) | combo, name, Mac slots, sounds, volume, brightness, motion options, Mac password (write-only) |

## The flows

### 1. Hold B and ask
1. **Press B** — the stick starts recording right away (pre-roll), in case this becomes a hold.
2. **0.6 s: it's a hold** — the stick sends `talk start` (TX), plays the "speak now" blip (the mic pauses ~0.2 s for it), shows *Listening*, and streams 20 ms ADPCM frames on AUDIO (incl. the pre-roll).
3. **Release** — `talk stop`. The stick shows *Thinking…* and waits (gives up after 20 s with no word).
4. **Agent: speech → text** — frames are decoded to 16 kHz audio; Parakeet transcribes on the `stt` thread (~0.1 s). Quick press with no words → `cancelled` (screen closes quietly).
5. **Quick command?** — `commands.match()` normalises the text ("PR twelve thirty four" → `pr 1234`) and tries each phrase. A hit runs instantly (`open -a "Brave Browser" https://…/pull/1234`) → `done` to the stick.
6. **Otherwise Claude** — `brain.ask()` runs `claude -p "<text>" --output-format stream-json --model sonnet --allowedTools WebSearch,WebFetch,Read,Glob,Grep [--resume <session>]` in `~/.stick-agent/workspace`. Each tool call it streams becomes a *Working: …* line on the stick. The final result (kept short by CLAUDE.md) → `done` (or `error`).
7. **Reply** — shown on the stick (+ chime); short ones optionally spoken (`say` → AIFF → ADPCM → AUDIO_IN → stick speaker).
8. **Escape** — a click of B (or a shake) on an agent screen sends `cancel`: the agent kills the Claude process and stays quiet.

Typical timings: quick command ≈ 1 s after release; Claude ≈ 3–15 s (process start ~2 s + the model + tools).

### 2. Approvals (any Claude Code session, including the agent's own)
Claude Code wants to run something not pre-allowed → runs the **PermissionRequest hook** (`hook.py`) → it POSTs to the agent (`/api/permission`, long-poll, 30 s) → the agent sends the prompt to the stick (orange screen) and the dashboard → **A**/**B** or a dashboard click → the hook prints allow/deny → Claude Code continues. No answer, agent not running, or *Stick approvals* off → the hook prints nothing and Claude Code asks in the terminal as usual.

### 3. Alerts
- **GitHub:** every 2 min the agent runs one read-only `gh api graphql` query (your login), compares with `github_state.json`, and turns changes into events (merged, approved, CI failed/fixed, review requested in the primary repo…).
- **Claude finished:** `UserPromptSubmit` records the start time, `Stop` reports the end; ≥ 2 min → an event.
- **Events** have a level per kind (dashboard → Events): celebrate / nice / headsup / off → the stick shows the full-screen screen + sound and acks.

### 4. Wireless Wispr mic (hold A away from USB)
1. **Press A** — the key combo goes down at once (Wispr starts recording). If the agent's hello said `"wmic":true` and no host has the USB mic open, the stick also sends `wispr start` and streams 20 ms ADPCM frames on AUDIO.
2. **Agent** (`wispr_mic.py`) — frames are decoded, put in order by seq (lost → silence), resampled to BlackHole's rate (soxr) and played from an ~80 ms jitter buffer by a PortAudio stream (sounddevice) into **BlackHole 2ch**, which Wispr has as its microphone. Nothing is transcribed by the agent.
3. **Release A** — the stick keeps streaming 250 ms, then the key goes up and `wispr stop` follows; the agent plays out what's left.

**Smart Wispr mic** — Wispr stays on BlackHole; the agent decides what it carries (`mic_router.py`):

```
 CoreAudio per-process objects ──▶ audio_procs.RecordingMonitor ──"Wispr records"──▶ SmartMic.reconcile()
 (macOS 14.2+: who runs input        (poll 20 Hz / listeners,                          │
  on BlackHole, not our PID)          our PID excluded)                                │ stick streaming?
                                                                                       ├─ yes ─▶ the stick's jitter buffer ─┐
 stick "wispr start/stop" ──▶ WisprMic.start/stop ──────────────────────────────────────┤                                    ├─▶ output stream ─▶ BlackHole 2ch ─▶ Wispr
                                                                                       └─ no ──▶ FallbackMic: real mic        │
                                                                                                 (InputStream → soxr → 30 ms) ┘
```

- **Keyboard dictation:** Wispr starts recording from BlackHole → the monitor sees it (≤ 50 ms) → the router resolves the mic (`wispr_fallback_mic`: the Mac's default input via CoreAudio, or a name; never BlackHole), opens a sounddevice InputStream on it, resamples to BlackHole's rate and the shared output stream plays it. Wispr stops → the input stream closes at once.
- **The stick wins:** the output callback plays the stick whenever a stick stream is active (and its tail), so an A press switches instantly, also mid-dictation; the real mic stays open but muted while Wispr is still recording. After a stick stream ends the router waits 0.6 s (Wispr stops just after the key-up) before handing back to the real mic. Right after Wispr starts it waits 0.15 s for a stick stream if the stick could send one, so an A dictation never opens the real mic.
- **Permission:** the real mic needs macOS's Microphone permission for the agent's (Python) process; without it CoreAudio delivers zeros — after 1 s of only zeros (or a PortAudio error) the dashboard / menu say how to allow it.
- **New devices** (AirPods connecting) need a PortAudio restart to be openable: done while idle when CoreAudio's device list changes, or at dictation start if the wanted mic is missing.

### 5. The home screen
Every 8 s the agent's *hello* message carries the time (stick clock), the weather (Open-Meteo, every 30 min) and the **home card** (`home.py`: approval > last answer > Claude Code sessions > facts / news / Wikipedia). The stick draws it — and the eyes peek now and then on its own.

## Why Claude Code as the brain — and the alternatives

Today the agent shells out to **Claude Code** (`claude -p`) for every request.

**What that gives us for free:** a proven agent loop with tools (shell, web search/fetch, files), permissions + hooks (→ approvals on the stick), MCP for extra tools, sessions (`--resume` for follow-ups), project instructions (`CLAUDE.md`), and it runs on **your own Claude Code login** — no API key, no extra billing, and each teammate uses theirs.

**What it costs:**
- **Latency:** a new process per request (~2 s before the model even starts).
- **Indirect control:** permissions go through the hook script and an HTTP hop; custom tools would need an MCP server; we parse its output stream.
- **Claude only:** no local or other models.

### Options
| | How | Gains | Loses |
|---|---|---|---|
| **A. Keep `claude -p`** (today) | spawn per request | simplest; nothing new | the ~2 s start, indirect control |
| **B. Claude Agent SDK (Python)** — recommended next | `ClaudeSDKClient` kept open inside the agent | same engine and login as Claude Code; **warm session** (no per-request start); **custom tools as Python functions** (open PR, set a stick screen, read the calendar, play a sound) via an in-process MCP server; **`can_use_tool` callback → approvals straight to the stick** (no hook detour); interrupt/cancel built in | a dependency on the SDK's versioning; still Claude-only |
| **C. A minimal harness like [Pi](https://pi.dev/)** (or our own loop) | the agent runs the tool loop itself against a model API | any model — 20+ providers incl. **local (Ollama)**; tiny, fully ours; lowest latency | needs an **API key** (pay per token, not your subscription); we'd rebuild tools, permissions, safety and sessions that Claude Code already does well |

**Recommendation:** move from A to **B (Agent SDK)** — the biggest wins (latency, custom stick-aware tools, direct approvals) with the same login. Keep `brain.py` behind a small interface (`ask(text, on_progress) -> reply`, `cancel()`) so a **C-style brain** (Pi or Ollama for private/offline requests) can be added later as the "local model" option, without touching the rest. A dedicated harness isn't worth it until we want non-Claude models.

Note when sharing: each teammate runs the agent with **their own** Claude Code login, exactly as today. If this ever becomes a product used by people without Claude Code, it would switch to an API key (Anthropic's terms).

## Security model
- The dashboard listens on **127.0.0.1 only**; requests must have `Host: localhost`/`127.0.0.1` and changes need the token — a website you visit can't drive the agent.
- The stick's Mac link requires an **encrypted (paired)** Bluetooth connection; the agent only connects to the stick you picked.
- Claude runs read-only tools freely; anything else needs your approval.
- The Mac password on the stick is write-only (never read back or logged); typing it is a deliberate double-press.

## Sources
- Pi: [pi.dev overview](https://agentic-ai.readthedocs.io/en/latest/AgentHarness/pi-dev/), [Pi on Terminal Trove](https://terminaltrove.com/ai-coding-agents/pi/)
- Claude Agent SDK (Python): [reference](https://code.claude.com/docs/en/agent-sdk/python), [GitHub](https://github.com/anthropics/claude-agent-sdk-python), [custom tools](https://docs.anthropic.com/en/docs/claude-code/sdk/custom-tools)
