// The one-button games (button A only): Stack, Flappy, Dino, Reaction. Pure C++
// (native-testable); src/games/ draws them. Sizes are the screen's pixels: portrait
// 135 x 240 (Stack, Flappy, Reaction), landscape 240 x 135 (Dino).
#pragma once
#include <stdint.h>
#include "games.h"

namespace games {

// Shared by every game: the score, game over and what happened (Fx bits).
struct Base {
  long     score = 0;
  bool     over = false;
  uint32_t t = 0;   // ms played
  uint16_t fx = 0;  // Fx bits since the last takeFx()
  uint16_t takeFx() { uint16_t f = fx; fx = 0; return f; }
};

// --- Stack: drop the sliding block on the tower; what hangs over is cut off --------------
// A drop within PERFECT_PX of the block below is perfect: it snaps on, scores a bonus point,
// and every third perfect in a row widens the block again. Missing the tower ends the game.
class Stack : public Base {
 public:
  static constexpr int   W = 135, BLOCK_H = 12, KEEP = 48;
  static constexpr float START_W = 90, PERFECT_PX = 3, GROW_PX = 6;
  enum class Drop { MISS, CUT, PERFECT };
  struct Block { float x, w; };

  void  reset(uint32_t seed);
  void  step(uint32_t ms);
  Drop  drop();
  int   height() const { return placed_; }       // blocks on the tower (the base is 0)
  const Block& block(int level) const { return ring_[level % KEEP]; }  // level 0..height()
  float x() const { return x_; }                  // the sliding block
  float w() const { return w_; }
  float speed() const { return speed_; }
  // where it turns: mostly off the screen, so it comes in from the side
  float lo() const { return -w_ * 0.8f; }
  float hi() const { return W - w_ * 0.2f; }
  int   combo() const { return combo_; }          // perfect drops in a row
  // The piece cut off by the last drop, falling (drawn while debris() is true)
  bool  debris() const { return debT_ < 1200; }
  float debX() const { return debX_; }
  float debW() const { return debW_; }
  int   debLevel() const { return debLevel_; }
  float debFall() const { return debFall_; }      // px fallen
  uint32_t lastDropMs() const { return t - dropAt_; }  // since the last drop (the perfect flash)
  Drop  lastDrop() const { return last_; }

 private:
  void  next();
  Block ring_[KEEP];
  int   placed_ = 0, combo_ = 0, dir_ = 1, debLevel_ = 0;
  float x_ = 0, w_ = START_W, speed_ = 0, debX_ = 0, debW_ = 0, debFall_ = 0, debV_ = 0;
  uint32_t debT_ = 99999, dropAt_ = 0;
  Drop  last_ = Drop::CUT;
  Rng   rng_;
};

// --- Flappy: tap to flap through the gaps -------------------------------------------------
class Flappy : public Base {
 public:
  static constexpr int   W = 135, H = 240, GROUND = 222, PIPES = 3;
  static constexpr float BIRD_X = 38, R = 8, PIPE_W = 24, SPACING = 88;
  static constexpr float GRAVITY = 900, FLAP_V = -240, MAX_FALL = 380;
  struct Pipe { float x, gapY, gapH; bool scored; };

  void  reset(uint32_t seed);
  void  flap();                 // the first one starts the game
  void  step(uint32_t ms);
  bool  started() const { return started_; }
  float y() const { return y_; }
  float vy() const { return vy_; }
  float speed() const { return speed_; }
  const Pipe& pipe(int i) const { return pipes_[i]; }
  float scroll() const { return scroll_; }  // px moved, for the ground's stripes

 private:
  void  place(Pipe& p, float x);
  Pipe  pipes_[PIPES];
  float y_ = 0, vy_ = 0, speed_ = 0, scroll_ = 0;
  bool  started_ = false;
  Rng   rng_;
};

// --- Dino: run, jump the bugs, pass under the drones -------------------------------------
// A press jumps; holding A keeps the jump rising longer (up to HOLD_MS), so a tap is a short
// hop and a hold clears the tall ones. It speeds up as it goes.
class Dino : public Base {
 public:
  static constexpr int   W = 240, H = 135, GROUND = 114, OBST = 5;
  static constexpr float BOT_X = 34, BOT_W = 18, BOT_H = 24;
  static constexpr float JUMP_V = 330, G_HOLD = 650, G = 2000, HOLD_MS = 230;
  static constexpr float START_SPEED = 120, MAX_SPEED = 300;
  struct Obstacle { float x, w, h, lift; bool fly, active; };  // lift: bottom above the ground

  void  reset(uint32_t seed);
  void  press();
  void  release();
  void  step(uint32_t ms);
  float height() const { return y_; }        // the bot's feet above the ground
  bool  onGround() const { return y_ <= 0 && vy_ <= 0; }
  float speed() const { return speed_; }
  float distance() const { return dist_; }
  const Obstacle& obstacle(int i) const { return obs_[i]; }

 private:
  void  spawn();
  Obstacle obs_[OBST];
  float y_ = 0, vy_ = 0, speed_ = START_SPEED, dist_ = 0, holdMs_ = 0, nextGap_ = 0;
  bool  holding_ = false;
  Rng   rng_;
};

// --- Reaction: wait for green, press A; three rounds, the average in ms (lower is better) ---
class Reaction : public Base {
 public:
  static constexpr int ROUNDS = 3;
  static constexpr uint32_t MIN_WAIT = 1200, MAX_WAIT = 3800, SLOW_MS = 1000, SHOW_MS = 1100;
  enum class Phase { WAIT, GO, SHOWN, DONE, FALSE_START };

  void  reset(uint32_t seed);
  void  step(uint32_t ms);
  // A pressed; lateMs: how long ago it really was (the button's own timestamp), so a slow
  // frame doesn't count against you.
  void  press(uint32_t lateMs = 0);
  Phase phase() const { return phase_; }
  int   round() const { return round_; }            // 0..ROUNDS-1 (the one playing / shown)
  int   time(int i) const { return times_[i]; }     // ms per round (0 = not yet)
  uint32_t inPhase() const { return t - phaseAt_; }
  bool  failed() const { return phase_ == Phase::FALSE_START; }

 private:
  void  wait();
  Phase phase_ = Phase::WAIT;
  int   round_ = 0, times_[ROUNDS] = {};
  uint32_t phaseAt_ = 0, waitMs_ = 0;
  Rng   rng_;
};

}  // namespace games
