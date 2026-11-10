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
| Triple press | open the **Launcher**, the app grid (wakes the screen first if it's off; closes web settings). Its **Settings** tile is the settings menu |

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

Upside down? Settings → **Flip screen** (saved).

With **Auto-rotate** on (the default) the motion sensor turns the screen to how you
hold the stick. Held sideways, the screens with text to read (Claude approvals and
questions, alerts, the agent's replies, Home / Agent / Status, messages) switch to a
landscape layout; the A answer then sits at the end of the screen next to button A,
with an arrow. The switch waits until the stick has been still on its new side for
about half a second, and lying flat keeps the last one. The Launcher, Quick settings and
the Faces picker turn too; the settings menu and the firmware update screen stay portrait.
Settings → **Landscape screens** turns the landscape layouts
off (portrait only, as before 1.9.0); with Auto-rotate off the screen stays portrait
and follows **Flip screen**. Serial `screens` (or `screens landscape`) previews them.

### Faces (firmware 2.1.0)

Home has **faces**: pick one on the stick (Launcher → **Faces**), on the Web settings page, or on the
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


### Launcher (firmware 2.5.1)

Triple-press B opens the **Launcher**: big tiles for the stick's apps, six a page (2 x 3
held upright, 3 x 2 held sideways), with page dots in the top bar and what the selected tile
does under the grid.

| Tile | Opens |
|---|---|
| **Games** | the games (below) |
| **Remote** | the IR remote (below) |
| **Air mouse** / **Presenter** / **Media** | that remote mode (below) |
| **Faces** | the face picker: every face by name, the one in use ticked, a line about the selected one. A next, B uses it (saved) and shows Home, hold B back |
| **Quick** | Quick settings (below) |
| **Settings** | the settings menu (section 6a) |

| Button / motion | |
|---|---|
| A | next tile (round all the pages) |
| B (click) | open it |
| Hold B | back to Home |
| Flick the wrist right / left | next / previous page |

It opens on the tile you opened last, and closes by itself after 60 s without a press. The
apps go back to **Home** when they end, as before (the games' and the Remote's *hold B*, the
remote modes' *hold A + B*, Settings' *Save & exit* / *Cancel*); Quick settings and Faces go
back to the Launcher. Approvals, questions and pairing requests still take the screen; the
Launcher is there again after. Started from the Mac (`@mode`, an IR learn) while the Launcher
is open, a remote mode closes it.

**Quick settings** (Android's quick toggles): two sliders and six tiles, each change applied
and saved at once (no Save):

| Item | B does |
|---|---|
| Brightness | 25 → 50 → 75 → 100 % |
| Volume | 25 → 50 → 75 → 100 % (plays a test sound; turns Sounds on if they were off) |
| Sounds | on / off (mute) |
| Wispr mic | the wireless Wispr mic on / off |
| 24-hour | the 24-hour clock on / off |
| Rotate | auto-rotate on / off |
| Gestures | lift / face-down / shake together: all off, or (all off) all on. Each one alone: Settings |
| Settings | opens the settings menu |

A next, B change, hold B back to the Launcher. Bright tiles are on.

**From the dashboard:** the Stick page's *Launcher* card puts the tiles in another order and
hides the ones you don't use (Quick settings and Settings always stay). It's kept on the
stick (`@launcher tiles=games,air,quick,…`, PROTOCOL.md). `make -C tools/face_shots
launcher` draws every Launcher screen, portrait and landscape; the logic (tile lists, grid,
pages) is `src/logic/launcher.*` (tested by `test_launcher`), the session
`src/launcher_session.*`, the drawing `src/launcher_ui.*`.

### Games (firmware 2.5.0)

Launcher (triple-press B) → **Games**: a picker with each game's best score (A next, B play,
hold B back to Home). Every game works the same way: a title card (A starts), **B or hold B
pauses** (Resume / Restart / Quit: A next, B pick), game over (A again, B the picker). The
bests are kept on the stick (`@games` lists them); with sounds on there are little effects
at the stick's volume. StickyBot is the hero where it fits.

| Game | Play | Score |
|---|---|---|
| **Stack** | A drops the sliding block; what hangs over is cut off. Within 3 px is *perfect* (+2), three perfects in a row widen it again | blocks (+ perfect bonus) |
| **Flappy** | tap A to flap StickyBot through the pipes | pipes passed |
| **Dino** (landscape) | A jumps, hold A to jump higher; drones fly at head height: stay down; it speeds up | distance |
| **Reaction** | press A when the screen turns green; three rounds; a press too soon ends it | average ms (lower is better) |
| **Maze** | tilt to roll the ball to the glowing exit; each level is bigger and gives 15 s back | levels cleared |
| **Balance** (landscape) | the wind tips the beam more and more: tilt against it, keep the ball in the green middle | 10 a second in the middle |
| **Catch** | tilt to move StickyBot's basket: coins +1, stars +3; a bomb caught or a coin missed costs a heart | coins and stars |

The tilt games calibrate "flat" during a 2 s countdown, so you can play holding the stick
at any comfortable angle. Each finished game tells the Mac app, which gives StickyBot a
little XP (at most 10 a day from games; PROTOCOL.md, *Games*). An approval or a question
from Claude still takes the screen (the game pauses). While the games are open the CPU runs
at 160 MHz instead of 80. `make -C tools/face_shots games` draws every game screen on the
Mac (and prints each game's frame cost); the rules are in `src/logic/games*` (tested by
`test_games`), the drawing in `src/games/draw.cpp`, the session in `src/games/games.cpp`, and
`src/games/duo.h` is the (not yet built) hook for two sticks over ESP-NOW.

### Air mouse, presenter, media remote (firmware 2.5.0)

Launcher (triple-press B) → *Air mouse*, *Presenter* or *Media*, the dashboard's Stick page
(*Mouse & presenter*) or serial `mode mouse|presenter|media|home`. Hold **A + B** together for
a second to go back to Home (not the Launcher: you're usually done with the stick). Started
from the Mac while the Launcher, the settings menu, the games or the Remote screen are open,
it closes them (the menu's unsaved changes are dropped).

| Mode | Buttons | Motion |
|---|---|---|
| Air mouse | A = left button (hold + move = drag), B = right click, A+B tap = pause | turn to point; hold B and tilt to scroll |
| Presenter | A = next slide, B = previous, double B = black screen, hold B = restart the timer, A+B tap = pause the timer | hold A = laser pointer |
| Media | A = play / pause, B = mute | twist = volume, flick right / left = next / previous track |

The pointer follows the world, not the stick: turning around gravity moves it sideways and
tipping moves it up and down, held like a remote, upright or sideways. Set down, it stops at
once and re-learns the gyro's drift; untouched for 5 minutes the mode ends. Settings (pointer
speed, invert, scroll speed / direction, talk length, arrow or page keys) live on the stick
(`@air`, PROTOCOL.md). Screens: `make -C tools/face_shots air`.

**After updating from 2.4 or older:** the stick added a mouse to its Bluetooth keyboard, and
tells the Mac its services changed. If the air mouse screen says *Pair again once*, or the
pointer doesn't move: Mac → System Settings → Bluetooth → the stick → (i) → Forget This
Device, then Connect again and accept the code on the stick.

**Tuning on a real stick** (the defaults are from the maths, not a hand): the pointer gain
and acceleration (`gainFor`, `accelFactor`), dead zone and smoothing are in
`src/logic/air_motion.cpp`; the stillness thresholds in `air_motion.h`; serial `imulog`
prints the raw gyro to check the axes and signs.

### IR remote (firmware 2.5.0)

Launcher (triple-press B) → **Remote**: the remotes the stick has learned, then their
buttons (favourites first). Remotes: A opens, B next, B-B previous, hold B back to Home.
Buttons: A sends (hold to repeat), B next, B-B previous, B-B-B favourite, hold B back to the
remotes. Tilting sideways scrolls. **+ Learn**: point the remote at the stick's top and press
the button twice (air conditioners once). Up to 16 remotes of 48 buttons, kept in the
stick's flash; NEC, Samsung, LG, Sony, RC5 / RC6 and Panasonic are recognised, anything
else is kept as raw timings. The dashboard's **Remote** page does the same from the Mac
(and has built-in TV codes); quick commands and the brain can press a button ("turn off the
TV"). Serial / Bluetooth: `ir list|send|learn|diag ...` (`@ir`, PROTOCOL.md, *IR remote*);
`ir diag` checks the IR pins (TX G46, RX G42; build with `-D IR_PINS_SWAPPED` if they come
out the other way round). The codes are in `src/logic/ir_*` (tested by `test_ir`), the RMT
driver and the screen in `src/ir/`.

---

## 6. Changing the combo — three ways, no reflashing

The combo is saved in the stick's flash (NVS) and used immediately.

### a) On the stick: triple-press B → Settings

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

2.5.1: Games, Remote (IR), Air mouse, Presenter, Media remote and the face picker moved out
of this menu to their own tiles in the Launcher; brightness, volume, sounds, the Wispr mic,
the clock, auto-rotate and the gestures are in Quick settings too (applied at once there).

Button A moves to the next item. The top bar shows the combo you're building.
The menu cancels itself after 60 s without input (back to Home).

### b) In the browser: Web settings

Turn it on from Settings (**Web settings**). Wi-Fi stays off the rest of the
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
| Screen upside down | Settings → **Flip screen**. |
| "Not connected" when pressing A | Pair it (step 4). If it was paired before, see "Re-pair from scratch". |
| `stickybot.local` doesn't open | Use the IP shown on the stick's screen. |
| Page unreachable on the hotspot | Make sure the Mac joined **stickybot-setup**, then open http://192.168.4.1. |

### Re-pair from scratch

1. Mac: **System Settings → Bluetooth** → **StickyBot** → **(i)** → **Forget This Device**.
2. Stick: Settings → **Forget pairing** (press B twice), or serial `forget`.
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
| `test_air` | the air mouse: the gyro to pointer mapping in any orientation, dead zone / smoothing / acceleration / sensitivity / invert, stillness and the gyro bias, tilt-scroll, the media knob (twist, flick), the remote buttons (click, hold, double, A+B), the talk timer |
| `test_ir` | the IR remote's codes: every protocol's encoder against its decoder and published codes, a receiver's distortion, repeats and toggles, raw captures, code strings, names and the `@ir` command line |
| `test_games` | the games: the list and bests, the tilt (calibration, dead zone, smoothing, rotations), Stack's cuts and perfects, Flappy / Dino / Balance / Catch played by autopilots, Reaction's timing and false starts, the mazes (perfect, reachable, the same from a seed) and the ball never passing a wall |
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
    ├── games/         the games (2.5.0): games.* the session, draw.* the screens, duo.h two-stick hook
    ├── air.*, air_ui.* the air mouse / presenter / media remote (2.5.0)
    ├── ir/            the IR remote (2.5.0): RMT driver, store, the Remote screen
    ├── logic/         the hardware-free logic, covered by the native tests (test/)
    ├── menu.*         on-device settings menu (the Launcher's Settings tile)
    ├── launcher_*     the Launcher: session, drawing (logic in logic/launcher.*)
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
