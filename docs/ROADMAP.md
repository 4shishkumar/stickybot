# Roadmap: setup website, web edition and companion app

The goal: anyone can go from a new stick to a working setup from **one website**,
installing nothing unless they want the full feature set.

## Three levels, one website

| Level | You install | You get |
|---|---|---|
| **1. Web installer** | nothing | the stick flashed and set up; **button A + Wispr** (keys over Bluetooth, the stick's mic over USB), Enter, unlock, sounds, menu, screens |
| **2. Web edition** | nothing — a browser tab (Chrome / Edge / Brave) + **your own API key** (Anthropic or OpenRouter) | hold B → voice agent: answers + web search, quick commands that open URLs / app links, GitHub alerts, clock + weather on the stick — while the tab is open |
| **3. Companion app** | one command (`curl … \| sh`) | everything: your **Claude Code subscription** (no API key), commands / files / AppleScript / any app, runs in the background from login, approvals + "Claude finished" for terminal Claude Code, wireless Wispr mic (later) |

The site uses **one interface** for levels 2 and 3: when it finds the companion app
running on the Mac it switches to full mode; otherwise it runs the web edition in
the tab. Wherever the web edition needs a key, it also offers: *"Don't want to use an
API key? Install the companion app"* — it uses your Claude Code login instead.

### What each level can't do
- **Level 2:** nothing on the Mac beyond opening URLs/app links (browsers are walled off); stops when the tab closes; needs Chrome-family browsers (Web Bluetooth/Serial); uses a paid API key; approvals for terminal Claude Code sessions aren't available.
- **Level 1:** the stick has no clock/weather (they come from the Mac side) and no voice agent.

## Model providers ("brains")

| Brain | Level | Key | Notes |
|---|---|---|---|
| **Claude Code** (your login) | 3 | none — your subscription | today's brain; most capable on the Mac (tools, permissions, MCP) |
| **Anthropic API** | 2 and 3 | your Anthropic key | Claude models; built-in web search tool |
| **OpenRouter** | 2 and 3 | your OpenRouter key | one key for Claude, GPT, Gemini, Llama, Mistral…; web search via OpenRouter's web plugin; pick the model in settings |
| **Local (Ollama)** | 3 | none | private / offline; later |

- **Keys are each person's own**, entered on the site and stored **only in their browser** (or, with the companion app, in the Mac's Keychain). They're sent only to the provider; never to us or any server of ours.
- The brain is a setting; quick commands ("PR 1234") never need a model at all.
- The same small tool set is given to every brain: `open_url`, `open_app` (app links on level 2; any app on level 3), `run_quick_command`, `show_on_stick`, `play_sound`, `celebrate`, web search; on level 3 also shell/files/AppleScript (with approval on the stick).

## Phases

### Phase 1 — Setup website + level 1 (no install)
- Static site (GitHub Pages or Cloudflare Pages once the remote exists; runs locally until then).
- **Flash:** ESP Web Tools install button with a manifest per release. *New stick:* clean install. *Update:* writes only bootloader / partitions / boot_app0 / app, so settings and pairing survive (never a merged image over NVS).
- **Set up over USB (Web Serial):** name, combo, Enter-after-hold, Mac password (write-only), sounds/volume, motion, slots — reusing the stick's existing serial protocol (PROTOCOL.md).
- **Pairing guide:** System Settings → Bluetooth → stick-key → accept with A; then "Hold A in Wispr" test and picking the stick's mic in Wispr.
- Firmware binaries published as GitHub Release assets (from `make -C m5-stick-s3 release`).

### Phase 2 — Companion app installer (level 3)
- `install.sh` using **uv** (standalone Python manager — no Homebrew/system Python): installs Python + stick-agent into `~/.stick-agent`, the login item, the Claude Code hooks (asks first), opens the local dashboard.
- Checks / guides Claude Code installation and login (Anthropic's official installer).
- The site detects the running app (`localhost:8660/api/health` with CORS + Private Network Access headers limited to the site's origin) and ticks the steps; then hands over to the setup wizard.
- `stick-agent uninstall` to remove everything cleanly.

### Phase 3 — Web edition (level 2)
- Web Bluetooth to the stick's Mac-link service (the keyboard part stays with macOS).
- Speech to text in the browser: Whisper (transformers.js, WebGPU, in a Web Worker), one-time model download, cached.
- A small agent loop in the browser (Pi-style: a model + a few tools) with **Anthropic or OpenRouter** — keys stored locally.
- Quick commands, the home card (clock, weather, facts), sounds/speech (browser speech synthesis → Mac speakers; stick speaker later), GitHub alerts (token stored locally).
- The same UI components as the local dashboard; "full mode" when the companion app is found.

### Phase 4 — Brain upgrade in the companion app
- A persistent Claude Code session (`claude -p --input-format stream-json --output-format stream-json`) — no ~2 s start-up per request.
- A "stick" MCP server with the shared tools; `--permission-prompt-tool` for approvals straight to the stick.
- A brain interface: Claude Code / Anthropic API / OpenRouter / Ollama selectable in settings.

### Later
Signed `.app` (if the command-line install is a hurdle), wake word, calendar on the home screen, wireless Wispr mic (BlackHole), local model.

## Risks and notes
- **Web Bluetooth:** Chrome-family only; the user may need to pick the stick once per session; macOS Bluetooth quirks (re-pair as a fix).
- **Browser speech-to-text** is slower than the companion app's (Parakeet on MLX); the model download is ~100–250 MB.
- **API costs** are on each person's key; show a rough per-request estimate in the UI.
- **Terms:** Claude Code (level 3) runs with each person's own login, as Claude Code intends. The Agent SDK and web edition use API keys, as Anthropic requires for third-party tools.
- **Security:** keys never leave the browser/Keychain except to the provider; the local app keeps its localhost + token protection and only trusts the site's own origin for the "is it installed" check.

## Open questions
- Where the git remote / site will live (personal GitHub, a company org, or Cloudflare).
- Default OpenRouter model for the web edition (e.g. a Claude model for quality, or a cheaper one).
