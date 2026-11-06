# Making and publishing a face

A face is the StickyBot's Home screen: a small manifest plus a few pixel-art images
([FORMAT.md](FORMAT.md)). Anyone can make one and submit it to the catalog, and StickyBot
owners install it from the dashboard with one click. This page covers how to make a good one,
what we accept, and how to submit and update it.

## Quick start

1. Copy an example: `cp -r faces/examples/textclock faces/community/<id>` (or `dialclock`,
   `catnap` for animation). Rename the folder and set the same `id` in `manifest.json`.
2. Edit `manifest.json`. Point your editor at `docs/faces/face.schema.json` (the examples'
   `$schema`) to get completion and checks as you type.
3. Draw images as indexed-colour PNGs in `img/` (Aseprite: *Sprite → Color Mode → Indexed*;
   Piskel and LibreSprite can export them too; or `pngquant`).
4. Check it: `python3 tools/faces/validate.py --no-previews faces/community/<id>`.
5. See it, drawn with the stick's own code and fonts, in six states (day, night, PR merged,
   CI failed, an approval waiting, no time yet), both ways up:

   ```sh
   make -C m5-stick-s3 build            # once: fetches the drawing libraries
   make -C m5-stick-s3/tools/face_shots pkg PKG=../../../faces/community/<id>
   open m5-stick-s3/tools/face_shots/out/pkg/<id>-sheet.png
   ```

6. When you're happy, write the previews into the package (`PREVIEWS=1` on that make line)
   and run the validator without `--no-previews`. Then open a pull request (see *Submitting*).

Trying it on your own stick before the catalog has it: the dashboard's *Install from file…*
(planned, PLAN.md step 4) takes a folder or `.sfb`.

## Design rules

The stick sits on a desk, about an arm's length away, on a 1.14" screen.

**Legibility**
- The time is the most important thing on the screen. It should be readable at a glance
  from 70 cm: 44 px digits or bigger in portrait, 56 px in landscape (the examples use
  44–96). Every layout must show the time; the validator checks this.
- Body text at least 12 px, small print at least 10 px, and short. Use `w` so long values
  (dates, card text, weather) are cut with "…" rather than running off the screen.
- Contrast: text against what's behind it should reach about **4.5:1** (WCAG AA). Over
  pictures and skies, use `outline`. Don't put text in mid-grey on dark grey.
- Leave the top right (about 70×18 px) quiet: the stick draws its status icons there. On a
  light background set `"status": "dark"` or `"pill"`.
- Keep the card area clear, or at least unimportant. When something is urgent, the stick
  draws its card in your layout's `card` rectangle, and `urgent.on` lets you move things out
  of its way.

**Both orientations.** Portrait (135×240) and landscape (240×135) are both required, and
both should be good, not one stretched into the other. Check both on the contact sheet.

**States.** Your face will see no time yet (just after boot), no weather, the agent offline,
low battery, night, and long values. The contact sheet shows the main ones. Use `when` with
`time.known`, `wx.valid` and `agent.linked` for graceful placeholders (the examples show the
stick's name until the time arrives).

**Battery-friendly animation**
- Pick the lowest tier that works: `static` for clocks, `calm` for gentle scenes, `lively`
  only when the face really is busy ([FORMAT.md §11](FORMAT.md#11-frame-rate-and-power)).
- Prefer animation on events (`on`, `react.*`) over constant motion: it costs nothing
  between events.
- Slow and small beats fast and big. A 1–2 px bob, slow parallax, and a twinkle every few
  seconds.
- Keep the per-frame cost down: big outlined text costs 9×, so don't outline what doesn't
  need it, and don't stack full-screen layers.

**No flashing content.** Nothing may flash more than 3 times a second, and avoid large,
high-contrast flashes altogether (photosensitive epilepsy; WCAG 2.3.1). The format enforces
part of this: colour and fade tweens are at least 500 ms. Reviewers check the rest (fast frame
animations that swap big bright areas, for example).

**Urgent information is never hidden.** You can't hide the status icons or the urgent card;
the stick draws them over your face. Don't imitate them either: no fake "Approve?" cards, no
fake low-battery icons, no fake notifications.

## Content policy

We accept faces that are:

- **Yours to share.** All art in the package is your own work, or under a licence that lets
  us redistribute it with your face (CC0, CC-BY, CC-BY-SA, MIT, Apache-2.0, BSD). Credit
  others' art in your `README.md`.
- **Licensed openly.** `license` in the manifest covers the manifest and every image, and
  must be one of `CC0-1.0`, `CC-BY-4.0`, `CC-BY-SA-4.0`, `MIT`, `Apache-2.0`, `BSD-2-Clause`,
  `BSD-3-Clause`. Code-style (MIT, Apache, BSD) and CC licences are both fine for this data. We
  don't take non-commercial or no-derivatives terms, because forks and fixes must be possible.
- **Free of marks you don't own.** No logos, trademarks, brand names used as decoration, or
  characters from games, films or comics, unless you own them or have written permission
  (say so in the PR). "Inspired by" a style is fine; copying a character is not.
- **Decent.** Nothing hateful, harassing, sexual, violent or gory, discriminatory, political
  campaigning, or targeting real people. It's a desk gadget seen by colleagues.
- **Honest.** No impersonating StickyBot's own screens (prompts, cards, updates), no
  pretending to be from another company, and no hidden messages or QR codes to elsewhere.
- **Not ads.** No advertising, sponsorship or calls to action.

## Naming

- `id`: 3–12 characters, `a-z` then `a-z0-9`, unique in the catalog and never a built-in
  face's id (`classic`, `stickyworld`, …). It is permanent: the stick saves it, and it is
  never reused after removal. Choose something descriptive (`pixelpond`, not `face1`).
- `name`: ≤ 24 characters, in title case ("Pixel Pond"). Don't use another product's name.
- `about`: one line, ≤ 80 characters, saying what you see and what it reacts to.
- `tags`: up to 5 from the usual set where they fit: `minimal`, `clock`, `analog`, `digital`,
  `animated`, `scene`, `dark`, `light`, `pixel-art`, `data`, `cute`, `retro`. Don't use
  `example`; it's ours.

## Submitting

1. Fork the repo and add your face under `faces/community/<id>/`, with `manifest.json`, `img/`,
   `previews/portrait.png` and `previews/landscape.png` from the renderer, and optionally
   `README.md` / `LICENSE`. Change nothing else.
2. Open a pull request titled `Face: <Name>`. In the description, paste the contact sheet
   (`out/pkg/<id>-sheet.png`), say where the art comes from, and tick the checklist below.
3. CI validates the package, re-renders it and builds the catalog. A maintainer reviews it.
   Merging publishes it: the catalog updates within minutes of the merge (CATALOG.md).

### Review checklist

Authors tick these; reviewers check them.

- [ ] `validate.py` passes (CI runs it) and the previews come from the renderer, unedited.
- [ ] Both orientations look intended, not squeezed.
- [ ] The time reads easily at arm's length; text contrast is about 4.5:1 or better.
- [ ] Placeholders work: no time, no weather, agent offline (see the contact sheet).
- [ ] The top right is clear for the status icons, and `status` suits the background.
- [ ] The urgent card has room and nothing important hides under it.
- [ ] Motion tier is the lowest that works; nothing flashes; reactions are short.
- [ ] All art is original or properly licensed and credited; no logos or characters you don't own.
- [ ] Content is decent and honest (no fake system screens).
- [ ] `name`, `about` and `tags` are accurate; the id is descriptive.

## Updates

- Change your face by PR, as for a new one, and **raise `version`** (semver: a fix is
  `1.0.1`, a new look `1.1.0`, a rethink `2.0.0`). CI refuses a changed package with an
  unchanged version.
- Only the author (the `author.github` in the manifest) or a maintainer may change a face.
  To hand a face over, the old author approves the PR that changes `author`.
- Users see "update available" in the dashboard and install it with one click. Nothing
  updates on a stick by itself.
- If a new version needs a newer firmware (FORMAT.md §20), raise `minFirmware`; users with
  older firmware keep the version they have and are asked to update the stick first.

## Takedown

- **You can withdraw your face** at any time by PR, or by asking in an issue. It leaves the
  catalog; sticks that have it keep it until their owner removes it (we can't and won't reach
  into people's sticks).
- **We remove faces** that break these rules, infringe someone's rights, or break on newer
  firmware and aren't fixed. Rights holders: open an issue or email the maintainers with the
  face id and what is yours. We remove first and discuss after.
- Removed ids go into `faces/retired.json` and are never reused, so an installed face can
  never silently become someone else's.
- The dashboard marks an installed face that has left the catalog as "no longer in the
  catalog", and offers to remove it.
