#include "motion.h"
#include <M5Unified.h>

namespace motion {

// The gesture logic (and its tuning) is in logic/gestures.cpp, covered by the
// native unit tests; this file just feeds it the IMU readings at 50 Hz.
constexpr uint32_t SAMPLE_MS = 20;

static gestures::Detector det;
static float    a[3] = {0, 0, 1}, g[3] = {0, 0, 0};
static uint32_t lastSample = 0;
static bool     ok = false;

void begin() {
  ok = M5.Imu.isEnabled();
  det.begin(millis());
}

void accel(float& x, float& y, float& z) { x = a[0]; y = a[1]; z = a[2]; }
void gyro(float& x, float& y, float& z) { x = g[0]; y = g[1]; z = g[2]; }
int upright() { return det.upright(); }
int sideways() { return det.sideways(); }

Event poll() {
  uint32_t now = millis();
  if (!ok || now - lastSample < SAMPLE_MS) return Event::NONE;
  lastSample = now;
  if (!M5.Imu.getAccel(&a[0], &a[1], &a[2])) return Event::NONE;
  M5.Imu.getGyro(&g[0], &g[1], &g[2]);
  return det.feed(now, a, g);
}

}  // namespace motion
