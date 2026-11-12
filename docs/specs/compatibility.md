# Old and new work together

**Intent**: a person updates the Mac app and the stick whenever each is convenient, in any
order, and StickyBot keeps working. Nobody is asked to update two things at once.

## What that means for a design

- **Additions only.** A new feature is a new message or a new field. An existing field never
  changes meaning; an unknown message or field is ignored without an error on screen.
- **Say what you can do.** The stick reports features in `@state` (`ota`, `pkgFaces`, `ir`, …);
  the app reports them in its hello (`wmic`, `wready`, `speak`, …). Each side uses a feature only
  after the other said it has it.
- **Fall back in words.** When the other side is too old, the old behaviour stays (the first 300
  characters of a reply instead of the whole reply) or the dashboard says what is needed ("needs
  firmware 2.5: update the stick") with the way to get it.
- **The first update is always possible.** A stick too old for Bluetooth updates can be updated
  over USB; USB stays the recovery path for every version.
- **Files others read grow by new keys.** Release manifests, the faces catalog and saved settings
  gain keys; readers skip what they don't know. A breaking change gets a new version number of
  its own (`catalog`, a face's `format`), and old readers say "update StickyBot" instead of
  guessing.

## Every design answers

1. How does each side find out the other has this?
2. What does a stick one release older do when it gets it? What does an app one release older do?
3. What does the person see when one side is too old?
4. Which lines of `PROTOCOL.md` change?

How it works today: [docs/memory/link/protocol-compatibility.md](../memory/link/protocol-compatibility.md),
`PROTOCOL.md`.
