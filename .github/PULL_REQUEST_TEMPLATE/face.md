<!-- A new or changed face. Title: "Face: <Name>". Guidelines: docs/faces/GUIDELINES.md.
     (Open this template with ?template=face.md on the "Open a pull request" URL.) -->

## The face

- **Id / name:** `<id>` / <Name>
- **New face or update:** new | update from <old version> to <new version>
- **Where the art comes from:** <drawn by me / CC0 from … / licensed from … (link)>

## Contact sheet

<!-- Drag in out/pkg/<id>-sheet.png from:
     make -C m5-stick-s3/tools/face_shots pkg PKG=../../../faces/community/<id> PREVIEWS=1
     (CI renders it again and attaches it to the checks.) -->

## Checklist

- [ ] The package is only `faces/community/<id>/` (plus one line for it in `.github/CODEOWNERS`); nothing else changed.
- [ ] `validate.py` passes (CI runs it) and the previews come from the renderer, unedited.
- [ ] An update raises `version` (and `minFirmware`, if it needs a newer stick).
- [ ] Both orientations look intended, not squeezed.
- [ ] The time reads easily at arm's length; text contrast is about 4.5:1 or better.
- [ ] Placeholders work: no time, no weather, agent offline (see the contact sheet).
- [ ] The top right is clear for the status icons, and `status` suits the background.
- [ ] The urgent card has room and nothing important hides under it.
- [ ] Motion tier is the lowest that works; nothing flashes; reactions are short.
- [ ] All art is original or properly licensed and credited; no logos or characters you don't own.
- [ ] Content is decent and honest (no fake system screens).
- [ ] `name`, `about` and `tags` are accurate; the id is descriptive.
