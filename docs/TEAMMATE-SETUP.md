# Setting up a teammate's stick

Two parts: **you** flash and name their stick on your Mac (about 3 minutes), then
**they** pair it — Wispr works straight away — and, if they want the voice agent
(hold B), run the installer on their Mac.

## Part 1 — on your Mac (once per stick)

1. **Plug their stick in by USB-C** (unplug yours, so you can't pick the wrong one).
2. **Find its port:** `ls /dev/cu.usbmodem*` — ignore the LG monitor ones.
3. **Flash it** (the agent uses the latest release's firmware; it downloads it at start-up,
   or run `stickybot setup --with firmware`):
   ```sh
   stickybot flash --port /dev/cu.usbmodemXXXX            # add --backup to keep M5Stack's factory firmware
   ```
   Can't connect? Hold the side power button until the green LED flashes (download
   mode) and run it again. Flashing only writes the firmware parts — it never wipes
   the stick's settings area.
4. **Give it its own name** (so it isn't a second "StickyBot" in the office):
   `m5-stick-s3/tools/usb-settings.sh` opens the USB settings page (Chrome / Brave) →
   *Name*, e.g. `priya-key` (the stick restarts). Set their combo there too if their
   Wispr shortcut isn't ⌥⌘. Close the tab when done (it holds the USB port).
5. **Don't pair it with your Mac** (if you did to test, forget it in Bluetooth settings).

## Part 2 — on their Mac

### Wispr only (no software)
1. **Pair:** System Settings → Bluetooth → *priya-key* → **Connect** → press **A** on the stick when it shows the code.
2. **Wispr:** hold **A** to dictate. To use the stick's mic, plug it in by USB: it is the mic **<stick>-usb-mic** (firmware 1.7.1+; older: *TinyUSB UAC1*) — rank it in Wispr → Settings → Microphone wherever you like; Wispr only sees it while the stick is plugged in (unplugged, Wispr uses the Mac's mic). Without a cable it needs the agent: with the *Wireless Wispr mic* on, hold **A** once and drag **<stick>-bt-mic** to the **top** of that ranking — it only exists while A is held, so Wispr uses the stick then and their usual mic otherwise. On macOS 14.2+ no BlackHole is needed: macOS asks once to allow *System Audio Recording* for **StickyBot** (the app, tapping only its own sound) — tell them to press *Allow* (or turn it on later in System Settings → Privacy & Security → Screen & System Audio Recording → *System Audio Recording Only*, then *Check again* in Settings). Older macOS uses BlackHole 2ch (the setup page installs it); never pick *BlackHole 2ch* itself in Wispr. See *Wireless Wispr mic* in `mac-agent/README.md`.
3. **Unlock (optional):** set their Mac password on the USB settings page (step 4 above, on their Mac); then double-press **B** on the lock screen.

That also gives them B click = Enter and the stick's own screens, sounds and menu.

### The voice agent (hold B)
1. **Run one line in Terminal** (no GitHub account needed):
   ```sh
   curl -fsSL https://raw.githubusercontent.com/4shishkumar/stickybot/main/install.sh | sh
   ```
   It installs the app into `~/.stick-agent`, starts it, and opens the **setup page**
   in the browser (http://localhost:8660/#install).
2. **Follow the page.** One step per screen, each with a button: the speech model
   (~2.5 GB), Claude Code (with **their own** login — the page says when to run `claude`
   once in Terminal), approvals on the stick, the stick tools for Claude Code, GitHub
   alerts (log in with a one-time code), the wireless Wispr mic (macOS asks for the Mac
   password once), then **their** stick: pair, pick, name, firmware, and a first
   "what time is it". Optional steps can be skipped. Closed the tab? `stickybot
   install-page`, or the menu bar icon → *Finish setup…*.

Over SSH (or with `--no-browser`) the installer asks its questions in the terminal instead.

Each Mac's agent only talks to the stick picked in its own dashboard, so two sticks
in the office don't interfere.

## Notes
- **Wi-Fi developer mode:** both sticks would claim `stickybot.local` (the Wi-Fi name
  doesn't follow the Bluetooth name yet) — keep it off on theirs.
- **Updating later:** the agent checks for new releases every day. The dashboard shows a
  banner with an **Update** button (it updates the agent in one click and restarts it), the
  menu bar says "Update available", and the stick shows a toast once. Update the stick from
  **Stick → Firmware** (or `stickybot flash`). Re-running the installer also moves to the
  newest stable release. Want betas? Settings → Updates → Channel: Beta.
- **Removing:** `stickybot uninstall` (add `--purge` to delete `~/.stick-agent` too).
