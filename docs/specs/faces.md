# Faces anyone can make, that can't hurt a stick

**Intent**: people share Home faces for the stick the way they share wallpapers, and installing
one from a stranger is safe.

## What that means for a design

- **Data, not code.** A face describes what to draw from a fixed set of parts. It can read the
  stick's data to show it; it cannot send anything, press keys, change settings or run logic of
  its own.
- **The stick stays in charge of the screen.** Status icons, approvals, questions and alerts are
  always drawn by the stick over or instead of the face. A broken face means the Classic face,
  never a blank screen.
- **Costs are known before install.** Size, images, pixels, elements and frame rate have limits
  a tool can check on the Mac, so install time, memory and battery use are predictable. The stick
  checks again and trusts nothing it is sent.
- **A person looks at every face.** Faces enter and change through a reviewed pull request. CI
  builds the catalog from what was merged, signs it, and the app checks the signature and every
  download.
- **No telemetry.** The catalog is static files; nothing counts installs or reports what a
  person uses.
- **Faces outlive firmware.** A face says which format and which firmware it needs, and the
  dashboard offers only what the connected stick can draw.

Planned work for faces is in [docs/faces/PLAN.md](../faces/PLAN.md).

How it works today: [docs/memory/faces/face-packages.md](../memory/faces/face-packages.md),
`docs/faces/FORMAT.md`, `docs/faces/CATALOG.md`.
