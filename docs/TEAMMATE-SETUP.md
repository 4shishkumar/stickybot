# Setting up a teammate's stick

Two parts: **you** flash and name their stick on your Mac (about 3 minutes), then
**they** pair it — Wispr works straight away — and, if they want the voice agent
(hold B), run the installer on their Mac.

## Part 1 — on your Mac (once per stick)

1. **Plug their stick in by USB-C** (unplug yours, so you can't pick the wrong one).
2. **Find its port:** `ls /dev/cu.usbmodem*` — ignore the LG monitor ones.
3. **Flash it:**
   ```sh
   stick-agent flash --port /dev/cu.usbmodemXXXX            # add --backup to keep M5Stack's factory firmware
   ```
   Can't connect? Hold the side power button until the green LED flashes (download
   mode) and run it again. Flashing only writes the firmware parts — it never wipes
   the stick's settings area.
4. **Give it its own name** (so it isn't a second "stick-key" in the office):
   `m5-stick-s3/tools/usb-settings.sh` opens the USB settings page (Chrome / Brave) →
   *Name*, e.g. `priya-key` (the stick restarts). Set their combo there too if their
   Wispr shortcut isn't ⌥⌘. Close the tab when done (it holds the USB port).
5. **Don't pair it with your Mac** (if you did to test, forget it in Bluetooth settings).

## Part 2 — on their Mac

### Wispr only (no software)
1. **Pair:** System Settings → Bluetooth → *priya-key* → **Connect** → press **A** on the stick when it shows the code.
2. **Wispr:** hold **A** to dictate. To use the stick's mic, plug it in by USB and pick **TinyUSB UAC1** as Wispr's mic (unplugged, Wispr uses the Mac's mic). Without a cable it needs the agent: see *Wireless Wispr mic* in `mac-agent/README.md`.
3. **Unlock (optional):** set their Mac password on the USB settings page (step 4 above, on their Mac); then double-press **B** on the lock screen.

That also gives them B click = Enter and the stick's own screens, sounds and menu.

### The voice agent (hold B)
1. **Access:** add them as a collaborator on the private repo
   (github.com/ashish-kumar-noon/stick-key → Settings → Collaborators).
2. **Install** — one command (with `gh` logged in):
   ```sh
   gh api repos/ashish-kumar-noon/stick-key/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
   ```
   or clone the repo and run `./install.sh`. It installs everything into
   `~/.stick-agent` and asks about the optional parts (speech model ~2.5 GB, Claude
   Code hooks, GitHub alerts). Claude Code with **their own** login is needed for
   Claude requests (the installer points to Anthropic's installer if it's missing).
3. **Setup wizard:** the dashboard (http://localhost:8660) opens it — pick **their**
   stick, set their repo for the PR / Devin quick commands, then hold B and say
   "what time is it".

Each Mac's agent only talks to the stick picked in its own dashboard, so two sticks
in the office don't interfere.

## Notes
- **Wi-Fi developer mode:** both sticks would claim `stick-key.local` (the Wi-Fi name
  doesn't follow the Bluetooth name yet) — keep it off on theirs.
- **Updating later:** re-run the installer (it pulls the latest code), and update the
  stick from the dashboard's **Stick → Firmware** (or `stick-agent flash`).
- **Removing:** `stick-agent uninstall` (add `--purge` to delete `~/.stick-agent` too).
