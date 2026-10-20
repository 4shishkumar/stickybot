# XIAO ESP32-C3 — Bluetooth macro button

Turns a Seeed Studio XIAO ESP32-C3 into a Bluetooth keyboard with one
programmable key combo on its **BOOT** button. Default: **⌥ Option + ⌘ Command**
(e.g. for Wispr Flow). Settings are changed without reflashing — over USB or
Wi-Fi. Follows [../PROTOCOL.md](../PROTOCOL.md).

The button mirrors the keys — button down holds the combo, button up releases it:

| You do       | Mac sees                          |
|--------------|-----------------------------------|
| Single press | combo once                        |
| Double press | combo twice                       |
| Long press   | combo held until you let go       |

Also: **3 Mac slots** (pair up to 3 Macs, switch between them),
**USB settings page**, **Wi-Fi settings page** on demand, **OTA updates**.

---

## Hardware

- Seeed Studio XIAO ESP32-C3
- **Its antenna, attached** — the small sticker/rod on a cable that clips onto
  the round U.FL connector on the board. The C3 has no built-in antenna.
  Bluetooth works at desk range without it, but Wi-Fi barely reaches and the
  chip heats up. Attach it before using the Wi-Fi settings page.
- USB-C **data** cable

Buttons:

- **BOOT** (GPIO9) — the macro button.
- **RESET** — restarts the board (also used for the Wi-Fi gesture).

> Don't hold BOOT *while* pressing RESET or plugging in — that enters flashing
> (download) mode. Press RESET alone to get back to normal.

Why Bluetooth, not USB keyboard? The ESP32-C3's USB port is serial/debug only;
it can't act as a USB keyboard.

---

## 1. One-time setup: install PlatformIO

The ESP32 toolchain needs **Python 3.10–3.14** (Homebrew's `platformio` runs on
3.15 and fails), so install it in its own environment:

```sh
brew install python@3.14
python3.14 -m venv ~/.platformio/penv
~/.platformio/penv/bin/pip install platformio
```

Optional — add to `~/.zshrc` so `pio` works without the full path:

```sh
export PATH="$HOME/.platformio/penv/bin:$PATH"
```

First build downloads the toolchain and libraries (a few minutes).

---

## 2. Build and flash (USB)

From this folder:

```sh
pio run -e xiao_c3 -t upload
```

Done when you see `Hash of data verified` and `[SUCCESS]`. If the port isn't
found (`ls /dev/cu.usbmodem*` is empty): unplug, hold BOOT, plug in, release,
flash, then press RESET.

Re-flashing keeps your settings and pairings. `pio run -t erase` wipes them.

---

## 3. Pair with a Mac

1. **System Settings → Bluetooth**.
2. **Connect** next to **xiao-key** (or the name you gave it).
3. If the Keyboard Setup Assistant opens, close it.

After that it reconnects by itself whenever it's powered near that Mac.

**Upgrading from an earlier version?** Old pairings ("XIAO Macro",
"XIAO Macro 1") no longer work — forget them in Bluetooth settings and pair
**xiao-key**.

**Rename it** on the settings page (Macs → Bluetooth name) or serial
`name my-key`, 1–20 characters. The board restarts; a Mac that's already
paired may keep showing the old name until you forget and re-pair.

### Mac slots

The board remembers up to **3 Macs**, one per slot. Each slot has its own
Bluetooth identity (same name, different address), so each Mac only sees
its own slot and two Macs never fight over the board.

- **Switch slot**: settings page → Macs → **Use**, or serial `slot 2`. The
  board restarts on slot 2; pair it on the other Mac the first time.
- **Disconnect** (stay paired, go quiet) / **Reconnect**: settings page or
  serial `disconnect` / `reconnect`. A restart reconnects too.
- **Forget** a slot: settings page or serial `forget 2` (`forget all` for
  every slot). Also remove it in that Mac's Bluetooth settings.

One Mac per slot — pairing the same Mac to two slots breaks the older one.

---

## 4. Change settings — no reflashing

Everything below is stored on the board and takes effect immediately.

### a) USB settings page (no Wi-Fi needed)

```sh
tools/usb-settings.sh
```

Opens http://localhost:8642/settings.html in Chrome, Edge or Brave (Web
Serial needs a Chromium browser — not Safari/Firefox). Click **Connect via
USB**, pick the **usbmodem** port. The board restarts when the port opens;
the page fills in a few seconds later. Ctrl+C in the terminal stops the page.

Brave: if it says it can't talk to USB devices, allow serial ports at
`brave://settings/content/serialPorts`.

### b) Wi-Fi settings page (on demand)

Turn it on with the **RESET → hold BOOT** gesture:

1. Press **RESET** and let go.
2. Within 2 s, press and **hold BOOT for 3 s**, then let go.

No keys are sent to the Mac during the gesture. Then:

- **No home Wi-Fi saved:** join the hotspot **xiao-key-setup** (password
  `macro-setup` until you change it) and open **http://192.168.4.1**.
- **Home Wi-Fi saved** (on the page): the board joins it; open
  **http://xiao-key.local**. Falls back to the hotspot if it can't join in 15 s.

Wi-Fi turns off when you press RESET, click **Turn off** on the page, or after
10 min without page use. You can also switch it on/off from the USB page or
serial (`web on` / `web off`).

While Wi-Fi is on, the board uses low transmit power (it's a same-desk page)
and gives Bluetooth priority. The C3 shares one radio between the two, so the
Mac connection may still blip briefly while Wi-Fi is on.

**Change the device password** (protects the hotspot and firmware updates) on
the settings page — anyone who reads this README knows the default.

### c) Serial commands

`pio device monitor`, then:

| Command | Does |
|---|---|
| `show` | name, combo, Bluetooth, Wi-Fi, slots |
| `name my-key` | rename (Bluetooth name, restarts) |
| `set cmd+opt` | set the combo (`set ctrl+space`, `set cmd+shift+4`, `set f18`, …) |
| `slot 1`–`slot 3` | switch Mac slot (restarts) |
| `disconnect` / `reconnect` | Bluetooth off/on, staying paired |
| `forget 2` / `forget all` | forget a slot's Mac |
| `web on` / `web off` | Wi-Fi settings page |
| `help` | list commands |

Combo names: modifiers `cmd`, `opt`/`option`/`alt`, `ctrl`, `shift`; keys
`a`–`z`, `0`–`9`, `f1`–`f20`, `space`, `return`, `esc`, `tab`, `backspace`,
`delete`, arrows, `home`, `end`, `pageup`, `pagedown`, punctuation. Full list
in [../PROTOCOL.md](../PROTOCOL.md). Tip: F13–F20 make conflict-free hotkeys
(e.g. set Wispr Flow's shortcut to F18 and `set f18`).

---

## 5. Firmware updates over Wi-Fi (OTA)

With the Wi-Fi settings page on:

- **From PlatformIO:** `pio run -e xiao_c3_ota -t upload`. Uses
  `xiao-key.local`; if you changed the device password, update
  `upload_flags = --auth=…` in `platformio.ini`. If `.local` doesn't resolve,
  use the hotspot IP: `pio run -e xiao_c3_ota -t upload --upload-port 192.168.4.1`.
- **From the browser:** settings page → **Firmware update** → choose
  `.pio/build/xiao_c3/firmware.bin`, enter the device password, **Upload**.

USB flashing always works as a fallback.

---

## Serial log

`pio device monitor` shows what's happening:

```
ready xiao-key fw 1.5.0 — combo Option+Cmd. Type 'help' for commands.
Mac connected
Slot 1 paired with fc:b2:14:c3:42:e4
-> Option+Cmd down
-> Option+Cmd up
Hold BOOT for 3 s to turn on Wi-Fi settings...
Web: hotspot xiao-key-setup -> http://192.168.4.1
```

Lines starting `@` are answers to the settings page. Opening the monitor
restarts the board.

---

## Troubleshooting

| Problem | Fix |
|---|---|
| Hotspot not visible / chip hot | Attach the antenna. Phones must be close without it. |
| Mac connection blips while Wi-Fi is on | Shared radio; turn Wi-Fi off when done (RESET). |
| Gesture didn't start Wi-Fi | Let go of RESET *before* pressing BOOT, and hold BOOT a full 3 s. The log says "Released too early" if you let go sooner. |
| Board doesn't run after flashing | Press RESET (without holding BOOT). |
| USB page: no port / no answer | Close `pio device monitor` (only one app can use the port), then Connect again. |
| "Not connected" in the log when pressing BOOT | Pair "xiao-key" (step 3). |
| Upload fails / port busy | Close the monitor and the USB settings page. |

---

## Security notes

- Pairing is "Just Works" (no screen for a passkey): while the active slot
  isn't connected, a nearby computer could pair into it.
- The Wi-Fi page has no login of its own; it only runs on demand. On the
  hotspot it's protected by the device password; on home Wi-Fi anyone on that
  network can reach it while it's on. Firmware upload always needs the password.
- Home Wi-Fi and device passwords are stored unencrypted in the board's flash.

Fine for a shortcut button. Don't use this as-is to type passwords.

---

## Project layout

```
xiao-esp32-c3/
├── platformio.ini     board, partitions (2 × 1.9 MB app slots for OTA), libraries
├── src/
│   ├── main.cpp       BOOT button, Wi-Fi gesture, serial input
│   ├── config.h       names, hostname, default password, version
│   ├── combo.*        combo type, key names, "cmd+opt" parsing
│   ├── settings.*     saved settings (NVS flash)
│   ├── hid.*          Bluetooth keyboard + Mac slots
│   ├── protocol.*     command set shared by serial and the web page
│   ├── web.*          Wi-Fi, settings page server, OTA
│   └── web_page.h     generated from web/settings.html at build time
├── web/settings.html  the settings page (Wi-Fi and USB versions are the same file)
└── tools/
    ├── embed_page.py  build step: embeds settings.html into the firmware
    └── usb-settings.sh  serves the page locally for USB use
```

Built with [pioarduino](https://github.com/pioarduino/platform-espressif32)
(ESP32 Arduino core 3.x), [NimBLE-Arduino](https://github.com/h2zero/NimBLE-Arduino)
and [HijelHID_BLEKeyboard](https://github.com/HijelHub/HijelHID_BLEKeyboard).
