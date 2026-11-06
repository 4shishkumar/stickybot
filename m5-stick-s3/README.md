# StickyBot — the M5StickS3 firmware

Turns an M5StickS3 into a Bluetooth keyboard with one programmable key combo,
a status screen, and settings you can change **without reflashing** — from a
menu on the stick, a settings page in your browser, or serial commands.

Default combo: **⌥ Option + ⌘ Command** (e.g. to trigger Wispr Flow).

| Button A (front) | Mac sees |
|---|---|
| Single press | combo once |
| Double press | combo twice |
| Hold         | combo held until you let go (screen shows **Listening...**); over USB the stick's mic streams to the Mac as a USB microphone meanwhile (pick **<name>-usb-mic**, e.g. *stickybot-usb-mic*, as the mic in Wispr Flow) |

| Button B (side) | |
|---|---|
| Click | Enter |
| Double press | unlock the Mac (types the saved Mac password + Return) |
| Hold | talk to the StickyBot Mac app (`../mac-agent`): speak, release — works over Bluetooth, no cable |
| Triple press | open the settings menu (wakes the screen first if it's off; closes web settings) |

The stick's third button (power/reset, on the other side) is handled by the
power chip: hold it for download mode, see "Flashing" below.

---

## 1. One-time setup: install PlatformIO

Same as the XIAO project — skip if you already did it there.

```sh
brew install python@3.14
python3.14 -m venv ~/.platformio/penv
~/.platformio/penv/bin/pip install platformio
```

Optional, so `pio` works without the full path — add to `~/.zshrc`:

```sh
export PATH="$HOME/.platformio/penv/bin:$PATH"
```

The commands below assume that; otherwise use `~/.platformio/penv/bin/pio`.
The first build downloads the toolchain and libraries (a few minutes); `git`
must be installed (it is on macOS with the Xcode command-line tools).

---

## 2. Back up the factory firmware (once, before the first flash)

A new StickS3 ships with M5Stack's demo firmware. Save a full copy of its
flash so you can go back to exactly that state later:

```sh
tools/backup.sh
```

It reads all 8 MB (bootloader, partition table, firmware, settings) into
`backup/sticks3-factory-<date>.bin`, verifies the copy against the device, and
writes the chip info + SHA-256 next to it. Keep the `backup/` folder somewhere
safe; it's specific to this stick.

If the port isn't found, hold the side power/reset button until the green LED
flashes (download mode) and run it again.

### Restore it later

```sh
tools/restore.sh backup/sticks3-factory-<date>.bin
```

Writes the backup back and verifies it, then press the power/reset button once.
The stick is then exactly as it was when you backed it up, including any
settings the factory firmware had.

**No backup?** Use M5Stack's **M5Burner** app (free, no account): put the
stick in download mode, pick the StickS3 firmware (factory demo or UIFlow2),
click Burn.

---

## 3. Build and flash

1. Connect the StickS3 to the Mac with a USB-C **data** cable.
2. Check it shows up: `ls /dev/cu.usbmodem*`
3. From this folder (`m5-stick-s3/`):
   ```sh
   pio run -t upload
   ```
   Done when you see `Hash of data verified` and `[SUCCESS]`.

**If upload fails or the port doesn't appear**, put the stick in download mode:
hold the **side power/reset button** until the green LED inside flashes, then
run the upload again. Press the power/reset button once afterwards to start
the new firmware.

Re-flashing keeps your saved combo, Wi-Fi and Mac pairing. Only
`pio run -t erase` wipes them.

---

## 4. Pair with the Mac (first time only)

1. **System Settings → Bluetooth**.
2. Click **Connect** next to **StickyBot** (or the name you gave it).
3. If the Keyboard Setup Assistant pops up, close it.

The top bar on the stick turns green: **Connected**. From then on it reconnects
by itself whenever it's on and near the Mac (1–3 s).

How that works: when you click Connect, the Mac and the stick exchange
encryption keys and both save them ("bonding") — the Mac in its Bluetooth
list, the stick in flash. On every later start the stick advertises, the Mac
recognises the bonded device and reconnects with the saved keys, no prompt.
If either side forgets the bond (Forget on the Mac, or "Forget Mac pairing"
on the stick), both must be paired again.

For a closed, docked MacBook: turn on **Allow Bluetooth devices to wake this
computer** (Bluetooth → Advanced, or Keyboard settings on newer macOS).

---

## 5. The screen

Portrait layout:

```
┌───────────────┐
│ ● Connected   │   green = connected, orange = waiting
│   ▭  +80%     │   battery, + = charging
│ ───────────── │
│ Button A sends│
│    Option     │   combo, one part per line
│    + Cmd      │
│               │
│  B: settings  │
└───────────────┘
```

Upside down? Menu → **Flip screen** (saved).

With **Auto-rotate** on (the default) the motion sensor turns the screen to how you
hold the stick. Held sideways, the screens with text to read (Claude approvals and
questions, alerts, the agent's replies, Home / Agent / Status, messages) switch to a
landscape layout; the A answer then sits at the end of the screen next to button A,
with an arrow. The switch waits until the stick has been still on its new side for
about half a second, and lying flat keeps the last one. The menu and the firmware
update screen stay portrait. Menu → **Landscape screens** turns the landscape layouts
off (portrait only, as before 1.9.0); with Auto-rotate off the screen stays portrait
and follows **Flip screen**. Serial `screens` (or `screens landscape`) previews them.

### Faces (firmware 2.1.0)

Home has **faces**: pick one in the menu (**Face**), on the Web settings page, or on the
dashboard's Stick page (with pictures and a **Preview** button that plays a short demo on
the stick). Every face keeps the time, the battery / Bluetooth and anything urgent (an
approval or question waiting, the Mac not connected, Bluetooth off, the battery low) on
screen. Approvals, questions, alerts and the dock clock still take over as before.

- **Classic** (default): the big clock, the weather and the agent's card.
- **StickyWorld**: StickyBot (a little stick with the agent's eyes) in a pixel world. The
  sky follows the time (dawn, day, dusk, night with stars and the moon) and the weather
  (clouds, rain, snow, fog, a storm's flashes). A city grows along the horizon: one building
  per merged PR, a tower per release, the same skyline every time (it squeezes, then
  scrolls, to fit). Moods: alert (it holds up a sign with what waits for you), sad (CI
  failed), curious (Claude sessions running), sleepy (night), happy (CI green). It eats a
  snack when a PR merges (a short pop if sounds are on), celebrates releases and level-ups
  with confetti and hops when CI goes green or Claude finishes. Toys: shake = dizzy, flick
  back to Home = backflip, upside down = panic, pick it up = it says hi. Along the ground:
  level + XP, the day streak, the city's size. It earns a hat (5-day streak), sunglasses
  (10 green builds) and a crown (100 PRs).
- **Aquarium**: each running Claude Code session is a fish (bigger the longer it runs,
  coloured by project, its name shown when it arrives); when the session finishes, its fish
  leaps out. Two ambient fish keep it company. A merged PR is a big bubble that pops with a
  sparkle, a release a golden one; review requests bring a crab with a tiny sign; failed CI
  turns the water murky until it's fixed. Tilt the stick and the water stays level, the fish
  drift to the low side; shake it for a burst of bubbles. At night the fish sleep.
- **Big Time** (2.2.0): just the time, as big as it gets (hours over minutes in portrait),
  the date small under it, the battery as a hairline.
- **Words** (2.2.0): the time the way you'd say it, "twenty-five to eleven", to the nearest
  five minutes ("just after" / "nearly" in between), the hour in teal.
- **Analog** (2.2.0): an anti-aliased dial with a ticking second hand (5 redraws a second
  on USB, 1 on battery), the date and the weather with the day's high and low.
- **Dashboard** (2.2.0): the time, date, weather + high / low, battery / Bluetooth, tiles for
  what needs you, review requests, Claude sessions and CI (+ PRs merged today), and the
  agent's current card, in one grid.
- **Space** (2.2.0): StickyBot in a bubble helmet over a ringed planet. Claude sessions are
  satellites in orbit; a merged PR launches a rocket (a release: a golden one, confetti),
  CI green / Claude done is a loop-the-loop, CI failed blinks its antenna red; signs and the
  agent's card float by like in StickyWorld. Shake = warp speed, lift = it waves.
- **Lava Lamp** (2.2.0): wax blobs warm, rise, cool and sink (one more per Claude session);
  purple / pink when calm, amber while Claude works, red when CI failed, pulsing orange when
  something waits for you. A merged PR sends a big blob up; shake it to swirl, tilt it and
  the blobs lean.

StickyWorld, Aquarium, Space and Lava Lamp show the clock big (2.2.0: 44 px digits across
the top in portrait, 56 px in landscape) with a dark rim so it reads over the scene.

The animated faces (StickyWorld, Aquarium, Space, Lava Lamp) run at ~20-25 fps on USB;
on battery 12-15 fps, 6-7 fps after 20 s without a press (the screen sleeps soon after
anyway). Classic, Big Time, Words and Dashboard only redraw when something changes. The data comes from the Mac agent (see
PROTOCOL.md, *Faces*); the stick keeps the last copy in NVS so they draw without it.
Serial `screens faces` plays every face's demo (`face demo <id>` one of them; A or B
stops it); `face <id>` picks one.

Adding a face: one file in `src/faces/` (subclass `faces::Face`, see `classic.cpp`)
and one `X(myFace)` in the `FACES` list in `src/faces/faces.cpp` (a clock with no world to
show: its id in `QUIET` in `demo.cpp`, for the short demo). To look at them without
a stick, `make -C tools/face_shots` draws every demo step with the firmware's own drawing
code into `tools/face_shots/out/` (PNGs and contact sheets; `SCALE=3` for big pixels);
`make -C tools/face_shots previews` remakes the dashboard's pictures.

- While you hold button A, the screen turns purple: **Listening...**
- The screen turns off after 20 s idle. Button A still works with the screen
  off (it sends the combo and wakes the screen). Triple-press B to wake it
  without sending anything.
- The battery level is also reported to the Mac over Bluetooth, so it should
  appear in the Bluetooth menu like other keyboards.

---

## 6. Changing the combo — three ways, no reflashing

The combo is saved in the stick's flash (NVS) and used immediately.

### a) On the stick: triple-press B → menu

| Item | Button B does |
|---|---|
| Ctrl / Option / Shift / Cmd | toggle that modifier on/off |
| Key | next key (hold B to scroll fast); `(none)` = modifier-only |
| Test combo | sends the combo you're editing |
| Web settings | turns on the browser settings page (below) |
| Auto-rotate | turn the screen with the motion sensor |
| Landscape screens | held sideways, the reading screens turn landscape (needs Auto-rotate) |
| Flip screen | rotate the screen 180° (saved) |
| Forget pairing | press twice to confirm; stick restarts |
| Save & exit / Cancel | |

Button A moves to the next item. The top bar shows the combo you're building.
The menu cancels itself after 60 s without input.

### b) In the browser: Web settings

Turn it on from the menu (**Web settings**). Wi-Fi stays off the rest of the
time to save battery, and switches itself off after 10 min with no page use.

- **First time (no home Wi-Fi saved):** the stick creates its own hotspot. The
  screen shows its name (**stickybot-setup**) and the password (**macro-setup** until you change it). Join that network
  on your Mac, then open **http://192.168.4.1**. Your Mac is off the internet
  while joined; switch back afterwards.
- **Easier after that:** on the page, save your home Wi-Fi under *Home Wi-Fi*.
  Next time you open Web settings the stick joins your network instead, and
  the page is at **http://stickybot.local** (the IP is also on the screen).
  If joining fails within 15 s it falls back to the hotspot.

On the page: pick modifiers + key, or type a combo like `cmd+shift+4`,
then **Save**. **Send test** fires it on the Mac. Press button B on the stick
to close web settings.

### c) Over USB: serial commands

```sh
pio device monitor
```

| Command | Does |
|---|---|
| `show` | current combo, connection, battery |
| `set cmd+opt` | set the combo (`set ctrl+space`, `set cmd+shift+4`, `set f5`, …) |
| `web` / `web off` | turn the settings page on/off |
| `forget` | forget the Mac pairing and restart |
| `help` | list commands |

Combo names (case doesn't matter): modifiers `cmd`, `opt`/`option`/`alt`,
`ctrl`, `shift`; keys `a`–`z`, `0`–`9`, `f1`–`f20`, `space`, `return`/`enter`,
`esc`, `tab`, `backspace`, `delete`, `up`/`down`/`left`/`right`, `home`, `end`,
`pageup`, `pagedown`, and `- = [ ] \ ; ' ` , . /`.

Note: the Mac's **Fn/Globe** key can't be sent — it isn't a standard keyboard
key. Use a regular combo for Wispr Flow (e.g. ⌥⌘, the default).

---

## Serial log

`pio device monitor` also shows what's happening:

```
ready StickyBot fw 1.9.0 — combo Option+Cmd. Type 'help' for commands.
Mac connected
-> Option+Cmd down
-> Option+Cmd up
Web: hotspot stickybot-setup -> http://192.168.4.1
Web: combo set to Ctrl+Space
```

Quit with **Ctrl+C**. Opening the monitor may restart the stick.

---

## Troubleshooting

| Problem | Fix |
|---|---|
| Port doesn't appear / upload fails | Data cable? Then download mode: hold the side power/reset button until the green LED flashes. |
| Screen black after flashing | Press the power/reset button once. |
| Screen upside down | Menu → **Flip screen**. |
| "Not connected" when pressing A | Pair it (step 4). If it was paired before, see "Re-pair from scratch". |
| `stickybot.local` doesn't open | Use the IP shown on the stick's screen. |
| Page unreachable on the hotspot | Make sure the Mac joined **stickybot-setup**, then open http://192.168.4.1. |

### Re-pair from scratch

1. Mac: **System Settings → Bluetooth** → **StickyBot** → **(i)** → **Forget This Device**.
2. Stick: menu → **Forget pairing** (press B twice), or serial `forget`.
3. Pair again (step 4).

---

## Security notes

- Pairing is "Just Works" (no passkey): while the stick isn't connected to your
  Mac, any nearby computer could pair with it.
- The web page has no login. It only runs while you've turned it on, but anyone
  on the same Wi-Fi (or the hotspot) could change settings during that time.
- The home Wi-Fi password is stored unencrypted in the stick's flash.

Fine for a shortcut button. Don't use this as-is to type passwords.

---

## Tests

The pure logic lives in `src/logic/` (no Arduino, no hardware) and is unit-tested
on the Mac with PlatformIO's `native` platform and Unity:

```sh
make test      # = pio test -e native   (45 tests, ~3 s)
make build     # the stick firmware still builds with the same logic
```

| Suite (`test/…`) | Covers |
|---|---|
| `test_adpcm` | IMA-ADPCM codec: frame header layout, odd sample counts, round-trip quality, frames decoding on their own, clamping |
| `test_clip` | sound clips from the Mac: packet queue parsing, frames placed by sequence number (a lost frame = silence) |
| `test_combo` | combo parsing (aliases, case, specials, errors) and display; every key round-trips |
| `test_battery` | voltage → %, median, trend slope, "full in" / "left" estimate, the last-hour RTC ring (wrap, dedupe, reset after long sleep) |
| `test_clock` | 12-hour clock, AM/PM, dates, UTC offsets across midnight, night window |
| `test_text` | word wrap and "…" cutting (UTF-8 safe) |
| `test_gestures` | lift, face-down, shake, flick, orientation and sideways detection from synthetic IMU samples |
| `test_queue` | the event queue (FIFO, drops the oldest when full) |
| `test_pet` | faces: levels / XP, streak, unlocks, the mood priority, what changed between two world messages |
| `test_city` | StickyWorld's city: the same buildings from a seed, towers in place, squeeze then scroll to fit |
| `test_sky` | the sky by time of day and weather, sun and moon arcs |
| `test_school` | Aquarium: fish per session (swim in, grow, leap out), the full tank, tilt drift, the damped spring |
| `test_clockface` | the Words face's time in words (fives, "just after" / "nearly", midnight / noon, the hour rolling over), the Analog face's hands, words stacked to fit |

Hardware-facing code (screens, Bluetooth, audio I/O) isn't covered by these;
`combo.cpp` checks at compile time that the shared key table matches the
keyboard library's codes.

## Project layout

```
m5-stick-s3/
├── platformio.ini     board config (esp32-s3-devkitc-1 + 8 MB flash/PSRAM) and libraries
├── tools/
│   ├── backup.sh      full-flash backup of the stick (run before first flash)
│   └── restore.sh     write a backup back
├── backup/            your backups (created by backup.sh)
└── src/
    ├── main.cpp       buttons, modes (status / menu / web), serial commands
    ├── combo.*        combo type, key names, "cmd+opt" parsing
    ├── settings.*     saved settings (NVS flash)
    ├── hid.*          Bluetooth keyboard
    ├── ui.*           screen drawing, screen sleep (ui_draw.h: the kit the faces share)
    ├── faces/         Home faces: registry + world (faces.*), classic, stickyworld, aquarium
    │                  (each with its Dark variant), bigtime, words, analog, dashboard, space, lava, demo
    ├── menu.*         on-device settings menu
    ├── web.*          Wi-Fi + settings page server
    └── web_page.h     the settings page (HTML/JS)
```

Built with [pioarduino](https://github.com/pioarduino/platform-espressif32)
(ESP32 Arduino core 3.x), [M5Unified](https://github.com/m5stack/M5Unified),
[M5PM1](https://github.com/m5stack/M5PM1),
[NimBLE-Arduino](https://github.com/h2zero/NimBLE-Arduino) and
[HijelHID_BLEKeyboard](https://github.com/HijelHub/HijelHID_BLEKeyboard).

## Roadmap

- **M1 (this):** combo button, status screen, battery, screen sleep, settings menu + web page.
- **M2:** multiple Macs (switch between paired Macs, disconnect without forgetting), profiles on button B, media keys.
- **M3:** Mac helper app ("StickLink"): notifications on the stick (on-air, meetings, Claude Code prompts).
- Later: motion gestures, wired USB keyboard mode, Wi-Fi firmware updates.
