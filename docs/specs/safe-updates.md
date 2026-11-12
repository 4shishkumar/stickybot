# Updating a stick is safe to try

**Intent**: updating the stick's firmware can fail in any way (a dropped link, an empty battery,
a bad build) and the person still has a working stick that knows its Mac.

## What that means for a design

- **Nothing personal is ever rewritten.** Pairings, the combo, the name, sounds, faces and
  learned remotes survive every update, over Bluetooth and over USB. Only an explicit erase or a
  restore the owner asked for removes them.
- **The running firmware stays until the new one is proven.** The new image goes to the other
  slot, is checked against its sha256 after it is written, and has to run for a while with
  Bluetooth up before it is kept. If it doesn't, the stick starts the old one by itself.
- **Interruptions cost time, not the update.** A dropped link resumes where it stopped. Cancel
  works from the Mac and from the stick, and leaves everything as it was.
- **The stick decides when.** It refuses while it is busy with something the person is doing
  (dictation, a talk, an approval, a game) and when the battery is low, and says why.
- **Only what the release says.** The Mac sends an image only when it matches the release
  manifest; downgrades and repeats need to be asked for.
- **Tried on a real stick before release.** A build and a fake stick prove the logic, not the
  radio, the flash or the power. Someone updates a real stick, from the previous release, before
  a firmware release goes out.

Planned, not built: signed images (the hooks are in place).

How it works today: [docs/memory/link/firmware-update.md](../memory/link/firmware-update.md),
`PROTOCOL.md` "Firmware update over Bluetooth".
