#include "games_tilt.h"
#include <math.h>
#include <string.h>

namespace games {

static constexpr uint32_t STEP_MS = 8;

static float minf(float a, float b) { return a < b ? a : b; }
static float maxf(float a, float b) { return a > b ? a : b; }
static float clampf(float v, float lo, float hi) { return v < lo ? lo : v > hi ? hi : v; }

// --- MazeGrid --------------------------------------------------------------------------------

void MazeGrid::generate(int c, int r, uint32_t seed) {
  cols = (uint8_t)(c < 2 ? 2 : c > MAX_COLS ? MAX_COLS : c);
  rows = (uint8_t)(r < 2 ? 2 : r > MAX_ROWS ? MAX_ROWS : r);
  memset(cell, N | E | S | W, sizeof cell);
  bool seen[MAX] = {};
  uint8_t stack[MAX];
  int top = 0;
  Rng rng(seed);
  stack[top++] = 0;
  seen[0] = true;
  static const int DC[4] = {0, 1, 0, -1}, DR[4] = {-1, 0, 1, 0};
  static const uint8_t SIDE[4] = {N, E, S, W}, BACK[4] = {S, W, N, E};
  while (top) {
    const int at = stack[top - 1], ac = at % cols, ar = at / cols;
    int options[4], k = 0;
    for (int d = 0; d < 4; d++) {
      const int nc = ac + DC[d], nr = ar + DR[d];
      if (nc >= 0 && nc < cols && nr >= 0 && nr < rows && !seen[nr * cols + nc]) options[k++] = d;
    }
    if (!k) { top--; continue; }
    const int d = options[rng.next() % k], to = (ar + DR[d]) * cols + ac + DC[d];
    cell[at] &= ~SIDE[d];
    cell[to] &= ~BACK[d];
    seen[to] = true;
    stack[top++] = (uint8_t)to;
  }
}

int MazeGrid::openings() const {
  int k = 0;
  for (int r = 0; r < rows; r++)
    for (int c = 0; c < cols; c++) {
      if (c + 1 < cols && !wall(c, r, E)) k++;
      if (r + 1 < rows && !wall(c, r, S)) k++;
    }
  return k;
}

int MazeGrid::distance(int c0, int r0, int c1, int r1) const {
  int16_t dist[MAX];
  uint8_t queue[MAX];
  for (auto& d : dist) d = -1;
  int head = 0, tail = 0;
  const int start = r0 * cols + c0, goal = r1 * cols + c1;
  dist[start] = 0;
  queue[tail++] = (uint8_t)start;
  static const int DC[4] = {0, 1, 0, -1}, DR[4] = {-1, 0, 1, 0};
  static const uint8_t SIDE[4] = {N, E, S, W};
  while (head < tail) {
    const int at = queue[head++];
    if (at == goal) return dist[at];
    const int c = at % cols, r = at / cols;
    for (int d = 0; d < 4; d++) {
      if (cell[at] & SIDE[d]) continue;
      const int nc = c + DC[d], nr = r + DR[d];
      if (nc < 0 || nc >= cols || nr < 0 || nr >= rows) continue;
      const int to = nr * cols + nc;
      if (dist[to] >= 0) continue;
      dist[to] = dist[at] + 1;
      queue[tail++] = (uint8_t)to;
    }
  }
  return -1;
}

// --- Maze ------------------------------------------------------------------------------------

void Maze::reset(uint32_t seed) {
  *this = Maze();
  base_ = seed ? seed : 1;
  layout();
}

void Maze::layout() {
  const int cols = level_ + 4 < MazeGrid::MAX_COLS ? level_ + 4 : MazeGrid::MAX_COLS;
  cell_ = floorf((W - 4) / (float)cols);
  int rows = (int)((H - TOP - 4) / cell_);
  if (rows > MazeGrid::MAX_ROWS) rows = MazeGrid::MAX_ROWS;
  seed_ = base_ * 2654435761u + (uint32_t)level_ * 7919u;
  grid_.generate(cols, rows, seed_);
  ox_ = floorf((W - grid_.cols * cell_) / 2);
  oy_ = TOP + floorf((H - TOP - grid_.rows * cell_) / 2);
  br_ = maxf(3.5f, cell_ * 0.27f);
  bx_ = ox_ + cell_ / 2;
  by_ = oy_ + cell_ / 2;
  vx_ = vy_ = 0;
  levelAt_ = t;
}

float Maze::speed() const { return sqrtf(vx_ * vx_ + vy_ * vy_); }

// The walls around the ball's cell: push it out of each, bounce a little.
void Maze::collide() {
  const int cc = (int)floorf((bx_ - ox_) / cell_), cr = (int)floorf((by_ - oy_) / cell_);
  const float T = WALL;
  for (int r = cr - 1; r <= cr + 1; r++)
    for (int c = cc - 1; c <= cc + 1; c++) {
      if (c < 0 || r < 0 || c >= grid_.cols || r >= grid_.rows) continue;
      const float x0 = ox_ + c * cell_, y0 = oy_ + r * cell_;
      float box[4][4];
      int nb = 0;
      if (grid_.wall(c, r, MazeGrid::N)) { float b[4] = {x0 - T / 2, y0 - T / 2, cell_ + T, T}; memcpy(box[nb++], b, sizeof b); }
      if (grid_.wall(c, r, MazeGrid::S)) { float b[4] = {x0 - T / 2, y0 + cell_ - T / 2, cell_ + T, T}; memcpy(box[nb++], b, sizeof b); }
      if (grid_.wall(c, r, MazeGrid::W)) { float b[4] = {x0 - T / 2, y0 - T / 2, T, cell_ + T}; memcpy(box[nb++], b, sizeof b); }
      if (grid_.wall(c, r, MazeGrid::E)) { float b[4] = {x0 + cell_ - T / 2, y0 - T / 2, T, cell_ + T}; memcpy(box[nb++], b, sizeof b); }
      for (int i = 0; i < nb; i++) {
        float nx, ny;
        if (!circleBox(bx_, by_, br_, box[i][0], box[i][1], box[i][2], box[i][3], nx, ny)) continue;
        const float vn = vx_ * nx + vy_ * ny;
        if (vn < 0) {
          if (-vn > cell_ * 4) fx |= FX_WALL;
          vx_ -= 1.3f * vn * nx;  // a soft bounce (0.3)
          vy_ -= 1.3f * vn * ny;
        }
      }
    }
  // never out of the maze, whatever happened
  bx_ = clampf(bx_, ox_ + br_, ox_ + grid_.cols * cell_ - br_);
  by_ = clampf(by_, oy_ + br_, oy_ + grid_.rows * cell_ - br_);
}

void Maze::step(uint32_t ms, float tx, float ty) {
  t += ms;
  if (over) return;
  left_ = left_ > ms ? left_ - ms : 0;
  const float accel = 40 * cell_, vmax = 12 * cell_;
  uint32_t rest = ms;
  while (rest) {
    const uint32_t s = rest > STEP_MS ? STEP_MS : rest;
    rest -= s;
    const float dt = s / 1000.0f;
    vx_ += tx * accel * dt;
    vy_ += ty * accel * dt;
    const float damp = expf(-1.2f * dt);
    vx_ *= damp;
    vy_ *= damp;
    const float v = speed();
    if (v > vmax) { vx_ *= vmax / v; vy_ *= vmax / v; }
    // short moves (less than half the ball) so it never jumps a wall
    const int n = 1 + (int)(speed() * dt / (br_ * 0.5f));
    for (int i = 0; i < n; i++) {
      bx_ += vx_ * dt / n;
      by_ += vy_ * dt / n;
      collide();
    }
    const float ex = ox_ + (grid_.cols - 0.5f) * cell_, ey = oy_ + (grid_.rows - 0.5f) * cell_;
    if ((bx_ - ex) * (bx_ - ex) + (by_ - ey) * (by_ - ey) < cell_ * cell_ * 0.09f) {  // in the exit
      score++;
      left_ = left_ + BONUS_MS > MAX_MS ? MAX_MS : left_ + BONUS_MS;
      level_++;
      fx |= FX_LEVEL | FX_SCORE;
      layout();
      return;
    }
  }
  if (!left_) {
    over = true;
    fx |= FX_OVER;
  }
}

// --- Balance ---------------------------------------------------------------------------------

void Balance::reset(uint32_t seed) {
  *this = Balance();
  rng_.seed_(seed);
  windAt_ = 1500;  // a calm moment first
}

void Balance::step(uint32_t ms, float tx) {
  uint32_t rest = ms;
  while (rest) {
    const uint32_t s = rest > STEP_MS ? STEP_MS : rest;
    rest -= s;
    t += s;
    const float dt = s / 1000.0f;
    if (over) {  // the ball falls off the end
      fallV_ += 900 * dt;
      fall_ += fallV_ * dt;
      pos_ += vel_ * dt;
      continue;
    }
    if (t >= windAt_) {  // the wind changes: a new lean, stronger as time goes on
      const float amp = minf(0.07f + t * 0.000003f, 0.26f);
      target_ = rng_.between(-amp, amp);
      windAt_ = t + (uint32_t)rng_.range(900, 2200);
    }
    const float turn = 0.35f * dt;  // the beam turns towards it at most this fast (rad/s)
    angle_ += clampf(target_ - angle_, -turn, turn);
    vel_ += ACCEL * (sinf(angle_) + tx) * dt;
    vel_ *= expf(-0.5f * dt);
    pos_ += vel_ * dt;
    if (inZone()) {
      zoneMs_ += s;
      while (zoneMs_ >= 100) {
        zoneMs_ -= 100;
        score++;
        if (score % 50 == 0) fx |= FX_SCORE;
      }
    }
    if (pos_ < -HALF || pos_ > HALF) {
      over = true;
      fx |= FX_HIT | FX_OVER;
    }
  }
}

// --- Catch -----------------------------------------------------------------------------------

void Catch::reset(uint32_t seed) {
  *this = Catch();
  rng_.seed_(seed);
  for (auto& i : items_) i.active = false;
  nextAt_ = 600;
}

void Catch::drop(float x, float y, float vy, Kind k) {
  for (auto& i : items_)
    if (!i.active) { i = {x, y, vy, k, true}; return; }
}

void Catch::spawn() {
  const float bomb = minf(0.15f + score * 0.004f, 0.32f), r = rng_.unit();
  const Kind k = r < bomb ? Kind::BOMB : r < bomb + 0.1f ? Kind::STAR : Kind::COIN;
  // one draw per line: arguments are evaluated in no set order (gcc and clang differ)
  const float x = rng_.between(10, W - 10);
  const float vy = 70 + minf(score * 2.2f, 110) + rng_.between(0, 20);
  drop(x, -ITEM_R, vy, k);
  const uint32_t every = (uint32_t)maxf(380, 900 - score * 12.0f);
  nextAt_ = t + every;
}

void Catch::step(uint32_t ms, float tx) {
  uint32_t rest = ms;
  while (rest) {
    const uint32_t s = rest > STEP_MS ? STEP_MS : rest;
    rest -= s;
    t += s;
    if (over) continue;
    const float dt = s / 1000.0f;
    const float want = tx * 520;  // px/s per g
    vx_ += (want - vx_) * minf(1, dt * 12);
    x_ += vx_ * dt;
    const float lo = BASKET_W / 2, hi = W - BASKET_W / 2;
    if (x_ < lo) { x_ = lo; vx_ = 0; }
    if (x_ > hi) { x_ = hi; vx_ = 0; }
    if (t >= nextAt_) spawn();
    for (auto& i : items_) {
      if (!i.active) continue;
      const float was = i.y;
      i.y += i.vy * dt;
      // crossed the basket's rim this step, over the basket: caught
      if (was + ITEM_R <= BASKET_Y + 4 && i.y + ITEM_R >= BASKET_Y && fabsf(i.x - x_) <= BASKET_W / 2 + 3) {
        i.active = false;
        if (i.kind == Kind::BOMB) {
          lives_--;
          hitAt_ = t;
          fx |= FX_HIT;
        } else {
          score += i.kind == Kind::STAR ? 3 : 1;
          catchAt_ = t;
          fx |= i.kind == Kind::STAR ? FX_BONUS : FX_SCORE;
        }
      } else if (i.y - ITEM_R > H) {
        i.active = false;
        if (i.kind == Kind::COIN) {  // a coin missed
          lives_--;
          hitAt_ = t;
          fx |= FX_HIT;
        }
      }
    }
    if (lives_ <= 0) {
      lives_ = 0;
      over = true;
      fx |= FX_OVER;
    }
  }
}

}  // namespace games
