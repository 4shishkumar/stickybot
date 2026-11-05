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
| `wmic` | `on=0\|1` | StickS3: the wireless Wispr mic switch (see section 5) |

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
| OTA data | `…0006…` | Mac → board | firmware image packets `[offset u32 LE][bytes]` (write without response; StickS3 1.8.0+, see *Firmware update over Bluetooth*) |

The service UUID is in the advertisement (while connected too), so Mac
programs find the board by it; they should then stay with the one board the
user picked (several boards can be in range).

**Claude Code permission prompts** (tools/claude-hook) use Claude's Hardware
Buddy messages with `"src":"cli"`; the board answers
`{"cmd":"permission","id":…,"decision":"once"|"deny"}` from buttons A / B.

**Questions from a Claude session** (the agent's stick tools, `ask()`; firmware 1.7.2 draws them)
use the same message with extra fields:
`{"src":"cli","prompt":{"id":"ask-…","tool":"Question","hint":"<question> - A: <a> / B: <b>","from":"web","kind":"ask","q":"<question>","a":"<label A>","b":"<label B or empty>"}}`.
1.7.2 shows `q` with the labels on two button rows; A answers `once` (the first option), B `deny`
(the second; with no `b`, B dismisses) and a **shake** answers `"decision":"dismiss"`. Older firmware
ignores the extra fields and shows it as an approval (A yes / B no, the options in `hint`; a shake
then answers `deny`). The agent re-sends it every 3 s like an approval, holds it back while an
approval is pending (one prompt on the screen at a time) and clears it with `{"src":"cli"}`.

The agent may also send the Hardware Buddy `{"cmd":"status"}` over this link (the stick tools'
`stick_status()`); the board answers `{"ack":"status","ok":true,"data":{"name","sec","bat":{"pct","mV","usb"},"sys":{"up","heap"}}}`.

**Mac agent** (tools/mac-agent), all messages tagged `"src":"agent"`:

| Direction | Message |
|---|---|
| Mac → board | `{"src":"agent","hello":1,"v":1}` every ≤10 s — the board shows "agent ready" for 25 s after each |
| board → Mac | `{"src":"agent","evt":"talk","state":"start","rate":16000,"codec":"ima-adpcm"}` when B becomes a hold |
| board → Mac | `{"src":"agent","evt":"talk","state":"stop"}` after the last audio frame |
| Mac → board | `"wmic":true\|false` in the hello (and on its own, `{"src":"agent","wmic":…}`, when it changes) — the agent can play the wireless Wispr mic (its `wireless_mic` is on and BlackHole 2ch was found) |
| Mac → board | `"wready":true` in the hello (agent 1.7.1+) — it answers `prepare` with `ready` (makes the *stick-mic-bluetooth* device) |
| board → Mac | `{"src":"agent","evt":"wispr","state":"prepare","id":n,"rate":16000,"codec":"ima-adpcm"}` when A goes down for Wispr away from USB and the agent said `wready` (firmware 1.7.1+); audio frames follow at once |
| Mac → board | `{"src":"agent","wispr":"ready","id":n}` — the device is there and Wispr has had its switch time: press the key. `"device":false` = no device (stick on USB, no BlackHole, CoreAudio error): press anyway |
| board → Mac | `{"src":"agent","evt":"wispr","state":"start","rate":16000,"codec":"ima-adpcm"}` instead of `prepare`, to an agent without `wready` (firmware 1.7.0 always) |
| board → Mac | `{"src":"agent","evt":"wispr","state":"stop"}` after the last audio frame (also after a press cancelled before `ready`) |
| board → Mac | `{"src":"agent","evt":"usb","mounted":true\|false}` — plugged into a computer that enumerated its USB device (firmware 1.7.1+): when it changes, to a new agent, and every 20 s with the hellos; `usbMounted` in `@state` |
| Mac → board | `{"src":"agent","state":"thinking"\|"working"\|"done"\|"error","heard":"…","text":"…"}` — shown on screen. While Claude writes, repeated `working` messages carry the answer so far (≤ ~3 per second, ≤ 300 characters); `done` (with its chime) follows once |

Audio frames: 16 kHz mono IMA-ADPCM, ≤20 ms each, little-endian:
`[seq u16][predictor i16][step index u8][samples, 4 bits each, low nibble first]`.
Each frame carries the decoder state at its start, so a lost frame only loses
its own 20 ms. `seq` restarts at 0 for every talk. The first frames are the
pre-roll (recorded since B went down) and arrive faster than real time.

**Wireless Wispr mic** (StickS3 firmware 1.7.0+; the mic device 1.7.1+): the same audio frames, for Wispr
instead of a request. The board streams while A holds the combo when all of these
hold: its own switch is on (menu *Wireless mic*, `@wmic on=0|1`, state `wirelessMic`,
default on), the agent's last hello said `"wmic":true`, the agent was heard from in
the last 25 s, it is not plugged into a computer (1.7.0: no host has the USB microphone open), and no B talk is recording. The
key goes down first (1.7.1 with a `wready` agent: see *the mic device* below) and the stream starts with it (no pre-roll: Wispr records from
key-down; the first ~100 ms are the mic's settle silence). While streaming, the
key-up waits 250 ms so the audio still on its way reaches Wispr before it stops.
`seq` restarts at 0 for every stream. The agent doesn't transcribe these: it plays
them in real time into the BlackHole 2ch virtual device (a lost frame becomes 20 ms
of silence). The speaker stays quiet meanwhile (the codec is half-duplex: cues wait
until the mic is off). Older agents never send `wmic`, so the board never streams to them.

*The mic device* (firmware 1.7.1 + agent with `wready`): Wispr is not set to BlackHole.
The agent shows a public CoreAudio aggregate device named **stick-mic-bluetooth**
(config `wispr_mic_name`; UID `com.stickkey.wireless-mic`; its only sub-device is
BlackHole 2ch, so what the agent plays is its input) only around a hold, and the user
ranks it first in Wispr's microphone list once; Wispr switches to the highest-ranked
mic present whenever devices change. Per press:

1. A down → the board sends `prepare` (id = a press counter) and streams right away, but
   **holds the key**.
2. The agent opens its output stream, creates the device if it isn't there (~100–170 ms),
   waits `wispr_switch_ms` (default 250) only if it just created it, and sends `ready`
   with the same id. A `ready` with another id is ignored.
3. The board presses the key on `ready`, or after **800 ms** without one
   (`wispr::READY_TIMEOUT_MS`). A released before that is cancelled: no key at all,
   the stream ends with `stop`.
4. Release → the 250 ms tail → key up → `stop`. The agent keeps the device
   `wispr_tail_ms` (default 800), longer while another process is still reading it
   (≤ 4 s more; removing it mid-dictation stops Wispr with *MicDisconnected*), then
   destroys it. A `prepare` in that time finds it present: `ready` at once.

The device exists only while the board is connected by Bluetooth and not USB-mounted:
the agent answers `prepare` with `"device":false` and removes the device at once on
`{"evt":"usb","mounted":true}` or when the link drops; also when `wireless_mic` is turned
off, on exit / SIGTERM / restart, and (left over from a crash) at start.

**USB wins**: while the board is plugged into a computer (TinyUSB mounted *and* VBUS
present — a charger-only cable never mounts), A is the USB microphone exactly as
before 1.7: the key at once, no `prepare`, no Bluetooth audio. The USB mic is named
**stick-mic-usb** (its audio interface string; firmware 1.7.1+, before: "TinyUSB UAC1").

**Sounds** (`"src":"agent"`):

| Direction | Message |
|---|---|
| Mac → board | `{"src":"agent","sound":{"get":1}}`, `{"sound":{"set":{"answer":"confirmation_001"}}}`, `{"sound":{"play":"answer"}}` (an event or a tune/clip name), `{"sound":{"on":true}}`, `{"sound":{"volume":0-255}}` |
| board → Mac | `{"src":"agent","evt":"sounds","sounds":{"tunes":[…],"map":{event: tune},"on":…,"volume":…}}` after each of those |

Events: sent, answer, approval, approved, denied, error, connected, disconnected, unlock, charging, lowbattery.
Tunes: built-in Kenney clips (CC0, `tools/make_sounds.py`) and synthesized beeps; `off` = silent.
The agent's hello may also carry `"home":{"k","a","b","u"}` (home screen card) and `"wx":{"t","hi","lo","i","c","day","at"}` (weather).
`{"src":"agent","home":{…}}` on its own (no `hello`) updates the card at once — the agent sends it when a Claude
session's `progress()` / `pin_home()` changes it (at most every 0.5 s); boards since the home card handle it.

**Events** (full-screen screens on the board; StickS3):

| Direction | Message |
|---|---|
| Mac → board | `{"src":"agent","event":{"id":"e123","level":"celebrate"\|"nice"\|"headsup","title":"PR #5507 merged","sub":"acme/web · feat: dark mode","icon":"merge"\|"pr"\|"check"\|"x"\|"review"\|"claude"\|"star"\|"bell","sound":""}}` |
| board → Mac | `{"src":"agent","evt":"event_shown","id":"e123"}` when it appears on screen |

- `celebrate`: confetti, a colour pulse, the happy face and the big icon, ~6 s; default sound `jingle_win` (Kenney Music Jingles, CC0).
- `nice`: a teal-to-green toast that slides in, ~4 s; default sound `confirmation_002`.
- `headsup`: orange, pulsing; stays until any button press (that press only dismisses) or 60 s; default sound `question_001`.
- `sound`: any event, tune or clip name the board knows (see Sounds); empty = the level's default; nothing plays if sounds are off.
- Up to 5 are queued and shown one at a time. They wait while an approval prompt, pairing, the menu, dictation (A held) or talking (B held) is on.
- Serial test: `event celebrate|nice|headsup <title>|<sub>|<icon>`; the `screens` demo shows all three.

### Firmware update over Bluetooth (StickS3 1.8.0+)

The agent can update the stick over this link — no USB cable, no Wi-Fi. Only the
**app image** is sent (`app.bin`, offset 0x10000 in the bundle); the bootloader,
partition table and settings (NVS) are never touched. The stick writes it to its
*other* OTA slot (`default_8MB.csv`: `ota_0` at 0x10000, `ota_1` at 0x340000,
3.2 MB each, `otadata` at 0xE000), so the running firmware stays intact until the
new one is checked and made the boot slot.

**Support:** the board's `@state` has `"ota":1` (and `"otaPending":true|false`).
Older firmware has neither, and no `…0006…` characteristic: update it over USB once.

**Messages** (all `"src":"agent"`; the board's answers are `"evt":"ota"`):

| Direction | Message |
|---|---|
| Mac → board | `{"ota":{"begin":{"size":N,"sha256":"<64 hex>","version":"1.8.1","compressed":false,"force":false}}}` |
| board → Mac | `{"evt":"ota","state":"ready","offset":0,"size":N,"resumed":false,"window":16384,"chunk":240}` — send from `offset`; at most `window` bytes past the last ack; at most `chunk` data bytes per packet (MTU − 3 − 4) |
| board → Mac | `{"evt":"ota","state":"refused","error":"busy","why":"dictation"}` — nothing started (codes below) |
| Mac → board | packets on `…0006…`: `[offset u32 LE][bytes]`, in order, write without response |
| board → Mac | `{"evt":"ota","state":"ack","offset":N}` — everything before N is written; after every 4 KB, at the end, and 150 ms after the last ack when more arrived |
| board → Mac | `{"evt":"ota","state":"ack","offset":N,"gap":true}` — a packet past N arrived, so something was lost: **resend from N** (packets after a gap are dropped; repeated every 300 ms until the resend starts) |
| Mac → board | `{"ota":{"end":1}}` once everything is acknowledged |
| board → Mac | `{"evt":"ota","state":"done","offset":N}` — size and SHA-256 match, the image is valid and set to boot; the board restarts ~1.5 s later. A repeated `end` gets `done` again |
| board → Mac | `{"evt":"ota","state":"error","error":"sha"\|"image"\|…,"offset":N}` — the transfer is over (`incomplete` is the exception: bytes are missing, the session stays; `begin` again to resume) |
| Mac → board | `{"ota":{"cancel":1}}` → `{"evt":"ota","state":"cancelled","by":"agent"}`; the stick's **B** on the update screen → `"by":"stick"` |
| Mac → board | `{"ota":{"status":1}}` → `{"evt":"ota","state":"status","receiving":…,"offset":…,"size":…,"pending":…}` |
| board → Mac | `{"evt":"ota","state":"confirmed","fw":"1.8.1"}` — after a reboot into a new image, once it has confirmed itself (see Rollback) |

Refusal / error codes: `busy` (+ `why`: `dictation`, `talk`, `approval`, `pairing`, `menu`, `wifi-update`),
`battery` (below 30 % and not on USB power; unknown counts as low), `pending` (the running image isn't
confirmed yet), `unsupported` (`compressed:true` — not implemented, send the raw image), `size` (0, bigger
than the slot, or data past the end), `sha` (not 64 hex digits, or the image read back from flash doesn't
match), `downgrade` / `same` (older / the running version without `force`), `image` (not an ESP32-S3 app
image — magic 0xE9 and chip id 9 in its header — or `esp_ota_end` refused it), `open`, `write`, `read`,
`incomplete`, `timeout` (no data or `begin` for 3 min), `not_started`.

**Flow control:** write-without-response packets can be dropped under pressure (Core Bluetooth drops them
when its queue is full; the agent waits for `canSendWriteWithoutResponse`, and the stick drops packets when
its 24 KB queue is full). The board only takes bytes in order and says where it is; the agent keeps at most
`window` bytes unacknowledged, rewinds to the offset of a `gap`, and resends from the last ack when nothing
comes back for 1 s with the window full. Duplicates are ignored (and re-acked).

**Resume:** if the link drops, the stick keeps the session (and the update screen, "Waiting for the Mac…")
for 3 minutes. A `begin` with the same `size` + `sha256` answers `ready` with `"resumed":true` and the
offset to continue from; anything else starts over.

**Checks:** the link must be encrypted (the characteristic is `WRITE_ENC`, i.e. the paired Mac); the agent
checks app.bin against the bundle manifest's sha256; the stick reads the written image back from flash,
compares its SHA-256 (mbedTLS) with `begin.sha256`, then `esp_ota_end` validates the image itself
(segments, checksum, appended hash) before `esp_ota_set_boot_partition`. Signing is not implemented yet
(hooks: `ota.verify_signature()` in the agent, before `commit()` in `ota.cpp`).

**Rollback:** the bootloader is built with `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`. A new image boots as
*pending verify*; firmware 1.8.0+ confirms it (`esp_ota_mark_app_valid_cancel_rollback`) after it has run
**25 s** with Bluetooth on, under the loop watchdog (5 s). Any reset before that — a crash, a hang (the
watchdog), a power cut — makes the bootloader start the previous firmware again. While pending, the stick
doesn't go to standby (waking is a reset) and refuses another update (`pending`).

**On the stick:** "Updating NN %" with a bar and the version; **B** cancels (nothing changes); the menu,
prompts, events and standby wait. Then "Restarting". On `begin` the stick asks the Mac for a quick connection interval, long
link-layer packets and the 2M PHY (macOS decides what it grants; apps on the Mac cannot set these).

