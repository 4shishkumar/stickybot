---
type: memory
description: "Face packages: data-only faces compiled to one .sfb (64 KB, 12 per stick), installed over the firmware-update channel, versioned by format and minFirmware, listed in a CI-built, ed25519-signed catalog on GitHub Pages."
---
# Face Packages

**Domain**: faces

## Overview

A face is the stick's Home screen. Besides the built-in faces, the stick (firmware 2.4.0 and
later) draws face packages: data the dashboard installs over Bluetooth from a catalog. The format
is `docs/faces/FORMAT.md`, the catalog `docs/faces/CATALOG.md`, the rules for contributions
`docs/faces/GUIDELINES.md`; the tools are in `tools/faces/`, the packages in `faces/`.

## Requirements

### Requirement: A face is data, never code
A package is a manifest (a fixed set of element types, bindings, conditions and tweens) plus
indexed-colour PNGs, with portrait (135×240) and landscape (240×135) layouts both required. There
are no scripts or expressions. The stick always draws its status icons and urgent cards over the
face, and prompts, questions and events stay full-screen.

### Requirement: Budgets are checked before install and again on the stick
A compiled face (`.sfb`) is at most 64 KB, with at most 16 images, 96 K pixels and 48 elements per
layout; a stick holds at most 12. The validator (`tools/faces/validate.py`) enforces the budgets
and that the frame-rate tier (`static`, `calm`, `lively`) matches what moves. The stick never
trusts the Mac: it bounds-checks the file and re-checks its sha256 after writing it.

#### Scenario: An installed face fails to load
- **GIVEN** an installed face the stick cannot draw
- **WHEN** it is the Home face
- **THEN** the stick draws Classic, and `@faces` reports the face with `err`

### Requirement: Install uses the firmware update's transfer
`faces put` (id, version, size, sha256) is answered `ready`, the file travels on the OTA data
characteristic with the same windows, acks, gaps and resume, and `end` installs it atomically.
One bulk transfer runs at a time. The part file is kept, so the same `put` resumes after a
restart.

### Requirement: Compatibility is carried by two numbers
`format` is the manifest language; a stick lists the formats it reads in `pkgFaces.fmt` and
refuses others. `minFirmware` covers additions within a format. The dashboard offers Install only
for faces the connected stick can read, and "needs firmware X.Y" otherwise.

### Requirement: The catalog is built, signed and published by CI
`faces.yml` runs on pushes to `main` that touch `faces/` and nightly: it validates every package
(one invalid package fails the build), compiles each to `.sfb`, writes `catalog.json`, signs it
with ed25519 (`catalog.json.sig`; the private key is an Actions secret, the apps carry the
accepted public keys) and deploys to GitHub Pages. The app fetches the catalog (not the browser),
checks the signature and each download's sha256, and collects no install counts.

### Requirement: Ids are permanent
A face's folder is named after its id, ids are unique, and an id listed in `faces/retired.json` is
never reused. Only a face's author or a maintainer changes it, through a reviewed pull request.

## Design Decisions

### One compiled file per face
**Decision**: CI compiles a package into a single `.sfb`: the minified manifest plus images as palette indices.
**Why**: The stick never inflates a PNG or walks a folder; one file means one sha256, one transfer and one atomic rename.
**Rejected**: Sending the package folder, or PNG decoding on the stick.
*Introduced by*: predates fab; see `docs/faces/FORMAT.md` section 1

### GitHub Pages hosts the catalog
**Decision**: The catalog and compiled faces are static files on GitHub Pages.
**Why**: A stable HTTPS URL on a CDN, updated minutes after a merge, independent of app releases.
**Rejected**: raw.githubusercontent.com (rate limits, wrong content types) and release assets (re-upload churn).
*Introduced by*: predates fab; see `docs/faces/CATALOG.md` "Hosting"

## Sources

`docs/faces/FORMAT.md`, `docs/faces/CATALOG.md`, `docs/faces/GUIDELINES.md`,
`.github/workflows/faces.yml`, `tools/faces/`, `PROTOCOL.md` ("Face packages over Bluetooth").
Related: [firmware update](/link/firmware-update.md), [protocol compatibility](/link/protocol-compatibility.md).
