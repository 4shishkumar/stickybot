# Changelog

All notable changes to StickyBot (formerly stick-key) — the Mac app (`stickybot`) and the
M5StickS3 firmware — are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/): `vX.Y.Z` is a stable release,
`vX.Y.Z-beta.N` a beta (a GitHub prerelease, offered on the beta channel).

One version number covers the release (the agent's `__version__`); the stick
firmware keeps its own `FW_VERSION`, and each release's `manifest.json` lists both.
How to cut a release: [docs/RELEASING.md](docs/RELEASING.md).

## [Unreleased]

## [2.3.1] - 2026-11-10

### Fixed
- **The v2.3.0 release build failed** before publishing anything: its test machine didn't
  install `cryptography`, which the face-package tests need. v2.3.1 is v2.3.0 with that fixed.

## [2.3.0] - 2026-11-10

### Added
- **Face packages, the design.** A format for Home faces made of
  data (a manifest + small pixel-art images) that anyone can publish and the dashboard will
  install over Bluetooth without reflashing: `docs/faces/` (format, guidelines, catalog,
  plan, JSON Schema), a validator / compiler / catalog builder in `tools/faces/`, a
  reference renderer in face_shots (`make -C m5-stick-s3/tools/face_shots pkg PKG=…`) and
  three examples in `faces/examples/`.
- **The stick draws face packages (firmware 2.4.0).** Faces made of data can be installed on
  the stick over Bluetooth (up to 12, 64 KB each, kept in its flash) and picked like any face
  in its menu, with `@face id=…`, or played as a demo. The stick checks every face it gets as
  if it came from a stranger: a face that doesn't load is never half-drawn (Classic shows
  instead and the stick says why), a face that is too slow drops to a calmer frame rate, and
  one that crashes the stick twice in a row is set aside until it's installed again. A small
  toast over Home shows the install ("Installing Cat Nap 60 %"); B cancels it.
  (PROTOCOL.md, *Face packages over Bluetooth*.)
- **A 24-hour clock (firmware 2.4.0).** A setting in the stick's menu (**Clock**) and on the
  dashboard's Stick page: every face's clock, the dock clock and the demos show 15:05
  instead of 3:05 PM.
- **More faces, in the dashboard (stick firmware 2.4.0+).** The Stick page's *More faces*:
  the faces added to the stick (Use, Update, Remove, or "didn't load on your stick"), *Get
  more faces* from the catalog (previews, filters for minimal / fun / utility and official /
  community, install progress, errors in words) and *Install from file…* for people making
  faces. The app fetches the catalog from GitHub Pages and keeps a copy for offline
  browsing, checks its ed25519 signature and every face file (size, checksum, contents,
  the stick's firmware) before sending it over Bluetooth. A stick older than 2.4.0 is asked
  to update first. CI checks face pull requests (validation, re-rendered previews, version
  raised) and publishes the signed catalog.
- **Air mouse, presenter and media remote (firmware 2.5.0).** The stick is now a mouse too
  (Bluetooth keyboard + media keys + mouse). Three modes, from the stick's menu (the last
  items), the dashboard's Stick page (*Mouse & presenter*) or `@mode`:
  - **Air mouse**: turn the stick to point (gyro, follows how you hold it: like a remote,
    upright or sideways). A = left button (hold to drag), B = right click, hold B and tilt
    to scroll, A+B together pauses. Set down, it stops at once and re-learns the gyro's
    drift; untouched for 5 minutes it goes back to Home.
  - **Presenter**: A = next slide, B = previous, hold A = laser pointer (the mouse pointer
    moves while held), double-press B = black screen. A big talk timer (starts on the first
    click; orange near the target, red past it), the battery. Keynote, PowerPoint, Google
    Slides (arrow keys, or Page Down / Up).
  - **Media remote**: twist like a knob for volume, flick right / left for the next / previous
    track, A = play / pause, B = mute.
  - Hold A and B together for a second to go back to Home. Pointer speed, invert, scroll
    speed and direction, the talk length and the slide keys are on the dashboard and in
    `@air` (PROTOCOL.md).
  - **Updating from an older firmware:** the Mac learns about the new mouse when the stick
    reconnects (the stick tells it its services changed). If the air mouse says *Pair again
    once* (or the pointer doesn't move), forget the stick in the Mac's Bluetooth settings and
    connect it again, once. The keyboard, the password key and the combos work as before.
- **Games on the stick (firmware 2.5.0).** A **Games** entry in the stick's menu with seven
  little games and their best scores: **Stack**, **Flappy** and **Dino** (StickyBot as the
  hero), **Reaction** with button A; **Maze** (a new maze every level), **Balance** and
  **Catch** by tilting the stick (it calibrates "flat" when a game starts). The same pause
  (B: Resume / Restart / Quit) and game over in every game, little sound effects (off with
  the stick's sounds, at its volume). Each game played gives StickyBot a little XP (+1, +3
  for a new best, at most 10 a day from games), and the dashboard's Stick page has a Games
  card with the bests (`@games`; PROTOCOL.md, *Games*).
- **The stick is an IR remote (firmware 2.5.0), experimental.** Sending is untested and
  learning hasn't heard a remote yet on a real stick; a fix is in progress. It learns the buttons of your TV, air
  conditioner or soundbar (menu → **Remote** → *+ Learn*, or the dashboard: press the button
  twice; air conditioners once) and keeps them in its flash, up to 16 remotes of 48 buttons.
  NEC (and extended), Samsung, LG, Sony 12/15/20, RC5, RC6 and Panasonic/Kaseikyo are
  recognised; anything else (air conditioners' long state frames) is kept as raw timings.
  The Remote screen sends them (A, hold to repeat; favourites first). Receiving and sending
  use the ESP32's RMT peripheral (our own encoders/decoders with native tests: IRremoteESP8266's
  release is written for the core-2 timer API, and neither it nor Arduino-IRremote receives with RMT). `@ir list|send|learn|
  rm|rename|fav|diag` (PROTOCOL.md, *IR remote*); `@ir diag` checks the IR pins (G46 / G42).
- **Remote, in the dashboard.** A new page: the remotes on the stick (send, star, rename,
  delete), learning with live status, built-in codes for Samsung, LG, Sony and Philips TVs
  that work without learning, import / export as JSON.
- **"Turn off the TV".** Quick commands can press a remote button (`"ir": {"dev": "tv",
  "btn": "power"}`); new phrases for TV power, volume and mute are added to your commands.
  The brain gets `ir_list()` / `ir_send()` stick tools, so "set the AC to 24" picks the learned
  button by its name.
- **A Launcher on the stick (firmware 2.5.1).** Triple-press B now opens a grid of big tiles
  (six a page, 2 x 3 upright and 3 x 2 held sideways; A next, B open, hold B back to Home,
  flick the wrist for the next page) instead of the long settings list: **Games**,
  **Remote**, **Air mouse**, **Presenter**, **Media**, **Faces** (a full-screen face picker
  with the one in use ticked), **Quick settings** and **Settings**. It opens on the tile you
  used last. **Quick settings** has Android-style toggles that apply and save at once:
  brightness and volume sliders, sounds (mute), the wireless Wispr mic, the 24-hour clock,
  auto-rotate, the motion gestures, and a way into Settings. **Settings** is the old menu
  without the apps and the face picker (34 items down to 28). The apps still go back to
  Home when they end (the remote modes' hold A + B too). The dashboard's Stick page has a
  **Launcher** card to put the tiles in order and hide the ones you don't use (`@launcher
  tiles=…`; `@state` gets `"launcher":"grapmfqs"`, one letter a tile; PROTOCOL.md).
  `make -C m5-stick-s3/tools/face_shots launcher` draws every Launcher screen.

### Fixed
- **The wireless Wispr mic failed or lagged with Bluetooth headphones as the Mac's output**
  (AirPods, OnePlus Buds…): with the Apple audio tap, the app played the stick into the
  default output and clocked the mic device by it; a Bluetooth one could fail to open
  ("Internal PortAudio error"), never run, or bring the audio in ~1.1 s late, so Wispr
  missed the start or got nothing. When the default output is Bluetooth, the app now uses
  the built-in speakers instead (another wired output on a Mac without them), and the
  inaudible tap check goes the same way. Nothing is heard: the tap mutes it there just the
  same. The log says which output is used and why.
- **The Mac app lost the stick after firmware 2.5.0** ("'CBService' object has no attribute
  'handle'", then "Writing is not permitted"): 2.5.0's keyboard + mouse moved the stick's
  Bluetooth services, and macOS handed the Battery service back in a way the Bluetooth
  library can't read. The app now asks only for its own service. If the stick still won't
  connect after updating, forget it in the Mac's Bluetooth settings and pair it again (once).
- **Other ESP32 boards on USB were taken for the stick**: the app's USB fallback sent its
  `@` lines to whatever Espressif device was plugged in. Only a device named StickyBot (or
  a chip in download mode, for flashing) counts now.
- **Air mouse** (firmware 2.5.0): a slower pointer, up/down faster so it reaches the bottom
  edge, and holding A+B for 1.2 s leaves even when one button was pressed first.

## [2.2.0] - 2026-11-09

### Added
- **Dark StickyWorld and Aquarium faces (firmware 2.3.0)**, for a desk at night, next to
  the originals (which don't change), in the same pickers with their pictures and demos:
  - **StickyWorld Dark**: the same world, always at night: a near-black sky with the moon
    and dim stars (rain, snow, clouds and storms still show), the city with a few lit
    windows, a dim ground. StickyBot's moods still follow the real time of day.
  - **Aquarium Dark**: the deep sea: near-black water with drifting specks of light, fish
    with a soft glow, glowing weed, dim sand. The fish still sleep at night.
  - On both, the big clock stays bright, and anything urgent shows as on the originals.

### Fixed
- **The stick's Bluetooth ran out of buffers** (firmware 2.3.0): a long reply to the Mac app
  (its status, with the list of faces) could use all 12, and then key presses were dropped
  (the Mac password typed by a double-press of B sometimes came out wrong), wireless-mic
  audio cut out, and the app's messages failed with "Insufficient Resource". The stick now
  has 48, a key press waits up to half a second for one, and the app sends again when the
  stick is busy for a moment.

## [2.1.0] - 2026-11-06

### Added
- **Faces for the stick's Home screen (firmware 2.1.0).** Pick one in the stick's menu
  (**Face**), on its Web settings page, or on the dashboard's Stick page (pictures, **Use**,
  and **Preview**, which plays a short demo on the stick). Every face keeps the time,
  battery / Bluetooth and anything urgent in view; approvals, questions and alerts still
  take over the screen. Portrait and landscape.
  - **Classic**: Home as before (the default).
  - **StickyWorld**: StickyBot in a pixel world. The sky follows the time and the weather;
    a city grows with your merged PRs (a tower per release). StickyBot holds up a sign when
    something waits for you, looks worried when CI fails, busy while Claude works, sleepy at
    night; it eats a snack when a PR merges and celebrates releases and level-ups. Shake,
    flick, turn it over or pick it up to play. It levels up, keeps a day streak and earns a
    hat, sunglasses and a crown.
  - **Aquarium**: a fish per running Claude Code session (it leaps out when the session
    finishes), PRs as bubbles, a crab for review requests, murky water while CI is red; tilt
    the stick and the water follows.
- **StickyBot's progress** lives in the Mac app (`~/.stick-agent/pet.json`): XP from merged
  PRs, green builds, finished Claude tasks and releases. The app sends the stick a small
  `world` message (PROTOCOL.md, *Faces*) and adds the `kind` to events; `GET /api/pet`.
- Serial `screens faces` plays every face's demo; `face <id>`, `face demo <id>`;
  protocol `@face id=…` / `@face demo=…`.
- `m5-stick-s3/tools/face_shots`: draws the faces on the Mac with the firmware's own drawing
  code and saves PNGs (the dashboard's pictures come from it).
- **Six more faces (firmware 2.2.0)**, in the same pickers, each with its demo:
  - **Big Time**: just the time, as big as the screen allows (hours over minutes in
    portrait), the date small under it and the battery as a hairline.
  - **Words**: the time the way you'd say it, "twenty-five to eleven" (to the nearest five
    minutes, "just after" / "nearly" for the ones in between), the hour in teal.
  - **Analog**: a clean anti-aliased dial with a ticking second hand, the date and the
    weather with the day's high and low.
  - **Dashboard**: everything at a glance: the time, the date, the weather with high / low,
    battery / Bluetooth, tiles for what needs you, review requests, Claude sessions and CI
    (with today's merged PRs), and the agent's current card.
  - **Space**: StickyBot floats in a bubble helmet over a ringed planet; each Claude session
    is a satellite in orbit, a merged PR launches a rocket (a release a golden one), CI green
    or Claude done is a loop-the-loop; shake it for warp speed.
  - **Lava Lamp**: wax blobs rise and sink (one more per Claude session); the colour says
    how things are (calm, Claude working, CI failed, something waiting); a merged PR sends a
    big blob up, shake it and the wax swirls, tilt it and the blobs lean.

### Changed
- **A much bigger clock on StickyWorld and Aquarium** (firmware 2.2.0), to read from across
  the desk: about twice the size in portrait (the semi-bold 44 px clock digits, centred
  across the top) and bigger still in landscape (new 56 px digits), with a dark rim so it
  reads over the sky and the water. To make room, StickyWorld's sign and speech bubble sit
  lower (in landscape, StickyBot holds the sign beside it) and the sun's arc stays under the
  clock; the Aquarium's air strip is taller. The date and the temperature sit under the
  clock in portrait, in a column on the right in landscape.

### Fixed
- After a Bluetooth update the menu bar kept saying "Stick firmware … available" for up to
  an hour: the check right after the update got a cached answer from before it. A finished
  update or a reconnect now asks the stick itself.

## [2.0.0] - 2026-11-06

**stick-key is now StickyBot.** After updating, macOS asks once more for Bluetooth (and
System Audio Recording, with the wireless mic on the Apple audio tap) — now for
*StickyBot* — and the mics have new names: **rank them in Wispr Flow → Settings →
Microphone once** (hold A away from USB for the Bluetooth one; plug the stick in for the
USB one).

### Added
- **StickyBot.app.** The installer builds `~/Applications/StickyBot.app` on this Mac and the
  login item runs it, so permission prompts, Privacy & Security, Activity Monitor and Force
  Quit say *StickyBot* (with its icon), not *Python*. Its executable is a copy of the venv's
  own Python, signed ad hoc (no Developer ID; built locally, so Gatekeeper doesn't ask). It is
  rebuilt only when Python itself changes, so code updates keep the permissions.
  `stickybot app` shows its state, `stickybot app rebuild` builds it again. If it can't be
  built the agent runs as Python as before and says so. `stickybot uninstall` removes it.
- **The dashboard in its own window:** menu bar → **Open StickyBot** (a Dock icon while it's
  open; closing it doesn't quit), or **Open in browser**. Opening StickyBot.app from Finder or
  Spotlight shows it too, and the installer's setup page opens there. New dependency:
  `pyobjc-framework-WebKit`.
- **`stickybot` command**, next to `stick-agent` (which keeps working).
- **Landscape screens (firmware 1.9.0).** Held sideways, the screens with text to read turn
  landscape: Claude approvals, questions, event alerts, the agent's listening / thinking /
  reply screens, dictation, messages, pairing, Web settings and the Home / Agent / Status
  pages. The wider lines fit more text in a bigger font. On approvals, questions and
  pairing the A button's answer sits at the end of the screen next to the real button A,
  with an arrow. The stick switches after it has been held still on a side for about half a
  second, so it doesn't flicker while you move it; lying flat keeps the last one. The menu,
  the firmware update screen and the dock clock work as before. A new setting **Landscape
  screens** (on by default; it needs Auto-rotate) is in the stick's menu, its settings page
  and the dashboard's Stick page. The serial `screens` demo now shows the landscape
  layouts too (`screens landscape` for just those).
- **The wireless Wispr mic needs no driver on macOS 14.2+.** An Apple audio tap of the agent's own
  sound now carries the stick's audio into the same Bluetooth mic (same name, so Wispr's
  ranking stays) — no BlackHole download, no Mac password. macOS asks once to allow *System Audio
  Recording* for StickyBot (it only taps its own sound). The agent checks the tap with an
  inaudible test before using it (at start, when turned on, and with **Check again**) and never lets the
  stick's audio reach the speakers. BlackHole 2ch stays for older macOS, or by choice: Settings →
  Wireless Wispr mic → **Carried by** (Automatic / Apple audio tap / BlackHole 2ch; config
  `wireless_mic_carrier`), or *Use BlackHole instead* on the setup page. The setup page's wireless mic
  step is now **Turn on** + the permission, and Settings shows which carrier is in use.

### Changed
- **Renamed to StickyBot** everywhere you see it: the menu bar, dashboard, setup page,
  notifications, installer and command output, the stick's screens and its settings page
  (*StickyBot Settings*), and releases (*StickyBot vX.Y.Z*). Internal names stay, so nothing
  needs moving: `~/.stick-agent`, the `stick_agent` package, `STICK_AGENT_*` variables, the
  login item `com.stickkey.agent`, the Keychain entry and the Bluetooth protocol.
- **New mic names:** the USB mic is `<stick>-usb-mic` and the Bluetooth one `<stick>-bt-mic`
  (were `<stick>-mic-usb` / `<stick>-mic-bluetooth`); a stick with the default name gives
  `stickybot-usb-mic` / `stickybot-bt-mic`. Wispr sees new devices: rank them once. An old
  automatic name saved as *Mic name* counts as automatic; a name you typed still wins.
- **Firmware:** new or reset sticks are called **StickyBot** (a saved name stays). With Web
  settings on, the stick is at `stickybot.local` and its hotspot is `stickybot-setup`
  (were `stick-key.local` / `stick-key-setup`); the USB device is *StickyBot*.
- **The repo is now `ashish-kumar-noon/stickybot`**:
  `curl -fsSL https://raw.githubusercontent.com/ashish-kumar-noon/stickybot/main/install.sh | sh`.
  An existing install's clone is switched to the new address by itself (install.sh and the
  dashboard's update).

## [1.8.6] - 2026-11-05

The v1.8.5 build stopped on a test that assumed it runs on a Mac; this release has the
same changes as 1.8.5 (the OpenRouter brain and the setup page).

### Fixed
- The setup page's tests no longer depend on the machine running them.

## [1.8.5] - 2026-11-05

### Added
- **Pick the brain: Claude Code or OpenRouter** (Settings → Brain). OpenRouter uses your own API key,
  pay per use, with any model from its list (prices shown, a fast cheap model suggested). Replies stream
  to the stick and are spoken sentence by sentence like Claude's; the stick tools work, and opening an
  app or a URL asks on the stick first. Each request's time and cost show in Settings, and **Test** sends
  one short request. The key is kept in the macOS Keychain, never in `config.json`, and can't be read back
  (`STICK_AGENT_OPENROUTER_KEY` overrides it for development). Switching needs no restart.
- **A setup page in the browser.** The install line (`curl … install.sh | sh`) now installs
  the app, starts it and opens http://localhost:8660/#install, which walks through the rest
  one step per screen with Install buttons and live progress: the speech model (with
  download size, speed and time left), Claude Code (Anthropic's installer; the page waits
  for the Terminal login by itself), the approval / "Claude finished" hooks (shows exactly
  what is added first), the stick tools for Claude Code, GitHub alerts (`gh` login with a
  one-time code shown on the page, then your main repo), the wireless Wispr mic and your
  stick (the existing setup wizard). Optional steps can be skipped and picked up later.
  Reopen it with `stick-agent install-page` or the menu bar's **Finish setup…**.
- The wireless mic's BlackHole 2ch installs from the page without Terminal: a pinned,
  tested package (0.7.1) downloaded over HTTPS, sha256 and signature checked, installed
  with the Mac's own password dialog.
- `stick-agent setup --only core` and `install.sh --no-browser`.

### Changed
- `install.sh` with `--yes` / `--with` / `--without` / `--dry-run` / `--uninstall`, over
  SSH, or without a browser still sets up in the terminal exactly as before.

## [1.8.4] - 2026-11-05

### Changed
- The dashboard labels the two version numbers: **Helper app** (each release's number) and
  **Stick firmware** (changes only when a release changes the stick), each marked *latest*
  or with the version available.

## [1.8.3] - 2026-11-05

### Changed
- **Mic names follow the stick's name**, so two sticks' mics differ: the USB mic is
  `<stick>-mic-usb` (firmware 1.8.1) and the Bluetooth one `<stick>-mic-bluetooth`, e.g.
  `stick-key-mic-usb` / `arbaaz-sticks3-mic-bluetooth`. Wispr sees new devices: rank them
  once more (hold A away from USB for the Bluetooth one; plug the stick in for the USB one).
  A custom *Mic name* in Settings still wins. Renaming the stick renames its mics.
- The home screen shows the stick's name (not "stick-key") until the Mac sends the time.

## [1.8.2] - 2026-11-05

### Added
- **Check for Updates…** in the menu bar (besides the daily check and Settings → Updates →
  Check now); the item says what it found.

### Fixed
- `stick-agent <command> --help` only prints the help: `stick-agent flash --help` used to
  start flashing. `stick-agent flash` refuses options it doesn't know.
- The dashboard checks for a newer copy of its page on every load, so controls added by an
  update (like *Update over Bluetooth*) show up without a hard reload.

## [1.8.1] - 2026-11-05

The first published release (the v1.8.0 build stopped on a test). Firmware 1.8.0.

### Fixed
- A component change from the dashboard always reports when it's done, even if applying
  the new setting fails.

## [1.8.0] - 2026-11-05

The first tagged release: everything on `main` so far. Firmware 1.8.0.

### Added
- **Installer:** `install.sh` (uv, its own Python 3.14 in `~/.stick-agent`, no Homebrew
  needed) and `stick-agent setup` with installable components (core, speech model,
  Claude Code hooks, GitHub alerts, firmware, wireless mic, stick tools), also shown in
  the dashboard's Components card. `--yes`, `--with`, `--without`, `--dry-run`,
  `--status`, `--uninstall [--purge]`.
- **Fn / Globe key** for button A (firmware 1.6.1): the stick re-pairs with a new
  keyboard identity when Fn is added or removed.
- **Wireless Wispr mic:** holding A away from USB streams the stick's mic over
  Bluetooth; the agent plays it into BlackHole 2ch behind a `stick-mic-bluetooth`
  microphone that exists only while A is held. Over USB the stick is the
  `stick-mic-usb` microphone (firmware 1.7.0 / 1.7.1).
- **Persistent Claude Code session:** one long-lived `claude` process (stream-json in
  and out) so a request doesn't pay the start-up cost; one-shot `claude -p` stays as the
  fallback. Replies stream to the stick while Claude writes them and are spoken sentence
  by sentence.
- **Stick tools (MCP):** an MCP server in the agent (`http://127.0.0.1:8660/mcp`) so any
  Claude Code session can notify you, celebrate, ask A/B questions and show progress on
  the stick; `stick-agent mcp install` for terminal sessions, a dashboard card to turn it
  on or off.
- **Firmware 1.7.2:** questions from Claude sessions get their own screen with the two
  answers on A and B.
- **Releases:** one version for the project, this changelog, `tools/release.sh` (local
  commit + tag) and a release workflow that tests, builds the firmware and publishes it as
  release assets with a sha256 manifest.
- **Updates:** the agent checks for new releases daily (stable or beta channel) and shows
  them in the dashboard (banner with the notes and an **Update** button), the menu bar and,
  once, on the stick. Installs made by `install.sh` update themselves in one click. Release
  firmware is downloaded and sha256-checked into `~/.stick-agent/firmware/`.
- **Firmware updates over Bluetooth (firmware 1.8.0):** no cable needed from now on —
  Stick → Firmware → *Update over Bluetooth*, or `stick-agent flash --ble`. The stick
  checks the image's sha256, and rolls back by itself if the new firmware doesn't start
  properly. (Sticks on older firmware need one USB flash to 1.8.0 first.)
- MIT licence and third-party notices; CI on pull requests (macOS runners).

### Changed
- Firmware binaries are no longer committed: they come from releases, or from
  `make -C m5-stick-s3 release` (now written to `~/.stick-agent/firmware/local/`).
- `install.sh` checks out the latest stable release (main before the first one) and
  clones over HTTPS when `gh` isn't logged in. The repo is public: install with
  `curl -fsSL https://raw.githubusercontent.com/ashish-kumar-noon/stickybot/main/install.sh | sh`.
- The PR / Devin quick commands ship with `<org>/<repo>` placeholders (the setup wizard
  fills in your repo) and GitHub alerts have no default repo.

[Unreleased]: https://github.com/ashish-kumar-noon/stickybot/compare/v2.3.1...HEAD
[2.3.1]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v2.3.1
[2.3.0]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v2.3.0
[2.2.0]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v2.2.0
[2.1.0]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v2.1.0
[2.0.0]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v2.0.0
[1.8.6]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.6
[1.8.5]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.5
[1.8.4]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.4
[1.8.3]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.3
[1.8.2]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.2
[1.8.1]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.1
[1.8.0]: https://github.com/ashish-kumar-noon/stickybot/releases/tag/v1.8.0
