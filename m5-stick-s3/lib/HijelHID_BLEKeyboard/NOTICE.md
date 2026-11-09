# Modified copy

This is HijelHID_BLEKeyboard 0.5.0 (https://github.com/HijelHub/HijelHID_BLEKeyboard,
Apache-2.0, see LICENSE), vendored for stick-key with these changes, each marked
`[stick-key change]` in the source:

- The keyboard report's reserved byte 1 is now the Apple fn / Globe key (vendor
  usage page 0xFF "AppleVendor Top Case", usage 0x03) in bit 0 + 7 padding bits.
- `setFn(bool)` holds / releases it.
- `setPnpId(source, vid, pid, version)` sets the Device Information PnP ID
  before `begin()` (macOS only honours fn from an Apple vendor ID).
- A key report waits up to 500 ms for a free Bluetooth buffer instead of ~20 ms, so
  keys aren't dropped (a typed password came out wrong) while other traffic has them.
- A mouse collection (Report ID 3: 5 buttons, X, Y, wheel, AC Pan, all relative) after
  the keyboard and consumer collections, with `mouseMove()` (never blocks; a dropped
  report returns `false`) and `mouseButtons()` (waits for a buffer like a key);
  `releaseAll()` also releases held mouse buttons.
