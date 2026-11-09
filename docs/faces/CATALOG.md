# The faces catalog

The catalog is the list of community and example faces the dashboard offers. It is built
by CI from the packages in this repo, published as static files, and moderated by pull
request review. See [FORMAT.md](FORMAT.md) for the face format and
[GUIDELINES.md](GUIDELINES.md) for how faces get in.

## Where faces live

```
faces/
  examples/<id>/      ours: textclock, dialclock, catnap (channel "examples")
  community/<id>/     everyone's, by pull request (channel "community")
  retired.json        ids that were removed: never reused
```

**Moderation is PR review.** A face appears in the catalog when a maintainer merges its PR.
CI (`validate.py`, the reference renderer, the catalog build) catches what a machine can
catch; a person checks the rest against the
[review checklist](GUIDELINES.md#review-checklist). Nothing goes live without a merge, and
nothing changes after a merge without another one.

## Building it

`tools/faces/build_catalog.py --out site/faces --base https://<pages>/faces/` (CI runs it on
every push to `main` that touches `faces/`):

- validates every package (all the checks, previews included); **one invalid package fails
  the build**, so the published catalog is always all-valid;
- checks that ids are unique, folders are named after their ids, and no retired id is reused;
- compiles each face to `<id>/<id>-<version>.sfb` and copies its previews;
- writes `catalog.json`.

In PRs, CI also re-renders each changed face with face_shots and attaches the contact sheet
as an artifact; the reviewer compares it with the committed previews (re-rendered previews
must match; a diff fails the check: planned, PLAN.md).

### catalog.json

```json
{
 "catalog": 1,
 "generated": "2026-11-06T12:00:00Z",
 "formats": [1],
 "faces": [
  {
   "id": "catnap",
   "name": "Cat Nap",
   "version": "1.0.0",
   "about": "A cat on a hill: cheers when a PR merges, bristles when CI fails, naps at night",
   "author": {"name": "StickyBot", "github": "ashish-kumar-noon"},
   "license": "CC0-1.0",
   "tags": ["animated", "cat", "example"],
   "channel": "examples",
   "format": 1,
   "minFirmware": "2.4.0",
   "motion": "calm",
   "sfb": "https://ashish-kumar-noon.github.io/stickybot/faces/catnap/catnap-1.0.0.sfb",
   "size": 8772,
   "sha256": "…64 hex…",
   "previews": {"portrait": "https://…/faces/catnap/portrait.png",
                "landscape": "https://…/faces/catnap/landscape.png"},
   "source": "https://github.com/ashish-kumar-noon/stickybot/tree/main/faces/examples/catnap",
   "updated": "2026-11-06"
  }
 ],
 "retired": []
}
```

| Field | |
|---|---|
| `catalog` | Version of this index schema (1). A dashboard that doesn't know it says "update StickyBot to browse faces". |
| `formats` | Face formats present. |
| `faces[]` | One entry per face, latest version only. Copied from the manifest, plus: `channel`, `sfb` (the compiled file), `size`, `sha256` (of the `.sfb`), `previews`, `source`, `updated` (the package's last commit date). |
| `retired` | Ids removed from the catalog (see Takedown in GUIDELINES.md). |

About 450 bytes per face: 200 faces is ~90 KB, fine for one file. If it ever outgrows that,
split it into pages (`catalog-2.json`), keeping `catalog.json` the entry point.

**Installs count?** Not collected. GitHub Pages gives us no per-file numbers and we don't
want telemetry from users' Macs. "Popular" could later come from GitHub reactions on a
per-face discussion; not in v1.

## Hosting: GitHub Pages (recommended)

| Option | For | Against |
|---|---|---|
| **GitHub Pages** (`actions/deploy-pages` from the catalog build) | A stable URL (`https://ashish-kumar-noon.github.io/stickybot/faces/catalog.json`); served by a CDN with HTTPS; correct content types and CORS; updates minutes after a merge; independent of app releases; free for public repos | One more workflow and a Pages setting to turn on |
| raw.githubusercontent.com (a generated `catalog` branch) | No Pages setup | Rate-limited, ~5 min caching, `text/plain` for everything, not meant as a CDN; a generated branch to keep |
| A release asset (`faces-catalog` release, re-uploaded) | Download counts per asset | Release noise; files must be re-uploaded every time; URLs of `latest` redirect; awkward for many small files |

**Recommendation: GitHub Pages**, deployed by a `faces.yml` workflow on pushes to `main` that
touch `faces/` (and nightly, to pick up retirements). The dashboard has the URL built in
(config `faces_catalog_url`, overridable for testing or a fork).

## The dashboard

The Stick page's Face card grows a **Get more faces** section (and the agent an
`/api/faces/*` API: PLAN.md step 4).

**Fetch and cache.** The agent (not the browser) fetches `catalog.json` with `If-None-Match`
/ `If-Modified-Since`, at most every 6 hours or when the user opens the section, and keeps it
in `~/.stick-agent/faces/catalog.json` so browsing works offline with the last copy. Previews
and `.sfb` files are cached under `~/.stick-agent/faces/<id>/` by version.

**Verify.** Every `.sfb` download is checked before it goes anywhere: its sha256 equals the
catalog's; `facepkg.read_sfb()` parses it; the budgets hold (size, images, pixels); its
`id`/`version` match the entry. A mismatch discards the file and shows "download damaged, try
again". (Planned: the catalog carries an ed25519 signature made by CI, checked against a key
built into the agent: `catalog.json.sig`. Until then integrity rests on HTTPS + GitHub.)

**Show.** A grid like the built-in faces' tiles: portrait and landscape previews, name,
"by @author", the about line, tags, motion as a small hint (Static / Calm / Lively: "uses a
little more battery"), size, and a filter for tags and channel. Faces the stick can't take
(`format` not in its `pkgFaces.fmt`, or `minFirmware` above its firmware) show "needs firmware
X.Y" with the update button instead of Install.

**Install.** *Install* → the agent downloads and verifies → sends it with `faces put`
([FORMAT.md §18](FORMAT.md#18-installing-over-bluetooth)) with a progress bar (a few seconds)
→ on `done`, asks "Use it now?" (`@face id=…`). Errors are shown in words: `full` → "Your
stick has 12 faces; remove one", `busy` → "The stick is busy (dictation); try again", and so
on. The stick must be connected; there's no queue in v1.

**Installed.** From `@faces`: each installed face with its catalog entry (matched by id),
*Use*, *Remove* (`faces rm`), and **Update** when the catalog's version is newer (or the sha
differs from `sha` in `@faces`). An installed face no longer in the catalog shows "no longer
in the catalog" with Remove. A face with `err` shows "didn't load on your stick" with
Reinstall / Remove.

**Preview without installing.** The catalog previews are pictures; a live preview on the
stick needs the face installed (a later step could add `faces put` with `"temp":true`, kept in
PSRAM only and shown via the existing `@face demo=`).
