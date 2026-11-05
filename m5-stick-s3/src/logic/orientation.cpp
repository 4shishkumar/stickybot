#include "orientation.h"
#include <math.h>

namespace orientation {

// Where gravity points in the screen's plane for each way up: atan2(x, y) in degrees.
static float centre(Orient o) {
  switch (o) {
    case Orient::LANDSCAPE_RIGHT:  return 90;
    case Orient::PORTRAIT_FLIPPED: return 180;
    case Orient::LANDSCAPE_LEFT:   return -90;
    default:                       return 0;
  }
}

static float apart(float a, float b) {  // 0..180 degrees between two angles
  float d = fmodf(fabsf(a - b), 360.0f);
  return d > 180 ? 360 - d : d;
}

static bool isPortrait(Orient o) { return o == Orient::PORTRAIT || o == Orient::PORTRAIT_FLIPPED; }

bool classify(const float a[3], Orient from, Orient& out) {
  if (sqrtf(a[0] * a[0] + a[1] * a[1]) < FLAT_G) return false;  // lying flat: no opinion
  float ang = atan2f(a[0], a[1]) * 180.0f / (float)M_PI;
  if (apart(ang, centre(from)) <= 45 + HYSTERESIS) { out = from; return true; }
  static const Orient ALL[] = {Orient::PORTRAIT, Orient::LANDSCAPE_RIGHT, Orient::PORTRAIT_FLIPPED,
                               Orient::LANDSCAPE_LEFT};
  for (Orient o : ALL)
    if (apart(ang, centre(o)) <= 45) { out = o; return true; }
  return false;
}

void Tracker::reset(Orient o) {
  cur = cand = o;
  if (isPortrait(o)) portrait = o;
  haveCand = false;
}

Orient Tracker::feed(uint32_t now, const float a[3], const float g[3]) {
  float mag = sqrtf(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);
  float spin = fmaxf(fabsf(g[0]), fmaxf(fabsf(g[1]), fabsf(g[2])));
  Orient o;
  if (spin > MOVING_DPS || fabsf(mag - 1) > MOVING_G || !classify(a, cur, o) || o == cur) {
    haveCand = false;  // moving, flat or unchanged: the dwell starts again
    return cur;
  }
  if (!haveCand || o != cand) {
    cand = o;
    candSince = now;
    haveCand = true;
  } else if (now - candSince >= DWELL_MS) {
    cur = o;
    if (isPortrait(o)) portrait = o;
    haveCand = false;
  }
  return cur;
}

Screen choose(const Tracker& t, bool autoRotate, bool landscape, bool flippedSetting) {
  if (!autoRotate) return {flippedSetting, 0};
  Orient o = t.current();
  int8_t side = !landscape ? 0 : o == Orient::LANDSCAPE_RIGHT ? 1 : o == Orient::LANDSCAPE_LEFT ? -1 : 0;
  return {t.lastPortrait() == Orient::PORTRAIT_FLIPPED, side};
}

}  // namespace orientation
