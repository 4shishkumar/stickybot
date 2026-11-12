# Code Quality

How code is written in this repo. The reviewer's side is in `code-review.md`; the rules that
never bend are in `constitution.md`.

## Principles

- **Match the code around you.** Each part has its own settled style (below); a change should
  read as if the same person wrote the file.
- **Keep logic away from hardware.** Decide in pure code (firmware `src/logic/`, Rust modules
  behind traits, plain JS modules), act in a thin layer. If a function needs a stick, Bluetooth,
  an audio device, the network or Claude to be tested, split it.
- **Comments say why, in plain words.** What the code is for, what it guards against, what
  happened last time ("With 12, a long reply filled them all and key presses were dropped").
  Units and reasons next to constants. No comments that repeat the code, no banners.
- **Names are the plain word for the thing**, and wire names are what goes on the wire
  (`otarx::Error` names are the protocol's strings). Use the words the docs use: stick, Mac link,
  hello, talk, face, slot.
- **Protocol first.** A message change starts in `PROTOCOL.md`, including how the other side
  detects it and what older firmware / older apps do.
- **Detect, don't assume.** Use a feature only when the other side reports it (`@state` keys,
  hello flags); otherwise fall back or say what is needed, in words a user understands.
- **Bounded everything on the stick.** Fixed-size buffers, capped counts, paged replies; check
  sizes before allocating; anything from the Mac or a file is untrusted.
- **Errors reach the person as words.** `busy` becomes "The stick is busy (dictation); try
  again", not a code. Text on the stick is short and uses only the characters its fonts have.
- **One side effect per statement in C++.** gcc and clang order argument evaluation differently.
- **Small, single-topic changes.** One topic per PR; the PR title is the commit on `main`.

### Per part

| Part | Style | Lint / format |
|---|---|---|
| `m5-stick-s3/` | C++17, 2-space indent, one `namespace` per module, `constexpr` constants with unit comments | builds with `-Wall -Wextra` in the native env; no formatter |
| `desktop/` | Rust 2024; `//!` module docs that say what it ports and why; traits + fakes for the OS and the stick | `cargo clippy --workspace --all-targets -- -D warnings`; `rustfmt.toml` (width 120) |
| `mac-agent/` | Python 3.11+, line length 120; existing files keep their hand-aligned style | `ruff check` (E, W, F, I, B, UP, SIM); `make format FILES=…` only for new files |
| `mac-agent/stick_agent/static/` | vanilla JS modules, no framework, no build step; API calls resolve with `{error}` instead of throwing | covered by `node --test` suites |
| `tools/`, `install.sh` | POSIX `sh`; standard-library Python where possible | shellcheck; ruff with the agent's config |

## Anti-Patterns

- Logic inside hardware-facing code (`ui.cpp`, `buddy.cpp`, `corebt.rs`, `system.rs`) that could
  have been a pure function with a test.
- A message or field added on one side only, or without its `PROTOCOL.md` entry.
- Sending a new message to a stick (or app) that did not say it supports it.
- Growing `@state` or any single reply instead of adding a paged query.
- Two calls with side effects (random draws, reads, `i++`) in one C++ expression.
- Editing generated files by hand (`fonts_vlw.cpp`, `sounds_clips.cpp`, `web_page.h`, face
  previews, `Cargo.lock` by hand).
- Real-looking keys, tokens, names, emails or private repo names in code, tests, fixtures or log
  lines. Build key-shaped test values from pieces.
- Tests that touch the real home folder, Bluetooth, audio devices, the Keychain, GitHub or the
  network. Use the fakes and a temporary home.
- Bumping versions by hand (`tools/release.sh` does it; `FW_VERSION` only when the firmware changes).
- Writing anywhere near the stick's settings storage, or building a single merged flash image.
- A new dependency, font, sound or image without its licence in `THIRD_PARTY_NOTICES.md`.
- Blocking the firmware's main loop (long waits, big synchronous writes): the watchdog and the
  rollback timer are running.
- Magic numbers without a name and a unit; timeouts without the reason for their value.
- Changelog lines written for developers ("refactor X") instead of for the person using it.

## Test Strategy

test-alongside

New logic arrives with its tests, in the same change. What to run, by the part touched:

| Touched | Run | Where |
|---|---|---|
| `m5-stick-s3/` | `pio test -e native` then `pio run -e sticks3` (check the `Flash:` line stays ≤ 85 %) | `m5-stick-s3/` |
| `desktop/` | `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` | `desktop/` |
| `mac-agent/`, `tools/` | `python -m ruff check .` and `python -m pytest` (runs the JS tests too when Node is installed) | `mac-agent/` |
| `mac-agent/stick_agent/static/` or `dashboard.html` | the `mac-agent/` commands **and** the `desktop/` ones (the Rust app serves these files) | both |
| `faces/`, `tools/faces/` | `python tools/faces/validate.py <package>`; `python -m pytest tests/test_face_*.py` | root; `mac-agent/` |
| `xiao-esp32-c3/` | `pio run -e xiao_c3` | `xiao-esp32-c3/` |
| everything Python + firmware at once | `make check` | repo root |

- Firmware tests are Unity suites in `m5-stick-s3/test/test_<area>/`, compiled for the host. They
  must pass with gcc as well as clang; CI uses gcc.
- Rust tests run against the fake stick (`stick-link/src/fake.rs`) and the in-memory Core Audio
  (`macos-audio/src/fake.rs`). Opt-in real-hardware tests stay opt-in.
- Python tests use the `home`, `agent`, `FakeLink`, `FakeSTT` fixtures; mark anything needing the
  speech model or hardware `@pytest.mark.slow`.
- A passing suite is not a hardware check. Firmware and device-facing changes also need a try on
  a real stick, by a person, before release; say in the PR what was tried.
