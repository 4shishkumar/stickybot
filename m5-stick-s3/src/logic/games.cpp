#include "games.h"
#include <math.h>
#include <string.h>

namespace games {

static const Info INFO[COUNT] = {
    {"stack", "Stack", "A drops the block. Line it up!", false, false, false},
    {"flappy", "Flappy", "Tap A to flap through the pipes", false, false, false},
    {"dino", "Dino", "A jumps, hold it to jump higher", false, false, true},
    {"reaction", "Reaction", "Press A when it turns green", false, true, false},
    {"maze", "Maze", "Tilt to roll the ball out", true, false, false},
    {"balance", "Balance", "Tilt to keep it in the middle", true, false, true},
    {"catch", "Catch", "Tilt the basket. No bombs!", true, false, false},
};

const Info& info(Id g) { return INFO[(int)g < COUNT ? (int)g : 0]; }

int find(const char* id) {
  if (!id) return -1;
  for (int i = 0; i < COUNT; i++)
    if (!strcmp(id, INFO[i].id)) return i;
  return -1;
}

bool better(Id g, long score, long best) {
  if (score <= 0) return false;
  if (info(g).lowerBetter) return best <= 0 || score < best;
  return score > best;
}

uint32_t Rng::next() {
  s ^= s << 13;
  s ^= s >> 17;
  s ^= s << 5;
  return s;
}

float Rng::unit() { return (next() >> 8) * (1.0f / 16777216.0f); }

int Rng::range(int lo, int hi) {
  if (hi <= lo) return lo;
  return lo + (int)(next() % (uint32_t)(hi - lo + 1));
}

float Rng::between(float lo, float hi) { return lo + (hi - lo) * unit(); }

void screenTilt(int rotation, float ax, float ay, float& tx, float& ty) {
  switch (rotation & 3) {
    case 1: tx = ay; ty = ax; break;
    case 2: tx = ax; ty = -ay; break;
    case 3: tx = -ay; ty = -ax; break;
    default: tx = -ax; ty = ay; break;
  }
}

void Tilt::begin(int rotation) {
  rot_ = rotation;
  n_ = samples_ = 0;
  sumX_ = sumY_ = zeroX_ = zeroY_ = x_ = y_ = 0;
}

void Tilt::sample(float ax, float ay) {
  sumX_ += ax;
  sumY_ += ay;
  samples_++;
}

void Tilt::calibrate() {
  if (samples_) {
    zeroX_ = sumX_ / samples_;
    zeroY_ = sumY_ / samples_;
  }
  n_ = samples_ ? samples_ : 1;
  sumX_ = sumY_ = 0;
  samples_ = 0;
  x_ = y_ = 0;
}

static float shape(float v) {
  // the dead zone, then continuous from 0 (no jump at its edge), clamped
  float a = fabsf(v) - Tilt::DEAD_G;
  if (a <= 0) return 0;
  if (a > Tilt::MAX_G) a = Tilt::MAX_G;
  return v < 0 ? -a : a;
}

void Tilt::feed(float ax, float ay, float dtS) {
  float tx, ty;
  screenTilt(rot_, ax - zeroX_, ay - zeroY_, tx, ty);
  tx = shape(tx);
  ty = shape(ty);
  const float k = dtS <= 0 ? 0 : dtS >= TAU_S * 4 ? 1 : 1 - expf(-dtS / TAU_S);
  x_ += (tx - x_) * k;
  y_ += (ty - y_) * k;
}

bool circleHitsBox(float cx, float cy, float r, float bx, float by, float bw, float bh) {
  const float px = cx < bx ? bx : cx > bx + bw ? bx + bw : cx;
  const float py = cy < by ? by : cy > by + bh ? by + bh : cy;
  const float dx = cx - px, dy = cy - py;
  return dx * dx + dy * dy < r * r;
}

bool circleBox(float& cx, float& cy, float r, float bx, float by, float bw, float bh, float& nx, float& ny) {
  const float px = cx < bx ? bx : cx > bx + bw ? bx + bw : cx;
  const float py = cy < by ? by : cy > by + bh ? by + bh : cy;
  float dx = cx - px, dy = cy - py;
  const float d2 = dx * dx + dy * dy;
  if (d2 >= r * r) return false;
  if (d2 > 1e-6f) {  // the centre is outside the box: out along the closest point
    const float d = sqrtf(d2);
    nx = dx / d;
    ny = dy / d;
    cx = px + nx * r;
    cy = py + ny * r;
    return true;
  }
  // the centre is inside: out the nearest side
  const float l = cx - bx, rr = bx + bw - cx, t = cy - by, b = by + bh - cy;
  float m = l;
  nx = -1; ny = 0;
  if (rr < m) { m = rr; nx = 1; ny = 0; }
  if (t < m) { m = t; nx = 0; ny = -1; }
  if (b < m) { nx = 0; ny = 1; }
  if (nx < 0) cx = bx - r;
  else if (nx > 0) cx = bx + bw + r;
  else if (ny < 0) cy = by - r;
  else cy = by + bh + r;
  return true;
}

}  // namespace games
