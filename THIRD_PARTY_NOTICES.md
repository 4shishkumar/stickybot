# Third-party notices

The project's own code is MIT-licensed (see [LICENSE](LICENSE)). It includes or
builds in work by others under these licences.

## Included in this repository
| What | Where | Licence |
|---|---|---|
| **Inter** font (Rasmus Andersson) — and the fonts generated from it | `m5-stick-s3/tools/fonts/Inter.ttf`, `m5-stick-s3/src/fonts_vlw.cpp` | SIL Open Font License 1.1 — [OFL.txt](m5-stick-s3/tools/fonts/OFL.txt) |
| **Kenney** sound packs (Interface Sounds, UI Audio, Music Jingles) — the clips built into the firmware | `m5-stick-s3/src/sounds_clips.cpp` (packs are downloaded by `tools/make_sounds.py`, not committed) | CC0 1.0 (public domain) — [kenney.nl](https://kenney.nl). The celebration clips next to them (applause, cheer, ta-da…) are synthesized by `tools/synth_sounds.py`, no third-party audio; each clip's source: [m5-stick-s3/SOUNDS.md](m5-stick-s3/SOUNDS.md) |

## Built into the stick firmware (and the bundled `mac-agent/stick_agent/firmware/*.bin`)
| Library | Licence |
|---|---|
| [M5Unified](https://github.com/m5stack/M5Unified), [M5GFX](https://github.com/m5stack/M5GFX), [M5PM1](https://github.com/m5stack/M5PM1) | MIT |
| [NimBLE-Arduino](https://github.com/h2zero/NimBLE-Arduino) | Apache-2.0 |
| [HijelHID_BLEKeyboard](https://github.com/HijelHub/HijelHID_BLEKeyboard) 0.5.0 | Apache-2.0 — vendored in `m5-stick-s3/lib/` and modified (Apple fn key bit, settable PnP ID; changes marked `[stick-key change]`) |
| USBAudioCard from the [Arduino core for ESP32](https://github.com/espressif/arduino-esp32) 3.3.12 | Apache-2.0 — vendored in `m5-stick-s3/lib/StickUSBAudio/` as `StickUSBAudio` and modified (renamed; settable microphone name "stick-mic-usb" instead of "TinyUSB UAC1"; changes marked `[stick-key change]`) |
| [ArduinoJson](https://github.com/bblanchon/ArduinoJson) | MIT |
| [Arduino core for ESP32](https://github.com/espressif/arduino-esp32) (via pioarduino) | LGPL-2.1 (core) / Apache-2.0 (ESP-IDF, TinyUSB parts MIT) |

## Used by the Mac agent (installed from PyPI, not included)
bleak (MIT), aiohttp (Apache-2.0), rumps (BSD-3-Clause), numpy (BSD-3-Clause),
parakeet-mlx (Apache-2.0), MLX (MIT), soundfile (BSD-3-Clause), soxr (LGPL-2.1),
pyserial (BSD-3-Clause), esptool (GPL-2.0 — run as a separate tool).

Downloaded at run time: the Parakeet speech models (NVIDIA, CC-BY-4.0), weather from
Open-Meteo (CC-BY-4.0 data), Hacker News, Wikipedia (CC-BY-SA text) and
uselessfacts content shown on the stick.
