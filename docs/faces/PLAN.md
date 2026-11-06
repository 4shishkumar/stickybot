# Faces marketplace: the plan

What exists now (branch `feature/face-format`) and what is left, in order. Efforts are rough
working days for one person who knows the codebase.

## Done (this branch)

- **The format, v1**: [FORMAT.md](FORMAT.md), [face.schema.json](face.schema.json),
  [GUIDELINES.md](GUIDELINES.md), [CATALOG.md](CATALOG.md).
- **`tools/faces/`** (standard library only):
  - `facepkg.py`: reading indexed PNGs, the schema interpreter, the semantic and budget
    checks, the `.sfb` compiler and reader;
  - `validate.py`: the command-line validator;
  - `build_catalog.py`: `catalog.json` plus compiled faces and previews;
  - `make_example_art.py`: the Cat Nap pixel art.
- **Reference renderer**: `m5-stick-s3/tools/face_shots/pkgface.{h,cpp}`. It reads `.sfb`
  and draws with the stick's own `ui_draw.h` / `faces/common.h` (fonts, fitting, icons,
  status, urgent card). It handles every element type, bindings, conditions, tweens, events
  and reactions, repeat, parts, tiles and mix. `make -C m5-stick-s3/tools/face_shots pkg
  PKG=…` renders six states in both orientations, plus a contact sheet and the previews.
- **Examples**: `faces/examples/textclock` (static, 3.6 KB), `dialclock` (static with
  seconds, 3.2 KB), `catnap` (calm, sprites, parallax, reactions, 8.8 KB).
- **Tests**: `mac-agent/tests/test_face_format.py` (examples valid, compile round trip, PNG,
  PackBits, 22 kinds of mistakes caught, catalog build).

## Remaining

### 1. Firmware engine (5–7 days)

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

### 2. Storage (1–2 days)

LittleFS on the existing `spiffs` partition (FORMAT.md §17): mount it with format-on-fail,
`/fs/faces/`, list, remove, the 12-face and space limits, and `.part` handling. Wipe the
folder in the menu's factory reset.
- **Risks:** first-mount format time (~1–2 s for 1.5 MB, once); flash wear is negligible at
  these sizes. Check that the Arduino 3.x LittleFS partition label matches (`spiffs`).

### 3. Bluetooth install + `@faces` (2–3 days)

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

## Open questions

1. **`minFirmware` "2.4.0"** in the examples assumes the engine ships in 2.4.0.
2. **24-hour time**: there is no setting yet. Should `time.hm` follow a future setting, with
   `h12` / `h24` explicit for designers (as written)?
3. **Hosting**: GitHub Pages (recommended) needs Pages turned on for the public repo. Is
   that OK?
4. **Card text in faces**: faces can show `card.a` / `card.b` (Claude progress, PR titles)
   on screen. It's the user's own screen and faces can't send anything anywhere, but should
   community faces be limited to the urgent card only?
5. **Install limit** of 12 faces and **64 KB** each: generous or tight?
6. Should the status icons be overridable for minimal faces (BigTime-style "battery as a
   hairline")? The design says no: the firmware always draws them.
