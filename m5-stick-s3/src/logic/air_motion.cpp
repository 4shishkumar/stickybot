#include "air_motion.h"
#include <math.h>

namespace airmotion {

static float clampf(float v, float lo, float hi) { return v < lo ? lo : v > hi ? hi : v; }
static float dot(const float a[3], const float b[3]) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
static void  cross(const float a[3], const float b[3], float o[3]) {
  o[0] = a[1] * b[2] - a[2] * b[1];
  o[1] = a[2] * b[0] - a[0] * b[2];
  o[2] = a[0] * b[1] - a[1] * b[0];
}
static bool normalize(float v[3]) {
  float n = sqrtf(dot(v, v));
  if (n < 1e-6f) return false;
  for (int i = 0; i < 3; i++) v[i] /= n;
  return true;
}
// EMA weight for a time constant, whatever the sample interval
static float ema(float dt, float tau) { return tau <= 0 ? 1.0f : clampf(dt / tau, 0.0f, 1.0f); }

// --- Frame ------------------------------------------------------------------------

constexpr float GRAVITY_TAU_S = 0.15f;  // the up direction: steady, yet follows a turn

void Frame::reset() {
  up_[0] = 0; up_[1] = 0; up_[2] = 1;
  primed = false;
}

void Frame::feedAccel(const float a[3], float dt) {
  float v[3] = {a[0], a[1], a[2]};
  if (!normalize(v)) return;  // free fall: keep the last
  if (!primed) {
    for (int i = 0; i < 3; i++) up_[i] = v[i];
    primed = true;
    return;
  }
  float k = ema(dt, GRAVITY_TAU_S);
  for (int i = 0; i < 3; i++) up_[i] += k * (v[i] - up_[i]);
  normalize(up_);
}

Rates Frame::rates(const float g[3], int side) const {
  // The screen's "right" in the stick's axes, for the way the screen is turned
  float R[3] = {1, 0, 0}, Us[3] = {0, 1, 0};  // portrait: right = +x, screen up = +y
  if (side == 2)       { R[0] = -1; Us[1] = -1; }
  else if (side == 1)  { R[0] = 0; R[1] = -1; Us[0] = 1; Us[1] = 0; }  // x up: right = -y
  else if (side == -1) { R[0] = 0; R[1] = 1;  Us[0] = -1; Us[1] = 0; } // x down: right = +y
  float u[3] = {up_[0], up_[1], up_[2]};
  float r[3];
  float ru = dot(R, u);
  if (fabsf(ru) < 0.8f) {
    for (int i = 0; i < 3; i++) r[i] = R[i] - ru * u[i];
  } else {
    // The screen's right edge points (nearly) up or down: the stick is held turned a quarter
    // from the screen's way. Its true right is then the screen's up / down axis.
    float s = ru > 0 ? -1.0f : 1.0f, uu = dot(Us, u);
    for (int i = 0; i < 3; i++) r[i] = s * (Us[i] - uu * u[i]);
  }
  Rates out;
  if (!normalize(r)) return out;
  float f[3];
  cross(u, r, f);  // forward: where the stick points
  out.yaw = -dot(g, u);   // turning right (clockwise seen from above) = pointer right
  out.pitch = -dot(g, r); // tip up (positive around "right") = pointer up (HID y is down)
  out.roll = dot(g, f);   // positive around forward = clockwise seen from behind
  return out;
}

// --- Stillness ----------------------------------------------------------------------

constexpr float MEAN_TAU_S     = 0.25f;
constexpr float BIAS_DRIFT_DPS = 3.0f;  // once the bias is known: a reading this far off it is motion

void Stillness::reset() {
  primed = still_ = false;
  quietSince = since = 0;
}

bool Stillness::feed(uint32_t now, const float a[3], const float g[3], float dt) {
  if (!primed) {
    for (int i = 0; i < 3; i++) { gMean[i] = g[i]; aMean[i] = a[i]; }
    primed = true;
    quietSince = now;
    return still_ = false;
  }
  bool quiet = true;
  for (int i = 0; i < 3; i++) {
    if (fabsf(g[i] - gMean[i]) > NOISE_DPS) quiet = false;
    if (fabsf(g[i]) > MAX_BIAS) quiet = false;
    if (calibrated_ && fabsf(g[i] - bias_[i]) > BIAS_DRIFT_DPS) quiet = false;  // a slow, steady turn
    if (fabsf(a[i] - aMean[i]) > ACCEL_G) quiet = false;
  }
  float k = ema(dt, MEAN_TAU_S);
  for (int i = 0; i < 3; i++) {
    gMean[i] += k * (g[i] - gMean[i]);
    aMean[i] += k * (a[i] - aMean[i]);
  }
  if (!quiet) {
    quietSince = now;
    still_ = false;
    return false;
  }
  if (!still_ && now - quietSince >= STILL_MS) {
    still_ = true;
    since = now;
    if (!calibrated_) {  // the first rest: take the mean as it is
      for (int i = 0; i < 3; i++) bias_[i] = gMean[i];
      calibrated_ = true;
    }
  }
  if (still_) {
    float kb = ema(dt, BIAS_TAU_S);
    for (int i = 0; i < 3; i++) bias_[i] += kb * (g[i] - bias_[i]);
  }
  return still_;
}

// --- Pointer ------------------------------------------------------------------------

float gainFor(uint8_t sensitivity) {
  int s = sensitivity < 1 ? 1 : sensitivity > 10 ? 10 : sensitivity;
  return 4.0f * powf(1.25f, (float)(s - 5));  // 4 counts/° at 5 (6 was too fast on a real Mac)
}

constexpr float Y_GAIN = 1.5f;

float accelFactor(float speedDps) { return 1.0f + 2.5f * clampf((speedDps - 15.0f) / 200.0f, 0.0f, 1.0f); }

void Pointer::configure(uint8_t sensitivity, bool invertX, bool invertY) {
  sens = sensitivity;
  invX = invertX;
  invY = invertY;
}

void Pointer::reset() { sx = sy = accX = accY = 0; frozenUntil = 0; }

void Pointer::freeze(uint32_t now, uint32_t ms) {
  frozenUntil = now + ms;
  if (!frozenUntil) frozenUntil = 1;
  sx = sy = accX = accY = 0;
}

void Pointer::feed(uint32_t now, float dt, float yaw, float pitch, int& dx, int& dy) {
  dx = dy = 0;
  if (invX) yaw = -yaw;
  if (invY) pitch = -pitch;
  float s = sqrtf(yaw * yaw + pitch * pitch);
  if (frozenUntil) {
    if ((int32_t)(now - frozenUntil) < 0 && s < BREAK_DPS) return;
    frozenUntil = 0;
  }
  // Soft dead zone: the tremor under it is dropped, a move above it starts from zero
  if (s <= DEAD_ZONE_DPS) {
    yaw = pitch = 0;
    s = 0;
  } else {
    float k = (s - DEAD_ZONE_DPS) / s;
    yaw *= k;
    pitch *= k;
    s -= DEAD_ZONE_DPS;
  }
  // Smoothing, per 10 ms: 0.3 when slow (steady aim) .. 1 (none) when fast (no lag)
  float a10 = 0.3f + 0.7f * clampf(s / 150.0f, 0.0f, 1.0f);
  float a = 1.0f - powf(1.0f - a10, clampf(dt / 0.01f, 0.0f, 10.0f));
  sx += a * (yaw - sx);
  sy += a * (pitch - sy);
  float sp = sqrtf(sx * sx + sy * sy);
  float g = gainFor(sens) * accelFactor(sp);
  accX += sx * g * dt;
  accY += sy * g * dt * Y_GAIN;  // a wrist tips through less than it turns: up/down goes further
  accX = clampf(accX, -400, 400);  // a stall (screen redraw) can't fling the pointer
  accY = clampf(accY, -400, 400);
  dx = (int)clampf(truncf(accX), -127, 127);
  dy = (int)clampf(truncf(accY), -127, 127);
  accX -= dx;
  accY -= dy;
}

void Pointer::giveBack(int dx, int dy) {
  accX = clampf(accX + dx, -400, 400);
  accY = clampf(accY + dy, -400, 400);
}

// --- Scroller ------------------------------------------------------------------------

void Scroller::configure(uint8_t speed, bool invert) {
  speed_ = speed;
  invert_ = invert;
}

void Scroller::begin() { tiltV = tiltH = accV = accH = 0; }

float Scroller::rateFor(float deg) const {
  float d = fabsf(deg) - DEAD_DEG;
  if (d <= 0) return 0;
  int s = speed_ < 1 ? 1 : speed_ > 10 ? 10 : speed_;
  float k = 0.35f * powf(1.25f, (float)(s - 5));
  float r = clampf(k * d * (1.0f + d / 30.0f), 0, MAX_RATE);
  return deg < 0 ? -r : r;
}

void Scroller::feed(float dt, float yaw, float pitch, int& wheel, int& pan) {
  wheel = pan = 0;
  tiltV = clampf(tiltV - pitch * dt, -80, 80);  // tip up (pitch < 0) = scroll up (wheel +)
  tiltH = clampf(tiltH + yaw * dt, -80, 80);    // turned right = pan right
  float v = rateFor(tiltV);
  // Sideways needs twice the tilt: a hand tilting up turns a little sideways too
  float h = fabsf(tiltH) > DEAD_DEG * 2 ? rateFor(tiltH > 0 ? tiltH - DEAD_DEG : tiltH + DEAD_DEG) : 0;
  if (invert_) { v = -v; h = -h; }
  accV += v * dt;
  accH += h * dt;
  wheel = (int)truncf(accV);
  pan = (int)truncf(accH);
  accV -= wheel;
  accH -= pan;
}

// --- Knob ------------------------------------------------------------------------------

void Knob::reset() {
  angle = 0;
  lastTwist = lastFlick = 0;
  flickArmed = true;
}

KnobEvent Knob::feed(uint32_t now, float dt, const Rates& r) {
  float ay = fabsf(r.yaw), ar = fabsf(r.roll);
  if (ay < FLICK_DPS * 0.4f) flickArmed = true;
  bool cooling = lastFlick && now - lastFlick < FLICK_COOLDOWN_MS;
  if (ay > FLICK_DPS && ay > 1.5f * ar && flickArmed && !cooling) {
    lastFlick = now ? now : 1;
    flickArmed = false;
    angle = 0;
    return r.yaw > 0 ? KnobEvent::NEXT : KnobEvent::PREVIOUS;
  }
  if (cooling || ay > 120.0f) {  // a flick (and its swing back), or the stick swung around
    angle = 0;
    return KnobEvent::NONE;
  }
  if (ar > TWIST_MIN_DPS) {
    angle += r.roll * dt;
    lastTwist = now;
  } else if (now - lastTwist > TWIST_IDLE_MS) {
    angle = 0;
  }
  if (angle >= STEP_DEG) {
    angle -= STEP_DEG;
    return KnobEvent::VOLUME_UP;
  }
  if (angle <= -STEP_DEG) {
    angle += STEP_DEG;
    return KnobEvent::VOLUME_DOWN;
  }
  return KnobEvent::NONE;
}

}  // namespace airmotion
