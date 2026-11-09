# Faces marketplace: the plan

What exists now and what is left, in order. Efforts are rough working days for one person
who knows the codebase.

## Done (branches `feature/face-format`, then `feature/face-engine`: firmware 2.4.0)

- **The format, v1**: [FORMAT.md](FORMAT.md), [face.schema.json](face.schema.json),
  [GUIDELINES.md](GUIDELINES.md), [CATALOG.md](CATALOG.md).
- **`tools/faces/`** (standard library only):
  - `facepkg.py`: reading indexed PNGs, the schema interpreter, the semantic and budget
    checks, the `.sfb` compiler and reader;
  - `validate.py`: the command-line validator;
  - `build_catalog.py`: `catalog.json` plus compiled faces and previews;
  - `make_example_art.py`: the Cat Nap pixel art.
- **The firmware engine** (step 1 below): `m5-stick-s3/src/faces/pkg/engine.*`, with the
  pure parts in `src/logic/facefmt.*` (bindings, templates, conditions, tweens, frame tiers,
  the guard rail) and `src/logic/sfb.*` (the container), unit-tested (`make test`). face_shots
  draws packages with it (`make -C m5-stick-s3/tools/face_shots pkg PKG=…`: six states both
  ways up, a contact sheet, the previews; `fuzz PKG=… SAN=1`; `shots check <face.sfb>`). Its
  pictures of the three examples are pixel-identical to the first reference renderer's.
- **Storage and the Bluetooth install, firmware half** (steps 2 and 3): `src/faces/pkg/store.*`,
  `package.*` (the registry, the crash guard), `install.*` (the file sink, `faces` messages,
  the toast), `@faces`, `pkgFaces` in `@state`. PROTOCOL.md, *Face packages over Bluetooth*.
- **The 12 / 24-hour setting** (`clock24`): the menu, `@clock24`, `@state`, the dashboard.
- **Examples**: `faces/examples/textclock` (static, 3.6 KB), `dialclock` (static with
  seconds, 3.2 KB), `catnap` (calm, sprites, parallax, reactions, 8.8 KB).
- **Tests**: `mac-agent/tests/test_face_format.py` (examples valid, compile round trip, PNG,
  PackBits, 22 kinds of mistakes caught, catalog build).

## Remaining

### 1. Firmware engine (done, 2.4.0)

Move `pkgface.cpp` into `src/faces/pkg/` as a `Face` subclass (`PackageFace`), registered
alongside the built-ins.
- Parse once at load into compact structs in PSRAM (elements, interned binding ids, colours
  resolved to RGB565, conditions as a small tree). Today the renderer walks the ArduinoJson
  document every frame, which is fine on a Mac and wasteful on the stick.
- Image drawing: palette rows through `pushImage` with the transparent index (today it
  draws pixel by pixel). Pre-blend `mix` palettes once per frame.
- Bindings: read `clockfmt::Local` directly instead of re-parsing `Dash.longDate` (add the
  parts to `Dash`, or give the engine the `Local`).
- The frame-time guard rail and the watchdog fallback to Classic (FORMAT.md §11, §16).
- Native unit tests (`pio test -e native`) for the pure parts. Split binding formatting,
  condition evaluation and tween maths into `src/logic/facefmt.*` so `make test` covers them.
  Also fuzz the `.sfb` loader on the Mac (libFuzzer over `load()`).
- `faces::json()`, the `@face` command and demo integration: a package face in the pickers,
  `@face demo=<id>`.
- **Risks:** speed of outlined big text and full-screen layers at `lively` on the S3, so
  measure early with Cat Nap on a real stick; PSRAM fragmentation when switching faces (use
  one arena per face, freed whole); ArduinoJson memory for a 16 KB manifest (~2–3× in a
  `JsonDocument`, parsed in PSRAM).

### 2. Storage (done, 2.4.0)

LittleFS on the existing `spiffs` partition (FORMAT.md §17): mount it with format-on-fail,
`/fs/faces/`, list, remove, the 12-face and space limits, and `.part` handling. Wipe the
folder in the menu's factory reset.
- **Risks:** first-mount format time (~1–2 s for 1.5 MB, once); flash wear is negligible at
  these sizes. Check that the Arduino 3.x LittleFS partition label matches (`spiffs`).

### 3. Bluetooth install + `@faces` (firmware done in 2.4.0; the agent side remains)

Firmware: a `FileSink` for `otarx::Receiver`, the `faces` message family, ownership of the
`…0006…` characteristic, refusals, the install toast, `@faces` with paging, and
`pkgFaces` in `@state`. Agent: `stick_agent/faces.py` with the sender (reusing
`ota.Sender`'s windowing: factor it out), download, verify, install, remove and list.
Update PROTOCOL.md.
- **Risks:** sharing `…0006…` with firmware OTA (one session at a time, an explicit owner);
  keeping `@state` small (a test that asserts its size with 12 faces installed).

### 4. Dashboard catalog UI (3–4 days)

In `static/pages/stick.js`: *Get more faces* (a grid, filters, install progress, errors in
words), *Installed* (Use / Update / Remove / didn't-load), and *Install from file…* for
creators. Agent API: `/api/faces/catalog`, `/api/faces/install`, `/api/faces/remove`.
Catalog caching in `~/.stick-agent/faces/` and the config `faces_catalog_url`.
- **Risks:** stale catalog versus installed versions (compare by sha); a stick disconnected
  mid-install (resume or a clear retry).

### 5. CI catalog build + Pages (1 day)

`faces.yml`:
- On PRs touching `faces/`: run `validate.py` and `face_shots pkg` on the changed packages,
  fail if the re-rendered previews differ from the committed ones, and fail if a changed
  package didn't raise its version. Upload the contact sheet.
- On `main`: `build_catalog.py` and deploy to GitHub Pages.

Also add `faces/retired.json`, a PR template for faces, and CODEOWNERS for
`faces/community/*` (the author plus maintainers).
- **Risks:** face_shots needs PlatformIO's libdeps in CI (cache them, as the firmware job
  does); the renderer's output must be deterministic across macOS runners (it has no clock or
  randomness).

### 6. Catalog signing (1 day, can wait)

CI signs `catalog.json` (ed25519; the key lives in an Actions secret) and the agent verifies
it with a built-in public key. Plan for key rotation (a list of accepted keys).

### 7. Migrating some built-ins (2–3 days, optional)

Moving built-ins to packages proves the format and lets them be updated without firmware.
Good candidates are the simple, static ones: **Big Time**, **Words** (with `time.words*`),
**Analog**, **Dashboard**. Keep them built in (as packages embedded in flash, `.sfb` in
`PROGMEM`), so they work without an install. Classic stays C++, as the fallback.
StickyWorld, Aquarium, Space and Lava stay C++: their physics, schools of fish and city
generation are exactly what the format deliberately can't express.
- **Risks:** small visual regressions. Compare face_shots output before and after; the
  previews test already pins the pictures.

### 8. Later (not planned)

- A `temp` install for live previews on the stick.
- Per-face settings (a colour or 12/24h choice declared in the manifest).
- More bindings as the agent learns more (calendar's next meeting, PR titles).
- A web editor that writes the manifest and previews with a WASM build of face_shots.

## Total

About **15–20 days** to ship steps 1–5 (the marketplace usable end to end), plus 1 day for
signing and 2–3 days for migrating built-ins.

## Decided

1. The engine ships in firmware **2.4.0** (the examples' `minFirmware`).
2. **24-hour time**: a setting (`clock24`, 2.4.0); `time.hm` follows it, `h12` / `h24` stay
   explicit for designers. The built-in faces' clocks follow it too.
3. **Hosting**: the catalog on GitHub Pages.
4. **Card text**: community faces may show `card.a` / `card.b`.
5. **Limits**: 12 installed faces, 64 KB each.
6. **Status icons and the urgent card** are always drawn by the firmware (not overridable).

## Open

- The stick has no factory reset; when it gets one, it should wipe the faces too
  (`faces::pkg::store::wipe()`; `@faces rm=all` does it today).
- `@state`'s `faces` lists the built-in faces only (package faces are in `@faces`, to keep
  `@state` short); moving the built-in list to `@faces` too is still to do (FORMAT.md
  section 19).
