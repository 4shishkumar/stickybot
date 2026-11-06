# Face packages: format 1

A **face** is the StickyBot's Home screen. The built-in faces (`m5-stick-s3/src/faces/`) are
C++ and ship with the firmware. A **face package** is a face made of *data*: a JSON manifest
plus a few small images, which a renderer in the firmware draws. Anyone can make one, the
dashboard installs one onto the stick over Bluetooth in a few seconds, and the stick never
reflashes or runs anyone's code.

This document defines format 1. Related documents:

- [GUIDELINES.md](GUIDELINES.md): how to make and publish a face (design and content rules, review).
- [CATALOG.md](CATALOG.md): the catalog the dashboard browses, and how installs and updates work.
- [PLAN.md](PLAN.md): what still has to be built (the firmware engine, the install path, the UI).
- [face.schema.json](face.schema.json): the manifest's JSON Schema.
- `tools/faces/validate.py`: checks a package against the schema **and** everything below
  (references, fonts, budgets, pictures), and compiles it.
- `make -C m5-stick-s3/tools/face_shots pkg PKG=<dir>`: draws a package with the stick's own
  drawing code. This is the reference renderer.
- `faces/examples/`: three packages to copy (`textclock`, `dialclock`, `catnap`).

## Contents

1. [Design in one page](#1-design-in-one-page)
2. [Package layout](#2-package-layout)
3. [The manifest](#3-the-manifest)
4. [Screens, coordinates, layouts](#4-screens-coordinates-layouts)
5. [Elements](#5-elements)
6. [Colours](#6-colours)
7. [Data bindings](#7-data-bindings)
8. [Conditions](#8-conditions)
9. [Animation](#9-animation)
10. [Events and reactions](#10-events-and-reactions)
11. [Frame rate and power](#11-frame-rate-and-power)
12. [What the stick always shows](#12-what-the-stick-always-shows)
13. [Images](#13-images)
14. [Budgets](#14-budgets)
15. [The compiled form (.sfb)](#15-the-compiled-form-sfb)
16. [Validation and errors](#16-validation-and-errors)
17. [Storage on the stick](#17-storage-on-the-stick)
18. [Installing over Bluetooth](#18-installing-over-bluetooth)
19. [Listing faces: `@faces`](#19-listing-faces-faces)
20. [Versioning](#20-versioning)
21. [Security](#21-security)

## 1. Design in one page

| Decision | Choice | Why |
|---|---|---|
| Code or data | **Data only.** A fixed set of element types, bindings, conditions and tweens. No scripts and no expressions. | Untrusted faces can't run code. The renderer's cost is bounded and checkable before install. |
| Source images | **Indexed-colour PNG** (≤256 colours, each colour fully opaque or fully clear). | Every pixel-art tool exports these, and GitHub shows them in review. |
| What the stick gets | **One compiled file (`.sfb`)**: the minified manifest plus images as palette indices (1/2/4/8 bits, PackBits). CI builds it and the catalog serves it. | The stick never inflates a PNG or walks a folder. One file means one sha256, one transfer and one atomic rename. |
| Screens | **Portrait (135×240) and landscape (240×135) layouts, both required.** | The stick rotates itself. A face that only works one way up looks broken half the time. |
| Fonts | **Only the stick's built-in Inter sizes** (10, 12, 14, 18, 24, 32, plus 44/56/96 with digits only). | Fonts are big, and these are already in flash and tuned for the panel. |
| Safety and UX | **The stick always draws the status icons and the urgent card over the face.** Prompts, questions and events stay full-screen, as today. | No face can hide an approval, a low battery or a dropped connection. |
| Power | Three frame-rate tiers (`static`, `calm`, `lively`), the same as the built-in faces use. The validator checks that the tier matches what moves. | Battery life stays predictable. |
| Budgets | 64 KB compiled, 16 images, 96 K pixels, 48 elements per layout. | About 4 s to install over Bluetooth (~15 KB/s), at most ~100 KB of PSRAM when active, and 12 faces fit in the 1.5 MB storage partition. |
| Install | The OTA receiver (`otarx`) with a file sink, on the same Bluetooth characteristic. A new `@faces` query keeps `@state` small. | The OTA receiver already handles windows, gaps and resume and is tested. `@state` has to stay short (the 2.3.0 buffer bug). |
| Failure | Anything invalid, on the Mac or on the stick, means **Classic**. | The stick always has a working Home screen. |

## 2. Package layout

```
faces/community/<id>/
  manifest.json            the face (section 3)
  img/<name>.png           its images: indexed PNG, lower-case names [a-z0-9_-], ≤ 16
  previews/portrait.png    135 x 240, made by the reference renderer
  previews/landscape.png   240 x 135
  README.md                optional: what it shows, credits
  LICENSE                  optional (the manifest's `license` is what counts)
```

Nothing else is allowed, because the validator refuses unexpected files. The folder is named
after the id. Packages live in this repo under `faces/community/` (submitted by pull request)
and `faces/examples/` (ours). The total size on disk must stay at or under 256 KB.

## 3. The manifest

```jsonc
{
  "$schema": "../../../docs/faces/face.schema.json",   // optional, for editors
  "format": 1,                       // this document
  "id": "catnap",                    // see below
  "name": "Cat Nap",                 // ≤ 24 characters, shown in the pickers
  "version": "1.0.0",                // semver, numbers only; raise it with every change
  "author": {"name": "Ada", "github": "ada", "url": "https://…"},   // name required
  "license": "CC-BY-4.0",            // CC0-1.0 | CC-BY-4.0 | CC-BY-SA-4.0 | MIT | Apache-2.0 | BSD-2/3-Clause
  "about": "A cat on a hill …",     // ≤ 80 characters, one line
  "tags": ["animated", "cat"],       // ≤ 5, [a-z0-9-]
  "minFirmware": "2.4.0",            // the first firmware with the engine (2.4.0 is the plan)
  "motion": "calm",                  // static | calm | lively (section 11), default static
  "colors": {"sky": "#6cb8ef"},      // ≤ 16 named colours (section 6)
  "images": {"cat": {"src": "img/cat.png", "frameW": 32}},   // ≤ 16 (section 13)
  "parts": {"cat": [ … elements … ]},                        // ≤ 8 reusable element lists
  "reactions": {"merged": 4000},     // how long react.<event> stays true, ms (section 10)
  "layouts": {"portrait": { … }, "landscape": { … }}         // both required (section 4)
}
```

**Id.** The rule is `^[a-z][a-z0-9]{2,11}$`: 3 to 12 characters, starting with a letter. This
matches the built-in faces' rule (`[a-z0-9]`, at most 12, checked in
`mac-agent/tests/test_faces.py`). The id is saved in NVS (the `face` setting), typed in
`@face id=…` and in the serial `screens faces` command, and used as a file name on the stick.

Namespacing: ids are flat on purpose. An `author/name` form doesn't fit 12 characters, the
NVS key or the serial commands. The catalog is the namespace instead:

- An id is unique across the built-in faces, a reserved list (`classic`, `stickyworld`, …,
  `all`, `none`, `demo`, `default`, `off`, `face`, `faces`, `stickybot`) and the whole catalog.
  First come, first served, enforced by CI.
- An id is **never reused**, even after a face is taken down (`faces/retired.json`).
- The pickers show `name` and the author (`by @github`), so two similar names can't be
  confused. Built-in faces can never be shadowed.

## 4. Screens, coordinates, layouts

The panel is 135×240. Held sideways it is 240×135, and the stick turns its picture to match
(`ui::setOrientation`). A face has one **layout** for each:

```jsonc
"portrait": {
  "bg": "#101418" | "name" | {"grad": ["top", "bottom"], "dir": "v" | "h"},   // default: the stick's ground
  "status": "light" | "dark" | "pill",   // the status icons' ink (section 12), default light
  "card": [x, y, w, h],                  // where the urgent card goes (section 12)
  "elements": [ … ]                      // ≤ 48, drawn in order
}
```

- **Coordinates** are whole pixels in that layout's screen. The origin is the top left and y
  points down. Values may be off-screen (−240 to 480), and anything off-screen is clipped.
- **Draw order is z-order.** Elements are drawn in list order, so later ones are on top.
  There is no `z` field: reorder the list instead.
- **Anchors.** For a box-shaped element (`rect`, `bar`, `text`, `image`, `weather`), `x`, `y`
  is the point on the box named by `anchor`: `tl tc tr ml mc mr bl bc br`
  (top/middle/bottom × left/centre/right). The default is `tl`. Text uses the same anchors as
  its datum, with the font's box. Round elements (`circle`, `arc`, `hand`, `ticks`) use
  `x`, `y` as their centre. `line` runs from `x`, `y` to `x2`, `y2`. `scatter` fills the box
  `x`, `y`, `w`, `h`.
- **Both layouts are required.** There is no auto-derivation: the shapes are too different
  (a 240-tall column versus a 135-tall strip) for scaling to look right. Reuse goes through
  **parts**: a `group` with `"part": "cat"` draws the part's elements offset by the group's
  `x`, `y`, so one character can sit in both layouts.

## 5. Elements

Every element has a `type`. It may also have:

| Field | Meaning |
|---|---|
| `x`, `y` | Position (section 4). Either a number or a **numeric binding** `{"var": "motion.tiltX", "mul": -3, "add": 0, "lo": …, "hi": …}`, which gives var × mul + add, clamped. |
| `when` | A condition (section 8). The element is drawn only while it holds. |
| `anim` | Up to 4 tweens (section 9). |
| `repeat` | `{"count": 3 \| "work.mergedToday", "max": 6, "dx": 15, "dy": 0}`: draw the element `count` times (at most 16, or `max`), each copy offset by `dx`, `dy`. A bound count needs `max`. |
| `mix`, `mixTo` | Blend all of the element's colours this far (0..1) toward `mixTo` (default: the layout's background). A cheap "dimmed" or "at night" variant. |
| `name` | A label for humans. It is ignored. |

| Type | Fields (required in **bold**) | Draws |
|---|---|---|
| `rect` | **`w`, `h`**, `r` (corner radius), `fill`, `stroke`, `anchor` | A (rounded) rectangle. `fill` is anti-aliased when `r` > 0. |
| `line` | **`x2`, `y2`, `color`**, `width` (0.5–8) | An anti-aliased line. |
| `circle` | **`r`**, `fill`, `stroke` | An anti-aliased disc and/or ring. |
| `arc` | **`r`, `t`** (thickness), **`color`**, `start`, `end` (degrees clockwise from 12 o'clock, default 0..360), `value`, `track` | A ring segment. With `value` (a constant 0..1 or `{"var": "dev.battery", "of": 100}`) it fills that fraction from `start` toward `end`. `track` draws the whole segment underneath. This is the gauge. |
| `bar` | **`w`, `h`, `value`, `color`**, `track`, `r`, `dir` (`right` \| `up`) | A progress bar. |
| `text` | **`text`, `color`**, `font` *or* `fit` + `w`, `w`, `lines` (1–3), `case` (`upper` \| `lower`), `outline`, `shadow`, `anchor` | A template (section 7) in a built-in font. `fit: [96, 56]` picks the first font in which the text fits in `w` (list them biggest first). With `w` alone, long text is cut with "…". With `lines`, it wraps within `w` (the last line is cut). `outline` draws a 1-px rim (2 px at 56/96) plus a drop shadow, as the built-in scene faces do. `shadow` draws a 1-px drop shadow. |
| `image` | **`image`**, `frame` (number or numeric binding), `play`, `scale` (1–4), `tile` (`x`), `flip` (`x`), `anchor` | A frame of an image (section 13). `play: {"fps": 4, "seq": [0, 0, 1, 2]}` flips through frames (fps ≤ 12; without `seq` it uses all frames in order). `tile: "x"` repeats it across the screen: animate `x` for a scrolling parallax layer. Nearest-neighbour `scale` suits pixel art. |
| `hand` | **`hand`** (`hour` \| `minute` \| `second`), **`length`, `color`**, `tail`, `w0`, `w1` (width at the centre and at the tip), `shadow` | A clock hand at the current time (`logic/clockface.cpp`: smooth hour and minute hands, ticking seconds). Before the time is known the hands rest at 10:10 and the second hand is hidden. |
| `ticks` | **`r`, `count`, `color`**, `len`, `width`, `every`, `majorLen`, `majorWidth`, `majorColor`, `firstColor` | Dial marks: `count` around a circle of radius `r`, every `every`-th one a major mark, the 12 o'clock one in `firstColor`. One element instead of 60. |
| `weather` | `size` (12–48), `anchor` | The stick's own weather icon for the current conditions (clear, partly, fog, rain, snow, storm; day or night). Hidden while there's no weather. |
| `scatter` | **`w`, `h`, `n`** (≤ 48), **`color`**, `seed`, `size` (1–3), `twinkle` (ms), `fall`, `drift` (px/s) | `n` dots at fixed pseudo-random places in the box: stars (`twinkle`), rain or snow (`fall`), dust (`drift`). Moving dots wrap within the box. |
| `group` | `part` *or* `children` (≤ 16) | Its elements, offset by the group's `x`, `y` (plus its tweens and repeat). Groups don't nest. |

Unknown fields and unknown types are errors (`additionalProperties: false`). A face made for
a newer format is refused, not half-drawn.

## 6. Colours

A colour is `"#rrggbb"` or the name of one of the manifest's `colors` (at most 16 names,
`[a-z][a-z0-9_]*`). The stick works in RGB565, so close colours may merge. There is no alpha:
use `mix` / `mixTo` to blend toward a colour, and use images for soft edges. Image palettes
are per image (section 13).

## 7. Data bindings

A text template is literal text with `{binding}` or `{binding:fmt}` in it. `fmt` is `02`,
`03` or `04` (pad with zeros) or `upper` / `lower`. Bindings are **read-only** and come from a
**fixed list**: a face can show your data on your own stick, but it has no way to send
anything anywhere (section 21). An unknown value (no time yet, no weather, the agent not
connected) is an empty string or 0 and counts as false. Use `when` to show a placeholder
(the examples show the stick's name until the time arrives).

| Binding | Kind | Example | Where it comes from today (firmware) |
|---|---|---|---|
| `time.known` | bool | | `Dash.minute >= 0` (the agent has sent the clock) |
| `time.hm` | text | `9:41` | `Dash.clock` (`buddy::clock()`, 12-hour. If a 24-hour setting arrives, `time.hm` follows it) |
| `time.hm24` | text | `21:41` | `Dash.minute` |
| `time.h12`, `time.hh12` | text | `9`, `09` | `Dash.minute` |
| `time.h24`, `time.hh24` | text | `21`, `21` | `Dash.minute` |
| `time.mm`, `time.ss` | text | `41`, `07` | `Dash.minute`, `Dash.second` |
| `time.ampm` | text | `AM` | `Dash.ampm` |
| `time.hour`, `time.minute`, `time.second`, `time.minuteOfDay` | number | | `Dash.minute`, `Dash.second` |
| `time.words`, `time.wordsLead`, `time.wordsHour` | text | `twenty to ten`, `just after`, `ten` | `clockface::words()` (the Words face) |
| `date.weekday`, `date.dow` | text | `Friday`, `Fri` | `clockfmt::Local.wday` (`Dash.longDate`) |
| `date.day`, `date.dd` | text | `6`, `06` | `Local.mday` |
| `date.month`, `date.mon`, `date.mm` | text | `November`, `Nov`, `11` | `Local.mon` |
| `date.short`, `date.long` | text | `Fri 6 Nov`, `Friday 6 November` | `Dash.date`, `Dash.longDate` |
| `wx.valid` | bool | | `Dash.wxValid` (the agent's `wx`) |
| `wx.temp`, `wx.hi`, `wx.lo` | number | `24` | `Dash.wxTemp` / `wxHi` / `wxLo` (°, the agent's unit) |
| `wx.icon`, `wx.cond`, `wx.day` | text, text, bool | `rain`, `light rain` | `Dash.wxIcon` / `wxCond` / `wxDay` |
| `dev.battery` | number | `76` (unknown: not known) | `Dash.battery` |
| `dev.charging` | bool | | `Dash.charging` |
| `dev.bt` | text | `connected` \| `waiting` \| `off` | `Dash.bt` |
| `dev.name` | text | `StickyBot` | `Dash.name` |
| `agent.linked` | bool | | `Dash.agentLinked` (heard from the Mac agent in the last 25 s) |
| `card.kind`, `card.a`, `card.b`, `card.urgent` | text ×3, bool | `fact`, `Did you know?` | `Dash.cardKind` / `cardA` / `cardB` / `cardUrgent` (the agent's `home` card) |
| `urgent.on` | bool | | `faces::urgent(d).on`: the urgent card is showing (section 12) |
| `work.known` | bool | | `agentLinked && world.valid` |
| `work.asks`, `work.reviews`, `work.sessions` | number | | `world::Snapshot.asks` / `reviews` / `nSessions` (the agent's `world`) |
| `work.ci` | text | `green` \| `red` \| `none` | `Snapshot.ci` |
| `work.merged`, `work.mergedToday` | number | | `Snapshot.merged` / `mergedToday` |
| `buddy.total`, `buddy.running`, `buddy.waiting`, `buddy.tokens` | number | | `Dash.sessions` / `running` / `waiting` / `tokens` (the Claude desktop app) |
| `pet.level`, `pet.into`, `pet.need` | number | `5`, `20`, `60` | `pet::levelFor(Snapshot.xp)` |
| `pet.xp`, `pet.streak`, `pet.greens` | number | | `Snapshot.xp` / `streak` / `greens` |
| `pet.hat`, `pet.shades`, `pet.crown` | bool | | `Snapshot.acc` bits |
| `sky.night` | bool | | `faces::isNightMinute()` (22:00–06:00) |
| `sky.day` | number | 0–99 | how far through the day (`minute × 100 / 1440`) |
| `motion.tiltX`, `motion.tiltY` | number | −1..1 | the accelerometer (`ui::gravity`): tilt parallax |
| `react.<event>` | bool | | true for `reactions[event]` ms after the event (section 10) |

The digit-only fonts (44, 56, 96) contain `0-9 : - – ° space` (96: `0-9 : space`). The
validator refuses literal letters and bindings that can contain letters (`time.ampm`, a
`date.*` name, …) in those fonts. A signed number (`wx.temp`) is fine in 44 and 56 but not
in 96.

## 8. Conditions

`when` is one of:

- `"sky.night"`: the binding is truthy (known and non-zero or non-empty). `"!sky.night"` is the negation.
- `{"var": "work.ci", "eq": "red"}`: one comparison, `eq ne gt ge lt le`. `gt` and friends need a number.
- `{"all": [ … ]}`, `{"any": [ … ]}` (≤ 8 each), `{"not": … }`. At most 3 deep.

That is the whole language. There is no arithmetic and no string building beyond templates.

## 9. Animation

A **tween** changes one property over time:

```jsonc
{"prop": "y", "from": 0, "to": -16, "ms": 280, "ease": "out", "loop": "pingpong",
 "delay": 0, "phase": 0, "on": "merged", "times": 3}
```

| Field | |
|---|---|
| `prop` | `x`, `y`: an offset added to the position (pixels). `mix`: 0..1 toward `mixTo` (fades). `color`: the element's main colour, from → to (colours). `frame`: an image frame. |
| `from`, `to` | Numbers, or colours for `color`. `from` defaults to 0 (for `color`, to the element's colour). |
| `ms` | One cycle, 50–120000. |
| `ease` | `linear` (default), `in`, `out`, `inout`, `sine`, `back` (overshoot). |
| `loop` | `repeat` (default), `pingpong` (there and back is two `ms`), `once` (then holds `to`). |
| `delay` | Before it starts. |
| `phase` | Extra delay per `repeat` copy (`phase × i`), so copies don't move in step (the three z's). |
| `on` | Start on an event (section 10) instead of running all the time. It plays `times` cycles (default 1), then stops and adds nothing. Before the event, it isn't there. |

Several `x` / `y` tweens on one element add up. For `mix`, `color` and `frame`, the last one wins.

**Sprites and frame animation:** an `image` with `frameW` has frames side by side. `play`
loops them at `fps`, and `frame` can be bound to data. **Parallax:** an `image` with
`tile: "x"` and a slow linear `x` tween from 0 to −(its width) scrolls forever. Layers at
different speeds (clouds, hills), plus `x: {"var": "motion.tiltX", "mul": -3}`, give depth.
**Bobbing:** `y` with `pingpong` and `sine`.

**Not allowed:** colour (`mix` or `color`) tweens shorter than 500 ms. With `pingpong` that
is at most one flash per second (GUIDELINES.md: no flashing). There are no rotations or
free scaling, because they're too costly per frame on the stick. Use frames.

## 10. Events and reactions

Events are the stick's existing reactions (`world::Kick`, `faces::onEventKind`), plus a few
the engine derives itself:

| Event | When |
|---|---|
| `merged` | A PR of yours merged (world `merged` grew, or the event `pr_merged`) |
| `ci_red`, `ci_green` | CI went red, or back to green |
| `claude_start`, `claude_done` | A Claude Code session started or finished |
| `release` | A release was published |
| `level_up` | StickyBot's level went up |
| `review` | Someone asked for your review |
| `shake`, `flick`, `upside_down`, `lift` | The toys (motion sensor) |
| `minute`, `hour` | The minute or hour changed |
| `dusk`, `dawn` | `sky.night` became true or false |
| `urgent` | The urgent card appeared |

The firmware already debounces these (the same reaction twice within 45 s plays once; toys
need 1.5 s between them). A face reacts in two ways:

- **`react.<event>`** is true for `reactions[event]` ms (500–15000, default 3000) after the
  event. Use it in `when` to swap a sprite or show a bubble.
- **Tweens with `"on": "<event>"`** play once from the event: a jump, a shake, a fish
  dropping in.

`catnap` does both. On `merged` the happy frame shows, jumps three times, a fish drops in
and a "+1 PR" bubble shows for 4 s. On `ci_red` the bristling frame shakes with a red
"CI failed". At night (`sky.night`) the cat sleeps with floating z's.

## 11. Frame rate and power

`motion` picks the tier. The numbers are the ones the built-in faces use
(`Face::frameMs`):

| `motion` | On USB | On battery | On battery, idle > 20 s | For |
|---|---|---|---|---|
| `static` (default) | redraw on change | redraw on change | redraw on change | Clocks. With seconds (`time.ss`, `time.second`, a second hand): every 200 ms on USB, 1 s on battery, as Analog does. |
| `calm` | 100 ms | 200 ms | 500 ms | Slow scenes: bobbing, twinkling, slow parallax |
| `lively` | 40 ms | 66 ms | 150 ms | Busy scenes (StickyWorld's rates) |

- The validator checks the tier: a `static` face must not have anything that moves all the
  time (a tween without `on`, `play`, a moving or twinkling `scatter`, a tilt binding), and
  a `calm` or `lively` face must have something.
- A `static` face that reacts to an event runs at `calm` while the reaction or its `on`
  tweens play, then goes back to static.
- When the screen sleeps, nothing is drawn, as today.
- **Guard rail:** the engine times every frame. If frames take more than 30 ms for 2 s, the
  face drops a tier, and below `static` it stops animating (it keeps drawing on change).
  Logged as `Faces: <id> too slow, tier lowered`.

## 12. What the stick always shows

A package face draws the screen, and then the **firmware** draws over it, every frame:

1. **The status icons** at the top right (battery %, battery, Bluetooth, the agent's dot),
   about 70×18 px. `status` picks the ink: `light` (default), `dark` for light
   backgrounds, or `pill` (light ink on a small dark pill, readable on anything). Leave the
   top right quiet.
2. **The urgent card** (`faces::urgent()`): Bluetooth waiting or off, the battery at 15 %
   or below, or the agent's urgent card (an approval or a question waiting). It is the
   built-in faces' sign board, in the layout's `card` rectangle. The defaults are
   `[6, 184, 123, 48]` portrait and `[10, 84, 220, 44]` landscape, and the minimums are
   110×38 and 96×34 (the text must fit). `urgent.on` lets a face move things out of the
   way (Text Clock shrinks its time).
3. **Everything that isn't Home stays as it is:** approval prompts, questions, events, the
   menu, dictation, the dock clock and updates take over the whole screen exactly as they do
   over the built-in faces.

A face can't cover these: they're drawn after it, by code the face doesn't control. The
validator also requires every layout to show the time (a text with a `time.*` binding, or a
`hand`), so the stick never becomes a picture frame.

## 13. Images

- **Indexed-colour PNG** (colour type 3, bit depth 1–8, not interlaced), at most 256
  colours. Transparency comes only from the palette (`tRNS`): each colour is fully opaque or
  fully clear, with no partial alpha. Draw at 1× and use `scale`.
- At most 240×240. Sprite sheets have frames of `frameW` px side by side (all the same
  width, the image's full height), at most 32 frames of at most 128×128.
- **Why indexed PNG in, palette indices out:** the stick has no PNG decoder in the face
  path. LovyanGFX's pngle would need ~40 KB to inflate and gives 24-bit pixels to convert.
  Palette images are what pixel art is anyway. They're 4–8× smaller than RGB565, and they
  make `mix` cheap: the engine blends the ≤256 palette entries, not every pixel. CI
  compiles the PNGs into the `.sfb` (section 15).
- In memory each pixel is one byte (its index) in PSRAM, plus the palette in RGB565. A
  frame is drawn row by row through its palette (on the stick: `pushImage` with a palette
  and the clear index as the transparent colour).

## 14. Budgets

| Limit | Value | Derived from |
|---|---|---|
| Compiled face (`.sfb`) | **64 KB** | ~15 KB/s over Bluetooth (the OTA path) → at most ~4.5 s to install |
| Manifest, minified | 16 KB | The ArduinoJson document in PSRAM |
| Images | 16 per face | |
| Image size | ≤ 240×240. Frames ≤ 128×128, ≤ 32 per sheet | One screen. Sprites stay small |
| Pixels, all images | **96 K** (98,304) | One byte each when decoded → ≤ 96 KB of PSRAM (8 MB on board) while active |
| Elements per layout | **48**, counting group and part children | |
| Things drawn per layout | 96, counting `repeat` copies | |
| Children per group or part | 16. Parts: 8. Groups don't nest | |
| Tweens per element | 4 | |
| `repeat` | 16 copies | |
| `scatter` | 48 dots | |
| Condition depth | 3, with ≤ 8 terms per `all` / `any` | |
| Text template | 64 characters, ≤ 3 lines | |
| Pixels touched per frame (estimate) | **6 screens** (194,400 px) | Checked for day and night. Outlined text counts 9× because it is drawn 9 times. The guard rail (section 11) catches the rest |
| Colour tween | ≥ 500 ms | No flashing |
| Image animation | ≤ 12 fps | |
| Package folder in the repo | 256 KB | Reviewable PRs |
| Installed faces on a stick | **12** | 12 × 64 KB = 768 KB of the 1.5 MB partition (section 17) |

The examples use 3.6 KB (Text Clock), 3.2 KB (Dial Clock) and 8.8 KB (Cat Nap, with 4
images and 6 cat frames). The limits are generous for this style of face.

## 15. The compiled form (.sfb)

`tools/faces/facepkg.py compile_face()` turns a package into one little-endian file:

```
offset  size
0       4     "SFB1"
4       2     container version (1)
6       2     face format (1)
8       4     manifest length M
12      2     image count N (≤ 16)
14      2     0
16      M     the manifest, minified UTF-8 JSON, "$schema" dropped and each
              images[name] replaced by {"i": index, "frameW": …}; zero-padded to 4
then N times:
        2     width            2  height          2  frameW (0: one frame)
        1     bits per pixel (1, 2, 4, 8)          1  colours − 1
        1     the clear index (255: none)          1  encoding (0 raw, 1 PackBits)
        2     0                4  data length D
        2×C   the palette, RGB565, zero-padded to 4
        D     the indices, rows padded to a byte (MSB first), maybe PackBits; padded to 4
```

The reference renderer (`m5-stick-s3/tools/face_shots/pkgface.cpp`) reads exactly this,
which is what the firmware engine will read. `facepkg.read_sfb()` is the Python reader (the
agent checks downloads with it).

## 16. Validation and errors

**Before install, on the Mac.** `tools/faces/validate.py` runs the schema
(`face.schema.json`, through a small built-in interpreter, so there is no `jsonschema`
dependency) and then the checks a schema can't express: every image, part and colour
reference exists; every binding is known; fonts can draw their text; frames exist; the
budgets (section 14) hold; the tier matches what moves; each layout shows the time; the
card fits; there are no unused images or parts; there are no stray files; the previews have
the right size. CI runs it on every package in a PR. The dashboard runs the same checks
(`read_sfb` plus the budgets) on every download before sending it.

**On the stick.** The stick checks everything again and never trusts the Mac: the header and
every size against the file, palette indices, the manifest parsed with a nesting limit, and
every reference resolved once at load. Unknown types or fields, or a value out of range, make
the face invalid. An invalid face is **never half-drawn**:

- The stick shows **Classic**, logs `Faces: <id> invalid: <why>`, and lists the face with
  `"err": "<why>"` in `@faces`. The dashboard shows "this face didn't load on your stick,
  reinstall or remove it".
- The saved `face` setting is kept, so a fixed reinstall brings the face back. Removing the
  face selects Classic.
- A face that crashes or hangs the renderer (it shouldn't, but it's a new parser) trips the
  loop watchdog. After 2 resets in a row while a package face was active, the stick boots
  with Classic and marks that face invalid (an NVS counter, cleared after 60 s of normal
  running).

## 17. Storage on the stick

The partition table (`default_8MB.csv`, `board_build.partitions` in `platformio.ini`)
already has a **1.5 MB data partition** that the firmware doesn't use today:

```
nvs       0x9000    20 KB      settings
otadata   0xE000     8 KB
app0      0x10000    3.2 MB    firmware (ota_0)
app1      0x340000   3.2 MB    firmware (ota_1)
spiffs    0x670000   1.5 MB    <- faces
coredump  0x7F0000    64 KB
```

- Mount it as **LittleFS** (Arduino-ESP32 3.x `LittleFS.begin(true, "/fs", 5, "spiffs")`,
  formatting it on first use). LittleFS survives power loss, has directories, and its
  `rename` is atomic. SPIFFS has neither and is deprecated.
- **No partition change is needed**, so faces arrive with an ordinary firmware update,
  including the Bluetooth OTA, which never touches the partition table. Sticks that had
  anything in that partition (nothing we ship writes it) get it formatted.
- Layout: `/fs/faces/<id>.sfb`, and while receiving `/fs/faces/.part` (with `.part.json`:
  id, size, sha256, so a transfer can resume after a reboot). That's all. The installed list
  is the directory, and each face's metadata is in its header and manifest.
- At most **12** faces and 64 KB each: 768 KB at most, leaving LittleFS the free space it
  needs. Install refuses with `full` past either limit.
- The active face's id stays in NVS (`face`, as today). When a package face is chosen, the
  engine loads its file into PSRAM: the manifest (ArduinoJson, parsed once into compact
  structs) and the images (indices, one byte each). Other faces stay on flash.
- **Removing** deletes the file. If it was the active face, the stick switches to Classic.
  A factory reset (menu) or `@faces rm=all` wipes the folder.

## 18. Installing over Bluetooth

The same link and the same receiver as firmware updates (PROTOCOL.md, *Firmware update over
Bluetooth*): `otarx::Receiver` with a **file sink** instead of the flash sink. Bytes go over
the existing OTA data characteristic (`…0006…`, `[offset u32 LE][bytes]`, write without
response, encrypted link). Only one bulk transfer runs at a time, so the open session (a
firmware or a face) owns `…0006…`. Messages are JSON on RX/TX with `"src":"agent"`:

| Direction | Message |
|---|---|
| Mac → stick | `{"faces":{"put":{"id":"catnap","version":"1.0.0","size":8772,"sha256":"<64 hex>"}}}` |
| stick → Mac | `{"evt":"faces","state":"ready","id":"catnap","offset":0,"window":16384,"chunk":240,"resumed":false}` |
| stick → Mac | `{"evt":"faces","state":"refused","error":"busy"\|"full"\|"size"\|"format"\|"firmware"\|"ota","why":…}` |
| Mac → stick | packets on `…0006…` |
| stick → Mac | `{"evt":"faces","state":"ack","offset":N}` and `"gap":true` exactly as for firmware (4 KB acks, resend from N) |
| Mac → stick | `{"faces":{"end":1}}` |
| stick → Mac | `{"evt":"faces","state":"done","id":"catnap","version":"1.0.0"}`: the sha256 of the file read back matches, the face validated (section 16), and `.part` was renamed to `<id>.sfb` (replacing the old version atomically) |
| stick → Mac | `{"evt":"faces","state":"error","error":"sha"\|"invalid","why":"image 2: bad header"}` |
| Mac → stick | `{"faces":{"cancel":1}}` → `"state":"cancelled"` |
| Mac → stick | `{"faces":{"rm":"catnap"}}` → `{"evt":"faces","state":"removed","id":"catnap"}` |
| Mac → stick | `@face id=catnap` (unchanged): pick it |

- **Refusals:** `busy` (dictation, talk, an approval, the menu: the same checks as OTA),
  `ota` (a firmware update is running or pending), `size` (0 or > 64 KB), `full` (12 faces
  or not enough space), `format` (a face format this firmware doesn't know), `firmware`
  (the agent should already have checked `minFirmware`, and the stick checks again from the
  manifest). There is no battery minimum: it's a 64 KB file write, not a reflash.
- **Resume:** as OTA. The session lives 3 min after a disconnect, and the same `put`
  (id + size + sha256) resumes from the acknowledged offset. `.part.json` lets it survive a
  reboot too.
- **Checksum:** sha256 end to end. The catalog publishes it (CATALOG.md), the agent checks
  the download against it, and the stick checks what it wrote (read back from flash, as
  OTA does). Signatures: see section 21.
- **Time:** 64 KB at ~15 KB/s is ~4.5 s. The examples take under a second.
- **On screen:** a small "Installing Cat Nap… 60 %" toast over Home (not the full update
  screen). B cancels.

## 19. Listing faces: `@faces`

`@state` is answered in ≤180-byte notifications, and in 2.3.0 a long `@state` (the built-in
faces' list, ~1.1 KB) used up all of NimBLE's buffers. The installed package faces must not
make `@state` grow. So:

- `@state` gains only `"pkgFaces":{"n":3,"fmt":[1]}`: the count and the formats the stick
  supports (~30 bytes). `"face":"catnap"` already names the active face.
- A new **`@faces`** query (protocol action `faces`) returns them, one page at a time:

```json
{"ok":true,"faces":[{"id":"catnap","v":"1.0.0","sz":8772,"sha":"3f9c0a1b"},
                    {"id":"dialclock","v":"1.0.0","sz":3224,"sha":"7d10e2c4","err":"image 0: bad header"}],
 "free":712,"max":12,"more":false}
```

  Each entry is ~55 bytes: id, version, size, the first 8 hex digits of the sha256 (enough
  to tell "installed = catalog version"), and `err` only if it failed to load. `@faces page=1`
  gives the next 6 if `more`. Names and titles stay on the Mac: the catalog has them.

- Later, the built-in list in `@state` (`faces`) should move to `@faces` (`"builtin":[…]`)
  too, keeping `@state` small for good. Agents check for `pkgFaces` before using `@faces`.

## 20. Versioning

Three numbers:

- **`format`** (this document: 1) is the manifest language. It changes only for a breaking
  change: a field that means something else, or a removed type. A stick lists the formats it
  reads (`pkgFaces.fmt`) and refuses others. The catalog lists each face's format, and the
  dashboard only offers faces the stick can read.
- **`minFirmware`** handles additions within a format. New element types, bindings, events
  or fields are *added* in a firmware release, and a face that uses them sets `minFirmware`
  to that release. Older firmware would refuse the unknown field (section 5) rather than
  draw it wrong, and the dashboard says "needs firmware 2.6 — update the stick" instead of
  installing. The validator will know which firmware introduced each addition (a table in
  `facepkg.py`) and require `minFirmware` to be at least that.
- **`version`** is the face's own version (semver). The catalog serves the latest, and the
  dashboard offers an update when `@faces` reports an older version (or a different sha).
- The `.sfb` container has its own version (bytes 4–5) so the packing can change without a
  format change.

## 21. Security

- **No code.** A face is drawn by a fixed interpreter with a fixed set of operations. There
  are no loops beyond bounded `repeat` and `scatter`, and no recursion beyond 3-deep
  conditions and one level of groups. Every size is capped before allocation.
- **No way out.** Bindings are read-only, and nothing a face does reaches the network, the
  Mac, the keyboard (HID) or the settings. The worst a hostile face can show is your own data
  on your own stick, which any face could show anyway.
- **Parser hardening** is the main risk, as with any file format. The stick bounds-checks
  everything, uses ArduinoJson with a nesting limit, and keeps the watchdog fallback
  (section 16). The C++ renderer runs on the Mac against every catalog face in CI
  (face_shots), so it is also a fuzzing target (PLAN.md).
- **Integrity:** HTTPS from GitHub Pages, the sha256 from the catalog checked by the agent,
  and sha256 re-checked on the stick. Signing the catalog (ed25519, a key held by CI) is
  planned (CATALOG.md) so that a compromised Pages deploy can't push a face.
- **Content** (logos, offensive images) is a human matter: PR review (GUIDELINES.md).
