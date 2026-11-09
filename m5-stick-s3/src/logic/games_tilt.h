// The tilt games (the motion sensor; button A isn't needed): Maze, Balance, Catch. Pure C++
// (native-testable); src/games/ draws them and feeds them the smoothed, calibrated tilt
// (games::Tilt, in g along the screen's axes: +x rolls right, +y rolls down).
#pragma once
#include <stdint.h>
#include "games.h"
#include "games_one.h"  // Base

namespace games {

// --- A maze, made from a seed (a "perfect" maze: one way between any two cells) ---------------
struct MazeGrid {
  static constexpr int MAX_COLS = 10, MAX_ROWS = 17, MAX = MAX_COLS * MAX_ROWS;
  enum : uint8_t { N = 1, E = 2, S = 4, W = 8 };  // a wall on that side
  uint8_t cols = 0, rows = 0;
  uint8_t cell[MAX] = {};

  // Recursive backtracker (with its own stack, no recursion), cols x rows (clamped).
  void generate(int cols, int rows, uint32_t seed);
  bool wall(int c, int r, uint8_t side) const { return cell[r * cols + c] & side; }
  int  openings() const;                              // passages between cells (cols*rows - 1)
  int  distance(int c0, int r0, int c1, int r1) const;  // steps along the passages, -1 = no way
};

// --- Maze: roll the ball from the top left to the exit (bottom right) before the time runs
// out; each level is bigger and gives time back. The score is the levels cleared.
class Maze : public Base {
 public:
  static constexpr int W = 135, H = 240, TOP = 24;  // the HUD above the maze
  static constexpr uint32_t START_MS = 45000, BONUS_MS = 15000, MAX_MS = 99000;
  static constexpr float WALL = 2;

  void  reset(uint32_t seed);
  void  step(uint32_t ms, float tx, float ty);
  int   level() const { return level_; }
  uint32_t timeLeft() const { return left_; }
  uint32_t levelMs() const { return t - levelAt_; }  // since this level started (its banner)
  const MazeGrid& grid() const { return grid_; }
  float cellPx() const { return cell_; }
  float originX() const { return ox_; }
  float originY() const { return oy_; }
  float ballX() const { return bx_; }
  float ballY() const { return by_; }
  float ballR() const { return br_; }
  float speed() const;
  uint32_t seed() const { return seed_; }  // this level's maze (the drawing caches by it)
  // Puts the ball somewhere (the host renderer's pictures, the tests)
  void  place(float x, float y) { bx_ = x; by_ = y; vx_ = vy_ = 0; }

 private:
  void  layout();
  void  collide();
  MazeGrid grid_;
  int   level_ = 1;
  uint32_t left_ = START_MS, levelAt_ = 0, base_ = 1, seed_ = 1;
  float cell_ = 20, ox_ = 0, oy_ = 0, bx_ = 0, by_ = 0, vx_ = 0, vy_ = 0, br_ = 5;
};

// --- Balance: a ball on a beam that the wind tips more and more; tilt the stick against it.
// Points while the ball is in the middle zone (one per 100 ms); off either end = over.
class Balance : public Base {
 public:
  static constexpr int W = 240, H = 135;
  static constexpr float CX = 120, CY = 88, HALF = 98, ZONE = 18, R = 9;
  static constexpr float ACCEL = 560;  // px/s^2 per g of slope

  void  reset(uint32_t seed);
  void  step(uint32_t ms, float tx);
  float angle() const { return angle_; }  // the beam's tilt, radians (+ = right end down)
  float pos() const { return pos_; }      // the ball along the beam, px from the middle
  float vel() const { return vel_; }
  bool  inZone() const { return pos_ > -ZONE && pos_ < ZONE; }
  float fall() const { return fall_; }    // px the ball has fallen (after it rolled off)

 private:
  float angle_ = 0, target_ = 0, pos_ = 0, vel_ = 0, fall_ = 0, fallV_ = 0;
  uint32_t windAt_ = 0, zoneMs_ = 0;
  Rng   rng_;
};

// --- Catch: tilt the basket to catch coins (+1) and stars (+3); a bomb caught or a coin
// missed costs a heart. Three hearts.
class Catch : public Base {
 public:
  static constexpr int W = 135, H = 240, ITEMS = 8, LIVES = 3;
  static constexpr float BASKET_Y = 210, BASKET_W = 36, ITEM_R = 6;
  enum class Kind : uint8_t { COIN, STAR, BOMB };
  struct Item { float x, y, vy; Kind kind; bool active; };

  void  reset(uint32_t seed);
  void  step(uint32_t ms, float tx);
  float basketX() const { return x_; }   // its middle
  float basketV() const { return vx_; }
  int   lives() const { return lives_; }
  const Item& item(int i) const { return items_[i]; }
  uint32_t sinceHit() const { return t - hitAt_; }
  uint32_t sinceCatch() const { return t - catchAt_; }
  // The host renderer's pictures and the tests: an item at x, y
  void  drop(float x, float y, float vy, Kind k);

 private:
  void  spawn();
  Item  items_[ITEMS];
  float x_ = W / 2.0f, vx_ = 0;
  int   lives_ = LIVES;
  uint32_t nextAt_ = 0, hitAt_ = 0xF0000000u, catchAt_ = 0xF0000000u;
  Rng   rng_;
};

}  // namespace games
