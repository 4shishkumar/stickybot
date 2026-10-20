#include "gestures.h"
#include <math.h>

namespace gestures {

constexpr int      LONG_AXIS        = 1;     // y runs along the stick
constexpr float    LONG_SIGN        = 1.0f;  // makes "upright" positive
constexpr float    MOVE_G           = 0.12f; // change between samples that counts as movement
constexpr uint32_t REST_BEFORE_LIFT = 3000;
constexpr float    FACE_DOWN_Z      = -0.85f;
constexpr uint32_t FACE_DOWN_MS     = 3000;
constexpr float    SHAKE_G          = 1.9f;
constexpr uint32_t SHAKE_WINDOW_MS  = 900;
constexpr uint32_t SHAKE_COOLDOWN   = 1500;
constexpr float    FLICK_DPS        = 280;
constexpr uint32_t FLICK_COOLDOWN   = 700;
constexpr float    UPRIGHT_G        = 0.55f;
constexpr uint32_t UPRIGHT_HOLD_MS  = 600;
constexpr uint32_t SIDE_HOLD_MS     = 800;

Event Detector::feed(uint32_t now, const float a[3], const float g[3]) {
  if (!primed) {  // the first sample isn't "movement"
    for (int i = 0; i < 3; i++) last[i] = a[i];
    primed = true;
  }
  float delta = fabsf(a[0] - last[0]) + fabsf(a[1] - last[1]) + fabsf(a[2] - last[2]);
  for (int i = 0; i < 3; i++) last[i] = a[i];
  float mag = sqrtf(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);

  // Orientation along the stick, debounced (auto-rotate)
  float along = a[LONG_AXIS] * LONG_SIGN;
  int cand = along > UPRIGHT_G ? 1 : along < -UPRIGHT_G ? -1 : 0;
  if (cand != orientCandidate) { orientCandidate = cand; orientSince = now; }
  if (cand && cand != orient && now - orientSince > UPRIGHT_HOLD_MS) orient = cand;

  // Lying on its side (gravity across the stick, screen vertical), debounced (dock)
  int sc = (fabsf(a[0]) > 0.75f && fabsf(a[1]) < 0.5f && fabsf(a[2]) < 0.55f) ? (a[0] > 0 ? 1 : -1) : 0;
  if (sc != sideCandidate) { sideCandidate = sc; sideSince = now; }
  if (sc != side && now - sideSince > SIDE_HOLD_MS) side = sc;

  // Shake: separate jolts above SHAKE_G inside a short window
  if (mag > SHAKE_G && !aboveShake) {
    aboveShake = true;
    peaks[peakN % SHAKE_PEAKS] = now;
    peakN++;
    if (peakN >= SHAKE_PEAKS && now - peaks[(peakN - SHAKE_PEAKS) % SHAKE_PEAKS] < SHAKE_WINDOW_MS &&
        (!lastShake || now - lastShake > SHAKE_COOLDOWN)) {
      lastShake = now;
      lastFlick = now;  // a shake isn't a flick
      peakN = 0;
      stillSince = now;
      return Event::SHAKE;
    }
  } else if (mag < SHAKE_G * 0.8f) {
    aboveShake = false;
  }

  // Flick: quick twist around the long axis (not during a shake)
  float twist = g[LONG_AXIS] * LONG_SIGN;
  if (fabsf(twist) > FLICK_DPS && (!lastFlick || now - lastFlick > FLICK_COOLDOWN) &&
      (!lastShake || now - lastShake > SHAKE_COOLDOWN)) {
    lastFlick = now;
    stillSince = now;
    return twist > 0 ? Event::FLICK_RIGHT : Event::FLICK_LEFT;
  }

  // Face-down: screen towards the table for a while
  if (a[2] < FACE_DOWN_Z) {
    if (!faceDownSince) faceDownSince = now ? now : 1;
    if (!faceDownFired && now - faceDownSince > FACE_DOWN_MS) {
      faceDownFired = true;
      return Event::FACE_DOWN;
    }
  } else {
    faceDownSince = 0;
    faceDownFired = false;
  }

  // Lift: movement after the stick had been resting
  if (delta > MOVE_G) {
    bool wasResting = now - stillSince > REST_BEFORE_LIFT;
    stillSince = now;
    if (wasResting && a[2] > FACE_DOWN_Z) return Event::LIFT;
  }
  return Event::NONE;
}

}  // namespace gestures
