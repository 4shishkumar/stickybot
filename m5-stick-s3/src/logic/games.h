// Games on the stick (firmware 2.5.0): what they share. Pure C++ (native-testable):
// the list, the scores, a seeded random number generator and the tilt input. The games
// themselves are in games_one.h (A only) and games_tilt.h (the motion sensor); src/games/
// draws them and runs a session (buttons, sounds, the best scores in NVS, the agent).
//
// Every game is deterministic: the same seed and the same inputs at the same times give
// the same game (the tests rely on it, and so would two sticks racing the same maze).
// Time goes in as whole milliseconds; each game splits a long frame into short steps.
#pragma once
#include <stdint.h>

namespace games {

enum class Id : uint8_t { STACK, FLAPPY, DINO, REACTION, MAZE, BALANCE, CATCH, COUNT };
constexpr int COUNT = (int)Id::COUNT;

struct Info {
  const char* id;     // "stack": the protocol's and the NVS key's name (<= 15 characters)
  const char* title;  // "Stack"
  const char* how;    // one line for the picker
  bool tilt;          // played with the motion sensor (calibrated flat at the start)
  bool lowerBetter;   // the best is the lowest score (Reaction: milliseconds)
  bool landscape;     // drawn 240 x 135 (else portrait 135 x 240)
};
const Info& info(Id g);
int  find(const char* id);  // -1 if unknown
// Is `score` a new best over `best`? A score of 0 never is (nothing scored, or a false
// start); for lowerBetter games a best of 0 means "none yet".
bool better(Id g, long score, long best);

// Things that happened in a step, for sounds and little effects (bits, read and cleared
// by the session: takeFx()).
enum Fx : uint16_t {
  FX_NONE = 0,
  FX_ACTION = 1 << 0,   // a flap, a jump, a drop
  FX_SCORE = 1 << 1,    // a point
  FX_PERFECT = 1 << 2,  // Stack: a perfect drop
  FX_HIT = 1 << 3,      // lost a life / crashed
  FX_OVER = 1 << 4,     // game over
  FX_LEVEL = 1 << 5,    // Maze: a level cleared
  FX_GO = 1 << 6,       // Reaction: green
  FX_BONUS = 1 << 7,    // Catch: a star
  FX_WALL = 1 << 8,     // Maze: the ball hit a wall hard
};

// xorshift32: small, fast and the same everywhere. A seed of 0 is changed to 1.
struct Rng {
  uint32_t s = 1;
  explicit Rng(uint32_t seed = 1) { seed_(seed); }
  void     seed_(uint32_t seed) { s = seed ? seed : 1; }
  uint32_t next();
  float    unit();                 // 0 <= x < 1
  int      range(int lo, int hi);  // lo <= x <= hi
  float    between(float lo, float hi);
};

// --- Tilt ----------------------------------------------------------------------------
// The stick lying (about) flat: the accelerometer's x / y (g, the stick's axes, see
// logic/orientation.h) are how far it leans. screenTilt() turns them into the screen's
// axes for a rotation (0 portrait, 1 / 3 landscape with button A right / left, 2 upside
// down): +x = rolls right, +y = rolls down the screen. The same as ui::gravity().
void screenTilt(int rotation, float ax, float ay, float& tx, float& ty);

// "Flat" is how the stick lies when a game starts (calibrate()), not level: you can play
// it held at an angle. Then feed() smooths the readings (a short low-pass, ~60 ms), drops
// a small dead zone (a resting hand) and clamps to +-MAX_G.
class Tilt {
 public:
  static constexpr float DEAD_G = 0.025f;
  static constexpr float MAX_G = 0.6f;
  static constexpr float TAU_S = 0.06f;
  void  begin(int rotation);                  // forget the calibration and the smoothing
  void  sample(float ax, float ay);           // a calibration reading (the start's countdown)
  void  calibrate();                          // the average of the samples is "flat"
  bool  calibrated() const { return n_ > 0; }
  void  feed(float ax, float ay, float dtS);  // a reading while playing
  float x() const { return x_; }              // screen axes, g
  float y() const { return y_; }

 private:
  int   rot_ = 0, n_ = 0, samples_ = 0;
  float sumX_ = 0, sumY_ = 0, zeroX_ = 0, zeroY_ = 0, x_ = 0, y_ = 0;
};

// Circle against an axis-aligned box: pushes (cx, cy) out of the box and returns the
// normal it was pushed along (nx, ny), or false when they don't touch.
bool circleBox(float& cx, float& cy, float r, float bx, float by, float bw, float bh, float& nx, float& ny);
bool circleHitsBox(float cx, float cy, float r, float bx, float by, float bw, float bh);

}  // namespace games
