# Macro button protocol

Every board in this folder (XIAO ESP32-C3, M5StickS3, future ones — any
language or OS) behaves the same from the outside, so the USB settings page,
the Wi-Fi settings page and a future Mac helper work with all of them.

Version: **1** (M1.5)

---

## 1. Bluetooth

- Standard BLE HID keyboard (+ battery service where the board has a battery).
- **3 Mac slots.** Each slot has its own Bluetooth identity (random static
  address derived from the chip ID + slot number), so each Mac only ever sees
  "its" slot and Macs never fight over the board.
- Advertised name: user-configurable (1–20 chars), the same for every slot.
  Defaults: `<board>-key` — `xiao-key`, `stick-key`. Renaming restarts the board.
- Only the **active slot** advertises / connects. Switching slot restarts the board.
- Pairing:
  - Boards with a screen: **confirmed code** — the Mac and the board show the
    same 6-digit code; you click Connect on the Mac and accept on the board
    (main button = accept, second button = reject, 30 s timeout).
  - Boards without a screen: "Just Works" (no code).
- Slot actions: **switch** (restart into slot N), **disconnect** (stay paired,
  stop advertising until reconnect or restart), **reconnect**, **forget** (one
  slot or all).

## 2. Combo

One key combo, sent by the main button: button down = combo down,
button up = combo up (single press → once, double → twice, hold → held).

Text form, case-insensitive, parts joined with `+`:

| Part | Accepted |
|---|---|
| Modifiers | `cmd`/`command`/`gui`/`meta`, `opt`/`option`/`alt`, `ctrl`/`control`, `shift` |
| Keys | `a`–`z`, `0`–`9`, `f1`–`f20`, `space`, `return`/`enter`, `esc`/`escape`, `tab`, `backspace`, `delete`/`del`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`/`pgup`, `pagedown`/`pgdn`, `- = [ ] \ ; ' \` , . /` |

At most one non-modifier key. Modifier-only combos are allowed (`cmd+opt`).
Display order: Ctrl, Option, Shift, Cmd, key — e.g. `Option+Cmd`.

Numeric form (used by the pages): `mods` = bit mask (Ctrl 1, Shift 2,
Option 4, Cmd 8), `key` = HID usage code (0 = none).

Stored in the board's flash; survives power-off and re-flashing.

## 3. USB serial

115200 baud, lines ending in `\n`. Opening the port may restart the board
(ESP32 USB behaviour); wait for the `ready` line.

### Human commands

`help`, `show`, `set <combo>`, `name <text>`, `slot <1-3>`, `disconnect`, `reconnect`,
`forget <1-3|all>`, `web on`, `web off`.

### Machine commands (used by the USB settings page)

Format: `@<action> [name=value&name=value…]` — the same actions and
URL-encoded parameters as the HTTP API (section 4). Each produces exactly
**one** response line: `@` followed by JSON. All other lines are log output
and can be ignored.

```
→ @state
← @{"ok":true,"proto":1,"board":"xiao-esp32-c3","fw":"1.5.0", ...state... }
→ @combo text=cmd%2Bopt
← @{"ok":true,"combo":"Option+Cmd"}
```

| Action | Parameters | Does |
|---|---|---|
| `state` | | Full state (below) |
| `keys` | | `{"ok":true,"mods":[["Ctrl",1],…],"keys":[["(none)",0],["A",4],…]}` |
| `combo` | `text=<combo>` or `mods=<n>&key=<n>` | Set the combo |
| `test` | | Tap the combo once |
| `name` | `name=…` | Bluetooth name, 1–20 chars (board restarts) |
| `slot` | `n=1..3` | Switch Mac slot (board restarts) |
| `disconnect` / `reconnect` | | Stop / resume Bluetooth on the active slot |
| `forget` | `n=1..3` or `n=all` | Forget a slot's Mac; restarts if it was the active slot |
| `wifi` | `ssid=…&pass=…` | Save home Wi-Fi (empty ssid = clear) |
| `password` | `password=…` | Device password (hotspot + OTA), 8–63 chars |
| `web-on` / `web-off` | | Wi-Fi settings page on/off |
| `standby` | `min=0..120` | Battery boards: standby after N idle minutes on battery (0 = never) |
| `brightness` | `pct=10..100` | Boards with a screen: screen brightness |

Responses that end in a restart include `"restarting":true`.

Errors: `@{"ok":false,"error":"<message>"}`.

### State object

```json
{
  "ok": true, "proto": 1, "board": "xiao-esp32-c3", "fw": "1.5.0",
  "name": "xiao-key",
  "combo": "Option+Cmd", "mods": 12, "key": 0,
  "connected": true, "bt": "connected",
  "battery": -1, "charging": false,
  "standby": 10, "brightness": 50,
  "slot": 1,
  "slots": [ {"n":1,"paired":true}, {"n":2,"paired":false}, {"n":3,"paired":false} ],
  "web": { "on": false, "mode": "off", "ssid": "", "url": "", "homeSsid": "" }
}
```

- `bt`: `connected` | `advertising` | `off` (disconnected on purpose)
- `battery`: 0–100, or -1 when the board has no battery
- `standby`: only present on boards with a battery (minutes, 0 = never)
- `brightness`: only present on boards with a screen (percent)
- `web.mode`: `off` | `joining` | `home` (joined home Wi-Fi) | `hotspot`

## 4. Wi-Fi settings page + OTA

Off by default. Turned on per board (StickS3: menu; C3: RESET → hold BOOT;
any board: `web on` over USB). Joins saved home Wi-Fi (`http://<host>.local`),
otherwise / on failure starts a hotspot `<Board>-Setup` protected by the
device password. Turns itself off after 10 min without requests.

HTTP API — same actions as the machine commands, JSON responses:

| Method + path | Body (form-encoded) |
|---|---|
| `GET /api/state` | |
| `GET /api/keys` | |
| `POST /api/test` | |
| `POST /api/combo` | `text=…` or `mods=…&key=…` |
| `POST /api/slot` | `n=…` |
| `POST /api/name` | `name=…` |
| `POST /api/disconnect`, `POST /api/reconnect` | |
| `POST /api/forget` | `n=1..3` or `n=all` |
| `POST /api/wifi` | `ssid=…&pass=…` |
| `POST /api/password` | `password=…` |
| `POST /api/web-on`, `POST /api/web-off` | |
| `POST /api/standby` | `min=…` |
| `POST /api/brightness` | `pct=…` |
| `POST /update` | multipart file upload of `firmware.bin` (OTA) |

OTA from PlatformIO while the page is on:
`pio run -t upload --upload-port <host>.local` (password = device password,
set via `upload_flags = --auth=<password>`).

Hosts: `<default name>.local` — `xiao-key.local`, `stick-key.local`.
Hotspots: `xiao-key-setup`, `stick-key-setup`.

## 5. Mac link (boards with a mic + screen: StickS3)

A Nordic UART service (`6e400001-b5a3-f393-e0a9-e50e24dcca9e`) on the same
Bluetooth connection as the keyboard. Writes need the paired (encrypted) link.

| Characteristic | UUID suffix | Direction | Content |
|---|---|---|---|
| RX | `…0002…` | Mac → board | JSON lines |
| TX | `…0003…` | board → Mac | JSON lines (notify, split into ≤180-byte pieces) |
| Audio | `…0004…` | board → Mac | agent audio frames (notify) |
| Audio in | `…0005…` | Mac → board | sound clips: `[0][samples u32][rate u16]` start, `[1][frame]` data, `[2]` end → play (write without response; first/last acknowledged) |

The service UUID is in the advertisement (while connected too), so Mac
programs find the board by it; they should then stay with the one board the
user picked (several boards can be in range).

**Claude Code permission prompts** (tools/claude-hook) use Claude's Hardware
Buddy messages with `"src":"cli"`; the board answers
`{"cmd":"permission","id":…,"decision":"once"|"deny"}` from buttons A / B.

**Mac agent** (tools/mac-agent), all messages tagged `"src":"agent"`:

| Direction | Message |
|---|---|
| Mac → board | `{"src":"agent","hello":1,"v":1}` every ≤10 s — the board shows "agent ready" for 25 s after each |
| board → Mac | `{"src":"agent","evt":"talk","state":"start","rate":16000,"codec":"ima-adpcm"}` when B becomes a hold |
| board → Mac | `{"src":"agent","evt":"talk","state":"stop"}` after the last audio frame |
| Mac → board | `{"src":"agent","state":"thinking"\|"working"\|"done"\|"error","heard":"…","text":"…"}` — shown on screen |

Audio frames: 16 kHz mono IMA-ADPCM, ≤20 ms each, little-endian:
`[seq u16][predictor i16][step index u8][samples, 4 bits each, low nibble first]`.
Each frame carries the decoder state at its start, so a lost frame only loses
its own 20 ms. `seq` restarts at 0 for every talk. The first frames are the
pre-roll (recorded since B went down) and arrive faster than real time.

**Sounds** (`"src":"agent"`):

| Direction | Message |
|---|---|
| Mac → board | `{"src":"agent","sound":{"get":1}}`, `{"sound":{"set":{"answer":"confirmation_001"}}}`, `{"sound":{"play":"answer"}}` (an event or a tune/clip name), `{"sound":{"on":true}}`, `{"sound":{"volume":0-255}}` |
| board → Mac | `{"src":"agent","evt":"sounds","sounds":{"tunes":[…],"map":{event: tune},"on":…,"volume":…}}` after each of those |

Events: sent, answer, approval, approved, denied, error, connected, disconnected, unlock, charging, lowbattery.
Tunes: built-in Kenney clips (CC0, `tools/make_sounds.py`) and synthesized beeps; `off` = silent.
The agent's hello may also carry `"home":{"k","a","b","u"}` (home screen card) and `"wx":{"t","hi","lo","i","c","day","at"}` (weather).

**Events** (full-screen screens on the board; StickS3):

| Direction | Message |
|---|---|
| Mac → board | `{"src":"agent","event":{"id":"e123","level":"celebrate"\|"nice"\|"headsup","title":"PR #5507 merged","sub":"wvrdz/loom · feat: dark mode","icon":"merge"\|"pr"\|"check"\|"x"\|"review"\|"claude"\|"star"\|"bell","sound":""}}` |
| board → Mac | `{"src":"agent","evt":"event_shown","id":"e123"}` when it appears on screen |

- `celebrate`: confetti, a colour pulse, the happy face and the big icon, ~6 s; default sound `jingle_win` (Kenney Music Jingles, CC0).
- `nice`: a teal-to-green toast that slides in, ~4 s; default sound `confirmation_002`.
- `headsup`: orange, pulsing; stays until any button press (that press only dismisses) or 60 s; default sound `question_001`.
- `sound`: any event, tune or clip name the board knows (see Sounds); empty = the level's default; nothing plays if sounds are off.
- Up to 5 are queued and shown one at a time. They wait while an approval prompt, pairing, the menu, dictation (A held) or talking (B held) is on.
- Serial test: `event celebrate|nice|headsup <title>|<sub>|<icon>`; the `screens` demo shows all three.
