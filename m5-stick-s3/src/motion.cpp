#include "motion.h"
#include <M5Unified.h>

namespace motion {

// Axes (measured on the StickS3): z points out of the screen (screen-up = +1 g,
// face-down = -1 g); y runs along the stick (upright = +1 g).
// LONG_AXIS runs along the stick; LONG_SIGN makes "upright" positive.
constexpr int   LONG_AXIS = 1;     // 0 = x, 1 = y, 2 = z
constexpr float LONG_SIGN = 1.0f;

constexpr uint32_t SAMPLE_MS        = 20;    // 50 Hz
constexpr float    MOVE_G           = 0.12f; // change between samples that counts as movement
constexpr uint32_t REST_BEFORE_LIFT = 3000;  // must have been still this long before a lift counts
constexpr float    FACE_DOWN_Z      = -0.85f;
constexpr uint32_t FACE_DOWN_MS     = 3000;
constexpr float    SHAKE_G          = 1.9f;  // jolt magnitude
constexpr int      SHAKE_PEAKS      = 3;
constexpr uint32_t SHAKE_WINDOW_MS  = 900;
constexpr uint32_t SHAKE_COOLDOWN   = 1500;
constexpr float    FLICK_DPS        = 280;   // twist speed around the long axis
constexpr uint32_t FLICK_COOLDOWN   = 700;
constexpr float    UPRIGHT_G        = 0.55f; // gravity along the stick needed to call it up/down
constexpr uint32_t UPRIGHT_HOLD_MS  = 600;

static float    a[3] = {0, 0, 1}, g[3] = {0, 0, 0}, last[3] = {0, 0, 1};
static uint32_t lastSample = 0, stillSince = 0, faceDownSince = 0, lastShake = 0, lastFlick = 0;
static uint32_t peaks[SHAKE_PEAKS], orientSince = 0;
static int      peakN = 0, orient = 0, orientCandidate = 0;
static int      side = 0, sideCandidate = 0;
static uint32_t sideSince = 0;
static bool     aboveShake = false, faceDownFired = false, ok = false;

void begin() {
  ok = M5.Imu.isEnabled();
  stillSince = millis();
}

void accel(float& x, float& y, float& z) { x = a[0]; y = a[1]; z = a[2]; }
void gyro(float& x, float& y, float& z) { x = g[0]; y = g[1]; z = g[2]; }
int upright() { return orient; }
int sideways() { return side; }

Event poll() {
  uint32_t now = millis();
  if (!ok || now - lastSample < SAMPLE_MS) return Event::NONE;
  lastSample = now;
  if (!M5.Imu.getAccel(&a[0], &a[1], &a[2])) return Event::NONE;
  M5.Imu.getGyro(&g[0], &g[1], &g[2]);

  float delta = fabsf(a[0] - last[0]) + fabsf(a[1] - last[1]) + fabsf(a[2] - last[2]);
  for (int i = 0; i < 3; i++) last[i] = a[i];
  float mag = sqrtf(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);

  // Orientation along the stick, debounced (for auto-rotate).
  float along = a[LONG_AXIS] * LONG_SIGN;
  int cand = along > UPRIGHT_G ? 1 : along < -UPRIGHT_G ? -1 : 0;
  if (cand != orientCandidate) { orientCandidate = cand; orientSince = now; }
  if (cand && cand != orient && now - orientSince > UPRIGHT_HOLD_MS) orient = cand;

  // Lying on its side (gravity across the stick, screen vertical), debounced: the dock's landscape clock.
  int sc = (fabsf(a[0]) > 0.75f && fabsf(a[1]) < 0.5f && fabsf(a[2]) < 0.55f) ? (a[0] > 0 ? 1 : -1) : 0;
  if (sc != sideCandidate) { sideCandidate = sc; sideSince = now; }
  if (sc != side && now - sideSince > 800) side = sc;

  // Shake: separate jolts above SHAKE_G inside a short window.
  if (mag > SHAKE_G && !aboveShake) {
    aboveShake = true;
    peaks[peakN % SHAKE_PEAKS] = now;
    peakN++;
    if (peakN >= SHAKE_PEAKS && now - peaks[(peakN - SHAKE_PEAKS) % SHAKE_PEAKS] < SHAKE_WINDOW_MS &&
        now - lastShake > SHAKE_COOLDOWN) {
      lastShake = now;
      lastFlick = now;  // a shake isn't a flick
      peakN = 0;
      stillSince = now;
      return Event::SHAKE;
    }
  } else if (mag < SHAKE_G * 0.8f) {
    aboveShake = false;
  }

  // Flick: quick twist around the long axis (not during a shake).
  float twist = g[LONG_AXIS] * LONG_SIGN;
  if (fabsf(twist) > FLICK_DPS && now - lastFlick > FLICK_COOLDOWN && now - lastShake > SHAKE_COOLDOWN) {
    lastFlick = now;
    stillSince = now;
    return twist > 0 ? Event::FLICK_RIGHT : Event::FLICK_LEFT;
  }

  // Face-down: screen towards the table for a while.
  if (a[2] < FACE_DOWN_Z) {
    if (!faceDownSince) faceDownSince = now;
    if (!faceDownFired && now - faceDownSince > FACE_DOWN_MS) {
      faceDownFired = true;
      return Event::FACE_DOWN;
    }
  } else {
    faceDownSince = 0;
    faceDownFired = false;
  }

  // Lift: movement after the stick had been resting.
  if (delta > MOVE_G) {
    bool wasResting = now - stillSince > REST_BEFORE_LIFT;
    stillSince = now;
    if (wasResting && a[2] > FACE_DOWN_Z) return Event::LIFT;
  }
  return Event::NONE;
}

}  // namespace motion
