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
| Claude Code | Anthropic's CLI, run headless — one long-lived `claude -p --input-format stream-json` session (or `claude -p` per request) — the "brain" | your Mac (the model itself runs in the cloud) | installed separately |
| Hooks | tiny scripts Claude Code runs at certain moments; they forward to the agent | started by Claude Code | `mac-agent/stick_agent/hook.py`, `claude_hooks.py` |
| Dashboard | a web page served by the agent | your browser → `localhost:8660` | `mac-agent/stick_agent/dashboard.html` |

### Inside the stick-agent process
| Thread / task | Does |
|---|---|
| main thread | the menu-bar icon (rumps / AppKit) |
| `agent` thread → asyncio loop | everything else, cooperatively: Bluetooth (bleak), the web server (aiohttp), the talk pipeline, events, GitHub polling, weather, feed |
| `stt` thread | Parakeet speech-to-text (MLX keeps its GPU streams per thread, so the model always runs on this one) |
| `stick-mic` thread | creates / removes the stick-mic-bluetooth CoreAudio device and checks who reads it (one at a time, off the event loop) |
| subprocesses | the Claude Code session (`claude -p --input-format stream-json …`, kept running; or one per request), `say` per spoken sentence (+ `afplay` for the Mac's speakers), `open`/`osascript` for quick commands, `gh api` for GitHub |

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
6. **Otherwise Claude** — `brain.ask()` writes the text as a stream-json user message to the long-lived session, `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --model sonnet --allowedTools WebSearch,WebFetch,Read,Glob,Grep [--resume <session>]` in `~/.stick-agent/workspace` (started when the stick connects, or on the first request; `brain_mode: "oneshot"` = the old `claude -p "<text>"` per request). Each tool call becomes a *Working: …* line on the stick; "new chat" / "start over" starts a new conversation.
7. **Reply, as it's written** — text deltas update the stick's screen (*Working* + the answer so far, ≤ 1 update per 300 ms); with speech on, each finished sentence goes to `say` at once and the clips play back to back (stick speaker: `say` → AIFF → ADPCM → AUDIO_IN; or the Mac's speakers). Text before a tool call ("Let me search…") isn't spoken. The final result (kept short by CLAUDE.md) → `done` (+ chime) or `error`.
8. **Escape** — a click of B (or a shake) on an agent screen sends `cancel`: the agent sends Claude Code an *interrupt* (the session stays warm; killed and resumed if it doesn't stop within 3 s), stops the speech, and stays quiet.

The session: stopped after `brain_idle_min` (10) unused — the follow-up window, so the next request is a new chat; restarted with `--resume` after a crash or a settings change (model, tools). One request at a time (a second one gets "Still working on the last request."). Timings per request are logged and shown in Settings.

Typical timings: quick command ≈ 1 s after release; Claude ≈ 2–15 s (the model + tools; the first words show while it's still writing). A new `claude -p` process adds ~0.5 s or more before the model starts — the session skips that.

### 2. Approvals (any Claude Code session, including the agent's own)
Claude Code wants to run something not pre-allowed → runs the **PermissionRequest hook** (`hook.py`) → it POSTs to the agent (`/api/permission`, long-poll, 30 s) → the agent sends the prompt to the stick (orange screen) and the dashboard → **A**/**B** or a dashboard click → the hook prints allow/deny → Claude Code continues. No answer, agent not running, or *Stick approvals* off → the hook prints nothing and Claude Code asks in the terminal as usual.

### 3. Alerts
- **GitHub:** every 2 min the agent runs one read-only `gh api graphql` query (your login), compares with `github_state.json`, and turns changes into events (merged, approved, CI failed/fixed, review requested in the primary repo…).
- **Claude finished:** `UserPromptSubmit` records the start time, `Stop` reports the end; ≥ 2 min → an event.
- **Events** have a level per kind (dashboard → Events): celebrate / nice / headsup / off → the stick shows the full-screen screen + sound and acks.

### 4. Wireless Wispr mic (hold A away from USB)
Wispr Flow keeps a ranked list of mics and uses the highest-ranked one present, re-checking whenever the device list changes. The agent makes a mic, **stick-mic-bluetooth**, exist only around an A hold; ranked first once, it is what Wispr records from while A is held, and the user's usual mic otherwise.

```
 stick                          agent (stick_mic_device.py, wispr_mic.py)            CoreAudio / Wispr
 A down ── prepare id=n ──────▶ wispr_mic.start (output into BlackHole open)
   │  stream ADPCM ───────────▶ ensure_present(): aggregate "stick-mic-bluetooth" ─▶ device appears (~100-170 ms)
   │                            (public, sub-device BlackHole 2ch, ctypes)          Wispr: devicechange → ranks it first
   │                            wait wispr_switch_ms (250)
   │  ◀────────── ready id=n ── 
 key down (≤ 800 ms after A) ─────────────────────────────────────────────────────▶ Wispr records from it
 A up → 250 ms → key up, stop ▶ wispr_mic.stop; keep wispr_tail_ms (800) + while
                                Wispr still reads it (≤ 4 s) → remove() ─────────▶ gone: Wispr back to the usual mic
```

1. **Press A** (firmware 1.7.1, not plugged into a computer — TinyUSB mounted + VBUS means *USB*: the key at once, the USB mic *stick-mic-usb*, nothing over Bluetooth). If the agent's hello said `"wmic":true,"wready":true`, the stick sends `wispr prepare` with an id and streams 20 ms ADPCM frames on AUDIO right away, but holds the key.
2. **Agent** — `Holds.prepare()` creates the aggregate on its `stick-mic` worker thread, waits the switch time and answers `ready`; the stick presses the key on it (or after 800 ms anyway). Released before that = no key. Meanwhile `wispr_mic.py` decodes the frames, puts them in order by seq (lost → silence), resamples to BlackHole's rate (soxr) and plays them from an ~80 ms jitter buffer by a PortAudio stream (sounddevice) into **BlackHole 2ch**, whose input is the aggregate's. Nothing is transcribed by the agent.
3. **Release A** — the stick keeps streaming 250 ms, then the key goes up and `wispr stop` follows; the agent plays out what's left, keeps the device for the tail (and while CoreAudio shows another process still reading BlackHole, ≤ 4 s), then removes it. A new `prepare` in that time finds it present and gets `ready` at once.
4. **Safety** — the device is removed at once when the stick disconnects or reports `usb mounted`, when the feature is turned off, and on exit / SIGTERM / restart; at start the agent removes any left over (an aggregate outlives the process that made it). A 1.7.0 stick sends `wispr start` instead: the agent streams as before but makes no device.

### 5. The home screen
Every 8 s the agent's *hello* message carries the time (stick clock), the weather (Open-Meteo, every 30 min) and the **home card** (`home.py`: approval > last answer > Claude Code sessions > facts / news / Wikipedia). The stick draws it — and the eyes peek now and then on its own.

## Why Claude Code as the brain — and the alternatives

The agent runs **Claude Code** headless: one long-lived `claude -p --input-format stream-json` session (option A+ below), with `claude -p` per request as the fallback.

**What that gives us for free:** a proven agent loop with tools (shell, web search/fetch, files), permissions + hooks (→ approvals on the stick), MCP for extra tools, sessions (`--resume` for follow-ups), project instructions (`CLAUDE.md`), and it runs on **your own Claude Code login** — no API key, no extra billing, and each teammate uses theirs.

**What it costs:**
- **Latency:** a new process per request (~0.5–2 s before the model even starts) — solved by the persistent session.
- **Indirect control:** permissions go through the hook script and an HTTP hop; custom tools would need an MCP server; we parse its output stream.
- **Claude only:** no local or other models.

### Options
| | How | Gains | Loses |
|---|---|---|---|
| **A. `claude -p`** (the fallback, `brain_mode: "oneshot"`) | spawn per request | simplest | the start-up per request, indirect control |
| **A+. A persistent `claude -p --input-format stream-json`** (now) | one process, a user message per request, the interrupt control request to cancel | no start-up per request, streaming, interrupt — with no new dependency (the same protocol the Agent SDK speaks) | still indirect control (hooks for approvals, no custom tools yet) |
| **B. Claude Agent SDK (Python)** — recommended next | `ClaudeSDKClient` kept open inside the agent | same engine and login as Claude Code; **warm session** (no per-request start); **custom tools as Python functions** (open PR, set a stick screen, read the calendar, play a sound) via an in-process MCP server; **`can_use_tool` callback → approvals straight to the stick** (no hook detour); interrupt/cancel built in | a dependency on the SDK's versioning; still Claude-only |
| **C. A minimal harness like [Pi](https://pi.dev/)** (or our own loop) | the agent runs the tool loop itself against a model API | any model — 20+ providers incl. **local (Ollama)**; tiny, fully ours; lowest latency | needs an **API key** (pay per token, not your subscription); we'd rebuild tools, permissions, safety and sessions that Claude Code already does well |

**Recommendation:** A+ gives the latency win today; move to **B (Agent SDK)** — the biggest wins (latency, custom stick-aware tools, direct approvals) with the same login. Keep `brain.py` behind a small interface (`ask(text, on_progress) -> reply`, `cancel()`) so a **C-style brain** (Pi or Ollama for private/offline requests) can be added later as the "local model" option, without touching the rest. A dedicated harness isn't worth it until we want non-Claude models.

Note when sharing: each teammate runs the agent with **their own** Claude Code login, exactly as today. If this ever becomes a product used by people without Claude Code, it would switch to an API key (Anthropic's terms).

## Security model
- The dashboard listens on **127.0.0.1 only**; requests must have `Host: localhost`/`127.0.0.1` and changes need the token — a website you visit can't drive the agent.
- The stick's Mac link requires an **encrypted (paired)** Bluetooth connection; the agent only connects to the stick you picked.
- Claude runs read-only tools freely; anything else needs your approval.
- The Mac password on the stick is write-only (never read back or logged); typing it is a deliberate double-press.

## Sources
- Pi: [pi.dev overview](https://agentic-ai.readthedocs.io/en/latest/AgentHarness/pi-dev/), [Pi on Terminal Trove](https://terminaltrove.com/ai-coding-agents/pi/)
- Claude Agent SDK (Python): [reference](https://code.claude.com/docs/en/agent-sdk/python), [GitHub](https://github.com/anthropics/claude-agent-sdk-python), [custom tools](https://docs.anthropic.com/en/docs/claude-code/sdk/custom-tools)
