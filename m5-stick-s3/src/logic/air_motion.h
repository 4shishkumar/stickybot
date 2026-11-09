// The air mouse's motion maths: gyro (deg/s) + accelerometer (g) samples at ~100 Hz in,
// pointer counts / scroll detents / volume steps out. Pure C++ (native-testable): air.cpp
// feeds it the IMU readings and sends what comes out over Bluetooth.
//
// Axes (StickS3, see orientation.h): x towards the right edge of the portrait screen,
// y along the stick (upright +1 g), z out of the screen. A right-handed frame, the gyro
// positive counter-clockwise around each axis.
//
// Orientation-aware: the pointer follows the world, not the stick. "Up" is gravity (the
// accelerometer, low-passed); turning around it moves the pointer sideways (yaw), turning
// around the screen's horizontal "right" axis moves it up and down (pitch), and turning
// around the pointing direction (roll) does nothing (the media knob uses it). So the stick
// works held like a TV remote (screen up), upright (screen towards you) or anything between,
// in portrait or landscape.
#pragma once
#include <stdint.h>

namespace airmotion {

// --- Frame: gyro -> yaw / pitch / roll rates in the world --------------------------------

struct Rates {
  float yaw = 0;    // deg/s, + = the pointer goes right
  float pitch = 0;  // deg/s, + = the pointer goes down
  float roll = 0;   // deg/s, + = turned clockwise (seen from behind the stick)
};

class Frame {
 public:
  // side: which way the screen is turned (orientation.h): 0 portrait, 2 portrait upside
  // down, +1 landscape with x up (rotation 1), -1 landscape with x down (rotation 3).
  void  reset();
  void  feedAccel(const float a[3], float dt);  // low-passed gravity (the "up" direction)
  Rates rates(const float g[3], int side) const;
  void  up(float out[3]) const { out[0] = up_[0]; out[1] = up_[1]; out[2] = up_[2]; }

 private:
  float up_[3] = {0, 0, 1};
  bool  primed = false;
};

// --- Stillness: is the stick lying still (on a desk)? + the gyro's bias -------------------

// Still = the gyro barely changes (whatever its bias) and the accelerometer is steady,
// for STILL_MS. While still, the gyro reading *is* the bias: it is learned, so the pointer
// never drifts. A hand never counts as still (tremor is well above the threshold).
class Stillness {
 public:
  static constexpr float    NOISE_DPS   = 1.2f;   // |gyro - its recent mean| below this, every axis
  static constexpr float    MAX_BIAS    = 12.0f;  // a reading above this (any axis) is motion, not bias
  static constexpr float    ACCEL_G     = 0.025f; // |accel - its recent mean| below this
  static constexpr uint32_t STILL_MS    = 600;    // that long = still
  static constexpr float    BIAS_TAU_S  = 0.8f;   // how fast the bias follows while still

  void  reset();
  // Returns true while still. dt in seconds.
  bool  feed(uint32_t now, const float a[3], const float g[3], float dt);
  bool  still() const { return still_; }
  uint32_t stillFor(uint32_t now) const { return still_ ? now - since : 0; }
  void  bias(float out[3]) const { out[0] = bias_[0]; out[1] = bias_[1]; out[2] = bias_[2]; }
  void  setBias(const float b[3]) { for (int i = 0; i < 3; i++) bias_[i] = b[i]; }
  bool  calibrated() const { return calibrated_; }

 private:
  float    gMean[3] = {0, 0, 0}, aMean[3] = {0, 0, 1}, bias_[3] = {0, 0, 0};
  bool     primed = false, still_ = false, calibrated_ = false;
  uint32_t quietSince = 0, since = 0;
};

// --- Pointer: yaw / pitch rates -> mouse counts --------------------------------------------

// Sensitivity 1-10 (5 = default). The curve: a soft dead zone (hand tremor), smoothing that
// is strong when slow (steady aim) and light when fast (no lag), then a gain that grows with
// speed (pointer acceleration: slow = precise, fast = across the screen).
constexpr float DEAD_ZONE_DPS = 2.0f;

float gainFor(uint8_t sensitivity);           // counts per degree at slow speed
float accelFactor(float speedDps);            // 1 when slow .. 3.5 when fast

class Pointer {
 public:
  static constexpr uint32_t FREEZE_MS = 140;  // after a click: hold still (no accidental drag)
  static constexpr float    BREAK_DPS = 120;  // ... unless the hand clearly moves on

  void configure(uint8_t sensitivity, bool invertX, bool invertY);
  void reset();                                     // drops the smoothing and the sub-counts
  void freeze(uint32_t now, uint32_t ms = FREEZE_MS);  // a click: hold the pointer
  // yaw / pitch from Frame::rates (bias already removed). Writes whole counts (clamped to
  // +-127, the remainder carries over to the next sample).
  void feed(uint32_t now, float dt, float yaw, float pitch, int& dx, int& dy);
  // A report that couldn't be sent: its counts go back into the carry-over.
  void giveBack(int dx, int dy);

 private:
  uint8_t  sens = 5;
  bool     invX = false, invY = false;
  float    sx = 0, sy = 0;        // smoothed rates
  float    accX = 0, accY = 0;    // sub-count carry-over
  uint32_t frozenUntil = 0;
};

// --- Scroller: tilt while B is held -> wheel / pan detents ------------------------------

// The angle turned since the hold began (integrated gyro, so any orientation works), past a
// small dead zone, sets the scroll speed: tilt up = scroll up, further = faster.
class Scroller {
 public:
  static constexpr float DEAD_DEG  = 4.0f;
  static constexpr float MAX_RATE  = 40.0f;  // detents / s

  void configure(uint8_t speed, bool invert);  // speed 1-10 (5 = default)
  void begin();                                // the hold started: angles from here
  // pitch / yaw rates (deg/s) from Frame::rates. Writes whole detents (wheel + = up,
  // pan + = right), the remainder carries over.
  void feed(float dt, float yaw, float pitch, int& wheel, int& pan);
  float tiltDeg() const { return tiltV; }
  float rateFor(float deg) const;              // detents / s for a tilt (signed)

 private:
  uint8_t speed_ = 5;
  bool    invert_ = false;
  float   tiltV = 0, tiltH = 0, accV = 0, accH = 0;
};

// --- Knob: the media remote's twist (volume) and flick (track) --------------------------

enum class KnobEvent : uint8_t { NONE, VOLUME_UP, VOLUME_DOWN, NEXT, PREVIOUS };

// Twist = turning the stick around the way it points (roll), like a volume knob: one step
// per STEP_DEG. Flick = a quick sideways flick (yaw spike) to the right = next track, left =
// previous. A flick's own roll never counts as a twist.
class Knob {
 public:
  static constexpr float    STEP_DEG      = 15.0f;
  static constexpr float    TWIST_MIN_DPS = 25.0f;   // slower turns are drift, not twist
  static constexpr uint32_t TWIST_IDLE_MS = 450;     // the part-step resets after this pause
  static constexpr float    FLICK_DPS     = 260.0f;
  static constexpr uint32_t FLICK_COOLDOWN_MS = 650;

  void      reset();
  KnobEvent feed(uint32_t now, float dt, const Rates& r);  // at most one event per sample
  float     partial() const { return angle; }              // the twist towards the next step

 private:
  float    angle = 0;
  uint32_t lastTwist = 0, lastFlick = 0;
  bool     flickArmed = true;
};

}  // namespace airmotion
