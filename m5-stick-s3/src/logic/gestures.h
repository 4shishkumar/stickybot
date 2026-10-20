// Motion gestures from accelerometer (g) + gyro (deg/s) samples, ~50 Hz.
// Pure C++ (native-testable): motion.cpp feeds it the IMU readings.
//   lift      : moved after resting >= 3 s       -> wake the screen
//   face-down : screen down (z < -0.85 g) for 3 s -> standby
//   shake     : 3 jolts > 1.9 g within 0.9 s     -> Escape / dismiss
//   flick L/R : twist > 280 deg/s around the stick -> page
//   upright / sideways: debounced orientation (auto-rotate, dock)
// Axes (StickS3): z out of the screen (face-up +1 g), y along the stick (upright +1 g).
#pragma once
#include <stdint.h>

namespace gestures {

enum class Event { NONE, LIFT, FACE_DOWN, SHAKE, FLICK_LEFT, FLICK_RIGHT };

class Detector {
 public:
  void  begin(uint32_t now) { stillSince = now; }
  Event feed(uint32_t now, const float a[3], const float g[3]);
  int   upright() const { return orient; }   // +1 upright, -1 upside down, 0 unknown/flat
  int   sideways() const { return side; }    // lying on its long edge: +1 / -1, 0 otherwise

 private:
  static constexpr int SHAKE_PEAKS = 3;
  float    last[3] = {0, 0, 1};
  uint32_t stillSince = 0, faceDownSince = 0, lastShake = 0, lastFlick = 0;
  uint32_t peaks[SHAKE_PEAKS] = {}, orientSince = 0, sideSince = 0;
  int      peakN = 0, orient = 0, orientCandidate = 0, side = 0, sideCandidate = 0;
  bool     aboveShake = false, faceDownFired = false, primed = false;
};

}  // namespace gestures
