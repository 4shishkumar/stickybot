// Which way the stick is held, from accelerometer (g) + gyro (deg/s) samples at ~50 Hz,
// and which screen rotation to use for it. Pure C++ (native-testable).
//
// Axes (StickS3): z out of the screen (face-up +1 g), y along the stick (upright +1 g),
// x across it, towards the right edge of the portrait screen (x = +1 g with that edge up).
//
//   PORTRAIT          y up       rotation 0, button A below the screen
//   PORTRAIT_FLIPPED  y down     rotation 2, button A above the screen
//   LANDSCAPE_RIGHT   x up       rotation 1, button A to the right of the screen
//   LANDSCAPE_LEFT    x down     rotation 3, button A to the left of the screen
//
// Flat (face up / down) or moving: keeps the last one. A new way up has to be clearly
// past the 45° line (hysteresis) and held still for DWELL_MS before it counts.
#pragma once
#include <stdint.h>

namespace orientation {

enum class Orient : uint8_t { PORTRAIT, PORTRAIT_FLIPPED, LANDSCAPE_RIGHT, LANDSCAPE_LEFT };

constexpr uint32_t DWELL_MS     = 250;    // a new way up must hold this long
constexpr float    FLAT_G       = 0.35f;  // less gravity than this in the screen's plane = lying flat (~20° of tilt)
constexpr float    HYSTERESIS   = 10.0f;  // degrees past the 45° line before leaving the current one
constexpr float    MOVING_DPS   = 120.0f; // turning faster than this = still moving
constexpr float    MOVING_G     = 0.25f;  // |a| this far from 1 g = being shaken / swung

class Tracker {
 public:
  explicit Tracker(Orient start = Orient::PORTRAIT) { reset(start); }
  void   reset(Orient o);
  // Returns the (debounced) orientation after this sample.
  Orient feed(uint32_t now, const float a[3], const float g[3]);
  Orient current() const { return cur; }
  Orient lastPortrait() const { return portrait; }  // the last upright / upside-down one

 private:
  Orient   cur = Orient::PORTRAIT, portrait = Orient::PORTRAIT, cand = Orient::PORTRAIT;
  bool     haveCand = false;
  uint32_t candSince = 0;
};

// The raw reading of one sample (no debounce): false when flat or past no clear line.
// `from` is the current orientation (it gets the hysteresis margin).
bool classify(const float a[3], Orient from, Orient& out);

// What the screens should use. side: 0 portrait (flipped or not), +1 rotation 1, -1 rotation 3.
// Portrait-only screens use `flipped` even while side != 0.
struct Screen {
  bool   flipped;
  int8_t side;
  bool operator==(const Screen& o) const { return flipped == o.flipped && side == o.side; }
  bool operator!=(const Screen& o) const { return !(*this == o); }
};

// autoRotate off: today's manual flip, always portrait. On: follow the stick; landscape
// screens only when `landscape` is on (otherwise the last portrait way up).
Screen choose(const Tracker& t, bool autoRotate, bool landscape, bool flippedSetting);

// Button A's side of the screen for a landscape `side` (+1 rotation 1: right, -1: left).
inline bool buttonAOnRight(int side) { return side > 0; }

}  // namespace orientation
