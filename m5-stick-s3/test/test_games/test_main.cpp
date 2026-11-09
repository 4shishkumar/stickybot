// Games (firmware 2.5.0): the list and scores, the tilt input, and each game's rules and
// physics, all deterministic (a seed + the same inputs = the same game).
#include <unity.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include "logic/games.h"
#include "logic/games_one.h"
#include "logic/games_tilt.h"

using namespace games;

void setUp() {}
void tearDown() {}

// --- shared -------------------------------------------------------------------------------

void test_list() {
  TEST_ASSERT_EQUAL(7, COUNT);
  TEST_ASSERT_EQUAL((int)Id::MAZE, find("maze"));
  TEST_ASSERT_EQUAL(-1, find("tetris"));
  TEST_ASSERT_EQUAL(-1, find(nullptr));
  for (int i = 0; i < COUNT; i++) {
    TEST_ASSERT_EQUAL(i, find(info((Id)i).id));
    TEST_ASSERT_TRUE(strlen(info((Id)i).id) <= 15);  // an NVS key
  }
  TEST_ASSERT_TRUE(info(Id::DINO).landscape);
  TEST_ASSERT_TRUE(info(Id::BALANCE).landscape && info(Id::BALANCE).tilt);
  TEST_ASSERT_FALSE(info(Id::FLAPPY).tilt);
}

void test_better() {
  TEST_ASSERT_TRUE(better(Id::STACK, 5, 4));
  TEST_ASSERT_FALSE(better(Id::STACK, 4, 4));
  TEST_ASSERT_FALSE(better(Id::STACK, 0, 0));        // nothing scored is never a best
  TEST_ASSERT_TRUE(better(Id::REACTION, 250, 0));    // lower is better; 0 = none yet
  TEST_ASSERT_TRUE(better(Id::REACTION, 240, 250));
  TEST_ASSERT_FALSE(better(Id::REACTION, 260, 250));
  TEST_ASSERT_FALSE(better(Id::REACTION, 0, 250));   // a false start
}

void test_rng() {
  Rng a(42), b(42), c(43);
  bool differs = false;
  for (int i = 0; i < 100; i++) {
    uint32_t x = a.next();
    TEST_ASSERT_EQUAL_UINT32(x, b.next());
    differs |= x != c.next();
  }
  TEST_ASSERT_TRUE(differs);
  Rng z(0);  // a zero seed would stick at zero
  TEST_ASSERT_NOT_EQUAL(0, z.next());
  Rng r(7);
  int lo = 99, hi = -99;
  for (int i = 0; i < 2000; i++) {
    int v = r.range(-3, 3);
    lo = v < lo ? v : lo;
    hi = v > hi ? v : hi;
    float u = r.unit();
    TEST_ASSERT_TRUE(u >= 0 && u < 1);
  }
  TEST_ASSERT_EQUAL(-3, lo);
  TEST_ASSERT_EQUAL(3, hi);
}

void test_screen_tilt() {
  float x, y;
  // portrait: the right edge down (ax < 0) rolls right; the top edge down (ay < 0) rolls up
  screenTilt(0, -0.2f, 0, x, y);
  TEST_ASSERT_FLOAT_WITHIN(1e-6, 0.2f, x);
  screenTilt(0, 0, -0.3f, x, y);
  TEST_ASSERT_FLOAT_WITHIN(1e-6, -0.3f, y);
  screenTilt(2, -0.2f, -0.3f, x, y);  // upside down: both turn round
  TEST_ASSERT_FLOAT_WITHIN(1e-6, -0.2f, x);
  TEST_ASSERT_FLOAT_WITHIN(1e-6, 0.3f, y);
  screenTilt(1, 0.1f, 0.4f, x, y);  // landscape, A on the right
  TEST_ASSERT_FLOAT_WITHIN(1e-6, 0.4f, x);
  TEST_ASSERT_FLOAT_WITHIN(1e-6, 0.1f, y);
  screenTilt(3, 0.1f, 0.4f, x, y);
  TEST_ASSERT_FLOAT_WITHIN(1e-6, -0.4f, x);
  TEST_ASSERT_FLOAT_WITHIN(1e-6, -0.1f, y);
}

void test_tilt_calibration_and_smoothing() {
  Tilt t;
  t.begin(0);
  TEST_ASSERT_FALSE(t.calibrated());
  // held at an angle at the start: that is "flat"
  for (int i = 0; i < 20; i++) t.sample(0.30f + (i % 2 ? 0.01f : -0.01f), -0.20f);
  t.calibrate();
  TEST_ASSERT_TRUE(t.calibrated());
  for (int i = 0; i < 50; i++) t.feed(0.30f, -0.20f, 0.02f);
  TEST_ASSERT_FLOAT_WITHIN(1e-4, 0, t.x());
  TEST_ASSERT_FLOAT_WITHIN(1e-4, 0, t.y());
  // a small wobble is inside the dead zone
  for (int i = 0; i < 50; i++) t.feed(0.31f, -0.21f, 0.02f);
  TEST_ASSERT_FLOAT_WITHIN(1e-4, 0, t.x());
  // a lean: smoothed (not all at once), then settles; continuous past the dead zone
  t.feed(0.30f - 0.2f, -0.20f, 0.02f);
  TEST_ASSERT_TRUE(t.x() > 0 && t.x() < 0.2f - Tilt::DEAD_G);
  for (int i = 0; i < 50; i++) t.feed(0.30f - 0.2f, -0.20f, 0.02f);
  TEST_ASSERT_FLOAT_WITHIN(1e-3, 0.2f - Tilt::DEAD_G, t.x());
  // clamped
  for (int i = 0; i < 50; i++) t.feed(0.30f - 2.0f, -0.20f, 0.02f);
  TEST_ASSERT_FLOAT_WITHIN(1e-3, Tilt::MAX_G, t.x());
}

void test_circle_box() {
  float cx = 5, cy = 5, nx, ny;
  TEST_ASSERT_FALSE(circleBox(cx, cy, 2, 10, 0, 4, 10, nx, ny));
  cx = 9;  // overlaps the box's left side by 1
  TEST_ASSERT_TRUE(circleBox(cx, cy, 2, 10, 0, 4, 10, nx, ny));
  TEST_ASSERT_FLOAT_WITHIN(1e-4, 8, cx);
  TEST_ASSERT_FLOAT_WITHIN(1e-4, -1, nx);
  cx = 11; cy = 5;  // centre inside: out the nearest side (left, 1 px)
  TEST_ASSERT_TRUE(circleBox(cx, cy, 2, 10, 0, 4, 10, nx, ny));
  TEST_ASSERT_FLOAT_WITHIN(1e-4, 8, cx);
  TEST_ASSERT_TRUE(circleHitsBox(9.5f, 5, 1, 10, 0, 4, 10));
  TEST_ASSERT_FALSE(circleHitsBox(8.5f, 5, 1, 10, 0, 4, 10));
}

// --- Stack --------------------------------------------------------------------------------

// Steps the slider until it's within `off` px of x (to the right of it when off > 0).
static void slideTo(Stack& s, float x) {
  for (int i = 0; i < 20000 && fabsf(s.x() - x) > 0.6f; i++) s.step(1);
}

void test_stack_cut_perfect_and_miss() {
  Stack s;
  s.reset(5);
  TEST_ASSERT_EQUAL(0, s.height());
  const float baseX = s.block(0).x;
  // 10 px off: 10 px cut off, the rest stays
  slideTo(s, baseX + 10);
  TEST_ASSERT_EQUAL((int)Stack::Drop::CUT, (int)s.drop());
  TEST_ASSERT_EQUAL(1, s.height());
  TEST_ASSERT_EQUAL(1, s.score);
  TEST_ASSERT_FLOAT_WITHIN(0.7f, Stack::START_W - 10, s.block(1).w);
  TEST_ASSERT_FLOAT_WITHIN(0.7f, baseX + 10, s.block(1).x);
  TEST_ASSERT_TRUE(s.debris());
  TEST_ASSERT_FLOAT_WITHIN(0.7f, 10, s.debW());
  TEST_ASSERT_TRUE(s.takeFx() & FX_SCORE);
  // perfect (within 3 px): snaps, +2
  slideTo(s, s.block(1).x + 2);
  TEST_ASSERT_EQUAL((int)Stack::Drop::PERFECT, (int)s.drop());
  TEST_ASSERT_EQUAL(3, s.score);
  TEST_ASSERT_FLOAT_WITHIN(1e-4, s.block(1).x, s.block(2).x);
  TEST_ASSERT_TRUE(s.takeFx() & FX_PERFECT);
  // three perfects in a row: a little wider
  const float w = s.block(2).w;
  slideTo(s, s.block(2).x);
  s.drop();
  slideTo(s, s.block(3).x);
  s.drop();
  TEST_ASSERT_EQUAL(3, s.combo());
  TEST_ASSERT_FLOAT_WITHIN(1e-3, w + Stack::GROW_PX, s.block(4).w);
  // faster as it grows
  TEST_ASSERT_TRUE(s.speed() > 90);
  // completely off the tower: over
  const Stack::Block top = s.block(s.height());
  TEST_ASSERT_TRUE(top.x - s.w() - 2 > s.lo());  // a miss can happen
  slideTo(s, top.x - s.w() - 2);
  TEST_ASSERT_EQUAL((int)Stack::Drop::MISS, (int)s.drop());
  TEST_ASSERT_TRUE(s.over);
  TEST_ASSERT_TRUE(s.takeFx() & FX_OVER);
  TEST_ASSERT_EQUAL((int)Stack::Drop::MISS, (int)s.drop());  // no more drops
}

void test_stack_slider_stays_in_bounds() {
  Stack s;
  s.reset(9);
  for (int i = 0; i < 5000; i++) {
    s.step(7);
    TEST_ASSERT_TRUE(s.x() >= s.lo() - 0.01f && s.x() <= s.hi() + 0.01f);
  }
}

// --- Flappy -------------------------------------------------------------------------------

// Flaps when it sinks below the next gap's middle (a simple autopilot).
static long flappyRun(uint32_t seed, bool pilot, uint32_t ms) {
  Flappy f;
  f.reset(seed);
  f.flap();
  for (uint32_t t = 0; t < ms && !f.over; t += 16) {
    if (pilot) {
      float target = f.H / 2.0f;
      for (int i = 0, best = 9999; i < Flappy::PIPES; i++) {
        const Flappy::Pipe& p = f.pipe(i);
        if (p.x + Flappy::PIPE_W > Flappy::BIRD_X - Flappy::R && p.x < best) { best = (int)p.x; target = p.gapY + 12; }
      }
      if (f.y() > target && f.vy() > 0) f.flap();
    }
    f.step(16);
  }
  return f.over ? -f.score - 1 : f.score;  // negative: crashed
}

void test_flappy() {
  Flappy f;
  f.reset(1);
  f.step(2000);  // not started: it hovers
  TEST_ASSERT_FALSE(f.over);
  TEST_ASSERT_FALSE(f.started());
  f.flap();
  TEST_ASSERT_TRUE(f.vy() < 0);
  TEST_ASSERT_TRUE(f.takeFx() & FX_ACTION);
  f.step(3000);  // no more flaps: down it goes
  TEST_ASSERT_TRUE(f.over);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, Flappy::GROUND - Flappy::R, f.y());
  // the autopilot gets through the pipes for a minute
  long s = flappyRun(3, true, 60000);
  TEST_ASSERT_TRUE_MESSAGE(s >= 20 || s <= -21, "autopilot scored too little");
  // deterministic
  TEST_ASSERT_EQUAL(flappyRun(77, true, 30000), flappyRun(77, true, 30000));
}

void test_flappy_gaps_fit() {
  Flappy f;
  f.reset(11);
  f.flap();
  for (int k = 0; k < 2000; k++) {
    f.flap();  // keeps it at the top: crashes into a pipe sooner or later; until then check
    f.step(16);
    for (int i = 0; i < Flappy::PIPES; i++) {
      const Flappy::Pipe& p = f.pipe(i);
      TEST_ASSERT_TRUE(p.gapY - p.gapH / 2 >= 30 && p.gapY + p.gapH / 2 <= Flappy::GROUND - 30);
      TEST_ASSERT_TRUE(p.gapH >= 54);
    }
    if (f.over) break;
  }
  TEST_ASSERT_TRUE(f.over);
}

// --- Dino ---------------------------------------------------------------------------------

void test_dino_jumps() {
  Dino d;
  d.reset(4);
  d.press();
  d.release();
  float tap = 0;
  for (int i = 0; i < 100; i++) { d.step(5); tap = fmaxf(tap, d.height()); }
  TEST_ASSERT_TRUE(d.onGround());
  d.press();  // held: higher
  float hold = 0;
  for (int i = 0; i < 160; i++) { d.step(5); hold = fmaxf(hold, d.height()); }
  d.release();
  TEST_ASSERT_TRUE(tap > 15 && tap < 40);
  TEST_ASSERT_TRUE(hold > tap * 1.8f);
  TEST_ASSERT_TRUE(hold < Dino::GROUND - Dino::BOT_H - 4);  // stays on the screen
  // a press in the air doesn't jump again
  d.press();
  d.step(50);
  float h = d.height();
  d.press();
  d.step(5);
  TEST_ASSERT_TRUE(d.height() < h + 5);
}

static long dinoRun(uint32_t seed, bool pilot, uint32_t ms) {
  Dino d;
  d.reset(seed);
  bool held = false;
  uint32_t heldMs = 0;
  for (uint32_t t = 0; t < ms && !d.over; t += 10) {
    if (pilot) {
      // the nearest one ahead: jump in time (hold for tall ones), don't jump drones
      float lead = 1e9;
      const Dino::Obstacle* o = nullptr;
      for (int i = 0; i < Dino::OBST; i++) {
        const Dino::Obstacle& x = d.obstacle(i);
        if (x.active && x.x + x.w > Dino::BOT_X - Dino::BOT_W / 2 && x.x < lead) { lead = x.x; o = &x; }
      }
      if (o && !o->fly && d.onGround() && o->x - (Dino::BOT_X + Dino::BOT_W / 2) < d.speed() * 0.12f + 4) {
        d.press();
        held = true;
        heldMs = 0;
      }
      if (held && (heldMs += 10) > (o && o->h > 20 ? 220u : 120u)) { d.release(); held = false; }
    }
    d.step(10);
  }
  return d.score;
}

void test_dino_runs() {
  Dino d;
  d.reset(8);
  for (int i = 0; i < 2000 && !d.over; i++) d.step(10);  // never jumps: hits the first bug
  TEST_ASSERT_TRUE(d.over);
  TEST_ASSERT_TRUE(d.takeFx() & FX_OVER);
  TEST_ASSERT_TRUE(d.score < 40);
  long s = dinoRun(8, true, 60000);
  TEST_ASSERT_TRUE_MESSAGE(s > 150, "the autopilot should get well past the first bugs");
  TEST_ASSERT_EQUAL(dinoRun(21, true, 40000), dinoRun(21, true, 40000));
  // it speeds up as it goes, up to its limit
  Dino e;
  e.reset(2);
  TEST_ASSERT_EQUAL_FLOAT(Dino::START_SPEED, e.speed());
  for (int i = 0; i < 300; i++) { e.step(10); if (e.over) break; }
  TEST_ASSERT_TRUE(e.speed() > Dino::START_SPEED && e.speed() <= Dino::MAX_SPEED);
}

// --- Reaction -----------------------------------------------------------------------------

void test_reaction_rounds() {
  Reaction r;
  r.reset(3);
  int expect[] = {200, 260, 330}, sum = 0;
  for (int k = 0; k < Reaction::ROUNDS; k++) {
    TEST_ASSERT_EQUAL((int)Reaction::Phase::WAIT, (int)r.phase());
    while (r.phase() == Reaction::Phase::WAIT) r.step(10);
    TEST_ASSERT_TRUE(r.takeFx() & FX_GO);
    r.step(expect[k] + 20);
    r.press(20);  // pressed 20 ms before this frame noticed: counts from the real press
    TEST_ASSERT_EQUAL(expect[k], r.time(k));
    sum += expect[k];
    TEST_ASSERT_EQUAL((int)Reaction::Phase::SHOWN, (int)r.phase());
    r.step(Reaction::SHOW_MS);
  }
  TEST_ASSERT_TRUE(r.over);
  TEST_ASSERT_FALSE(r.failed());
  TEST_ASSERT_EQUAL((sum + 1) / 3, r.score);
}

void test_reaction_false_start_and_slow() {
  Reaction r;
  r.reset(4);
  r.step(500);  // the wait is at least 1.2 s
  r.press();
  TEST_ASSERT_TRUE(r.over);
  TEST_ASSERT_TRUE(r.failed());
  TEST_ASSERT_EQUAL(0, r.score);
  TEST_ASSERT_FALSE(better(Id::REACTION, r.score, 300));

  Reaction s;
  s.reset(4);
  while (s.phase() == Reaction::Phase::WAIT) s.step(10);
  s.step(3000);  // asleep: counted as slow, on to the next round
  TEST_ASSERT_EQUAL((int)Reaction::Phase::SHOWN, (int)s.phase());
  TEST_ASSERT_EQUAL((int)Reaction::SLOW_MS, s.time(0));
}

void test_reaction_waits_vary() {
  uint32_t waits[5];
  for (int k = 0; k < 5; k++) {
    Reaction r;
    r.reset(100 + k);
    uint32_t ms = 0;
    while (r.phase() == Reaction::Phase::WAIT) { r.step(10); ms += 10; }
    waits[k] = ms;
    TEST_ASSERT_TRUE(ms >= Reaction::MIN_WAIT && ms <= Reaction::MAX_WAIT + 10);
  }
  bool differ = false;
  for (int k = 1; k < 5; k++) differ |= waits[k] != waits[0];
  TEST_ASSERT_TRUE(differ);  // you can't learn the rhythm
}

// --- Maze ---------------------------------------------------------------------------------

void test_maze_generation() {
  for (uint32_t seed = 1; seed < 40; seed++) {
    MazeGrid g;
    const int cols = 5 + seed % 6, rows = 8 + seed % 9;
    g.generate(cols, rows, seed);
    TEST_ASSERT_EQUAL(cols, g.cols);
    TEST_ASSERT_EQUAL(rows, g.rows);
    // a perfect maze: a tree, n - 1 passages, every cell reachable
    TEST_ASSERT_EQUAL(cols * rows - 1, g.openings());
    for (int r = 0; r < rows; r++)
      for (int c = 0; c < cols; c++) TEST_ASSERT_TRUE(g.distance(0, 0, c, r) >= 0);
    // the outside is closed, and walls agree from both sides
    for (int r = 0; r < rows; r++)
      for (int c = 0; c < cols; c++) {
        if (c == 0) TEST_ASSERT_TRUE(g.wall(c, r, MazeGrid::W));
        if (c == cols - 1) TEST_ASSERT_TRUE(g.wall(c, r, MazeGrid::E));
        if (r == 0) TEST_ASSERT_TRUE(g.wall(c, r, MazeGrid::N));
        if (r == rows - 1) TEST_ASSERT_TRUE(g.wall(c, r, MazeGrid::S));
        if (c + 1 < cols) TEST_ASSERT_EQUAL(g.wall(c, r, MazeGrid::E), g.wall(c + 1, r, MazeGrid::W));
        if (r + 1 < rows) TEST_ASSERT_EQUAL(g.wall(c, r, MazeGrid::S), g.wall(c, r + 1, MazeGrid::N));
      }
  }
  // the same seed, the same maze; another seed, another
  MazeGrid a, b, c;
  a.generate(8, 12, 99);
  b.generate(8, 12, 99);
  c.generate(8, 12, 100);
  TEST_ASSERT_EQUAL_MEMORY(a.cell, b.cell, sizeof a.cell);
  TEST_ASSERT_TRUE(memcmp(a.cell, c.cell, sizeof a.cell) != 0);
  // out of range sizes are clamped
  MazeGrid big;
  big.generate(99, 99, 1);
  TEST_ASSERT_EQUAL(MazeGrid::MAX_COLS, big.cols);
  TEST_ASSERT_EQUAL(MazeGrid::MAX_ROWS, big.rows);
  TEST_ASSERT_EQUAL(MazeGrid::MAX - 1, big.openings());
}

void test_maze_levels_fit_the_screen() {
  Maze m;
  m.reset(5);
  for (int level = 1; level <= 12; level++) {
    const MazeGrid& g = m.grid();
    TEST_ASSERT_TRUE(m.originX() >= 0 && m.originX() + g.cols * m.cellPx() <= Maze::W);
    TEST_ASSERT_TRUE(m.originY() >= Maze::TOP && m.originY() + g.rows * m.cellPx() <= Maze::H);
    TEST_ASSERT_TRUE(m.cellPx() >= 12);
    TEST_ASSERT_TRUE(m.ballR() * 2 < m.cellPx() - Maze::WALL);
    // into the exit: the next level, time back, a point
    const uint32_t left = m.timeLeft();
    m.place(m.originX() + (g.cols - 0.5f) * m.cellPx(), m.originY() + (g.rows - 0.5f) * m.cellPx());
    m.step(10, 0, 0);
    TEST_ASSERT_EQUAL(level + 1, m.level());
    TEST_ASSERT_EQUAL(level, m.score);
    TEST_ASSERT_TRUE(m.timeLeft() > left || m.timeLeft() == Maze::MAX_MS);
    TEST_ASSERT_TRUE(m.takeFx() & FX_LEVEL);
  }
}

// The ball never goes through a wall: from cell to cell only through openings, however hard
// it's tilted (and the tilt keeps changing).
void test_maze_walls_hold() {
  Maze m;
  m.reset(12);
  m.step(0, 0, 0);
  int lastC = 0, lastR = 0;
  Rng rng(3);
  float tx = 0.6f, ty = 0.6f;
  for (int i = 0; i < 20000 && !m.over; i++) {
    if (i % 40 == 0) { tx = rng.between(-0.6f, 0.6f); ty = rng.between(-0.6f, 0.6f); }
    const int level = m.level();
    m.step(i % 3 == 0 ? 33 : 16, tx, ty);
    if (m.level() != level) { lastC = lastR = 0; continue; }
    const MazeGrid& g = m.grid();
    const int c = (int)floorf((m.ballX() - m.originX()) / m.cellPx()), r = (int)floorf((m.ballY() - m.originY()) / m.cellPx());
    TEST_ASSERT_TRUE(c >= 0 && c < g.cols && r >= 0 && r < g.rows);
    if (c != lastC || r != lastR) {
      TEST_ASSERT_TRUE_MESSAGE(abs(c - lastC) + abs(r - lastR) == 1, "jumped more than a cell");
      if (c > lastC) TEST_ASSERT_FALSE(g.wall(lastC, lastR, MazeGrid::E));
      if (c < lastC) TEST_ASSERT_FALSE(g.wall(lastC, lastR, MazeGrid::W));
      if (r > lastR) TEST_ASSERT_FALSE(g.wall(lastC, lastR, MazeGrid::S));
      if (r < lastR) TEST_ASSERT_FALSE(g.wall(lastC, lastR, MazeGrid::N));
      lastC = c;
      lastR = r;
    }
  }
}

void test_maze_time_runs_out() {
  Maze m;
  m.reset(1);
  m.step(Maze::START_MS - 10, 0, 0);
  TEST_ASSERT_FALSE(m.over);
  m.step(20, 0, 0);
  TEST_ASSERT_TRUE(m.over);
  TEST_ASSERT_EQUAL(0, m.score);
  TEST_ASSERT_TRUE(m.takeFx() & FX_OVER);
}

void test_maze_rolls_downhill() {
  Maze m;
  m.reset(1);
  const float x0 = m.ballX(), y0 = m.ballY();
  // level 1 starts in the top left cell; tilting right or down moves it unless a wall's there
  m.step(300, m.grid().wall(0, 0, MazeGrid::E) ? 0 : 0.4f, m.grid().wall(0, 0, MazeGrid::E) ? 0.4f : 0);
  TEST_ASSERT_TRUE(m.ballX() > x0 + 2 || m.ballY() > y0 + 2);
}

// --- Balance ------------------------------------------------------------------------------

static long balanceRun(uint32_t seed, bool pilot, uint32_t ms, Balance& b) {
  b.reset(seed);
  for (uint32_t t = 0; t < ms && !b.over; t += 20) {
    // lean against the beam, and back towards the middle
    float tx = pilot ? -sinf(b.angle()) - b.pos() * 0.004f - b.vel() * 0.004f : 0;
    tx = tx > 0.6f ? 0.6f : tx < -0.6f ? -0.6f : tx;
    b.step(20, tx);
  }
  return b.score;
}

void test_balance() {
  Balance b;
  balanceRun(6, false, 120000, b);  // no hands: the wind tips it off sooner or later
  TEST_ASSERT_TRUE(b.over);
  TEST_ASSERT_TRUE(b.takeFx() & FX_OVER);
  const uint32_t lasted = b.t;
  b.step(1000, 0);  // ...and the ball falls
  TEST_ASSERT_TRUE(b.fall() > 50);
  Balance p;
  long s = balanceRun(6, true, 90000, p);
  TEST_ASSERT_FALSE_MESSAGE(p.over, "the autopilot should keep it on");
  TEST_ASSERT_TRUE(s > 600);  // mostly in the middle: ~10 points a second
  TEST_ASSERT_TRUE(p.t > lasted);
  Balance q;
  TEST_ASSERT_EQUAL(balanceRun(9, true, 30000, q), balanceRun(9, true, 30000, p));
  // the wind gets stronger: later leans are bigger (on the average)
  Balance w;
  w.reset(3);
  float early = 0, late = 0;
  for (int i = 0; i < 1500; i++) { w.step(20, -sinf(w.angle()) - w.pos() * 0.004f); if (i < 750) early = fmaxf(early, fabsf(w.angle())); else late = fmaxf(late, fabsf(w.angle())); }
  TEST_ASSERT_TRUE(late > early);
}

// --- Catch --------------------------------------------------------------------------------

void test_catch_rules() {
  Catch c;
  c.reset(2);
  c.step(10, 0);  // (nothing spawns yet)
  const float bx = c.basketX();
  c.drop(bx, Catch::BASKET_Y - 30, 100, Catch::Kind::COIN);
  c.step(400, 0);
  TEST_ASSERT_EQUAL(1, c.score);
  TEST_ASSERT_TRUE(c.takeFx() & FX_SCORE);
  c.drop(bx, Catch::BASKET_Y - 30, 100, Catch::Kind::STAR);
  c.step(400, 0);
  TEST_ASSERT_EQUAL(4, c.score);
  TEST_ASSERT_TRUE(c.takeFx() & FX_BONUS);
  c.drop(bx, Catch::BASKET_Y - 30, 100, Catch::Kind::BOMB);
  c.step(400, 0);
  TEST_ASSERT_EQUAL(Catch::LIVES - 1, c.lives());
  TEST_ASSERT_TRUE(c.takeFx() & FX_HIT);
  // a coin missed costs a heart; a bomb missed doesn't
  c.drop(5, Catch::H - 20, 200, Catch::Kind::COIN);
  c.drop(Catch::W - 5, Catch::H - 20, 200, Catch::Kind::BOMB);
  c.step(300, 0);
  TEST_ASSERT_EQUAL(Catch::LIVES - 2, c.lives());
}

void test_catch_basket_follows_tilt() {
  Catch c;
  c.reset(2);
  const float x0 = c.basketX();
  c.step(200, 0.4f);
  TEST_ASSERT_TRUE(c.basketX() > x0 + 20);
  c.step(3000, 0.6f);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, Catch::W - Catch::BASKET_W / 2, c.basketX());  // stops at the edge
  c.step(3000, -0.6f);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, Catch::BASKET_W / 2, c.basketX());
}

static long catchRun(uint32_t seed, uint32_t ms, Catch& c) {
  c.reset(seed);
  for (uint32_t t = 0; t < ms && !c.over; t += 20) {
    // go for the lowest coin or star; ignore bombs
    float target = c.basketX(), low = -1;
    for (int i = 0; i < Catch::ITEMS; i++) {
      const Catch::Item& it = c.item(i);
      if (it.active && it.kind != Catch::Kind::BOMB && it.y > low && it.y < Catch::BASKET_Y) { low = it.y; target = it.x; }
    }
    float tx = (target - c.basketX()) * 0.02f;
    tx = tx > 0.6f ? 0.6f : tx < -0.6f ? -0.6f : tx;
    c.step(20, tx);
  }
  return c.score;
}

void test_catch_plays() {
  Catch c;
  long s = catchRun(4, 120000, c);
  TEST_ASSERT_TRUE(s > 30);
  Catch idle;
  idle.reset(4);
  for (int i = 0; i < 6000 && !idle.over; i++) idle.step(20, 0);
  TEST_ASSERT_TRUE(idle.over);  // never moving: the coins that fall elsewhere use up the hearts
  Catch a, b;
  TEST_ASSERT_EQUAL(catchRun(5, 40000, a), catchRun(5, 40000, b));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_list);
  RUN_TEST(test_better);
  RUN_TEST(test_rng);
  RUN_TEST(test_screen_tilt);
  RUN_TEST(test_tilt_calibration_and_smoothing);
  RUN_TEST(test_circle_box);
  RUN_TEST(test_stack_cut_perfect_and_miss);
  RUN_TEST(test_stack_slider_stays_in_bounds);
  RUN_TEST(test_flappy);
  RUN_TEST(test_flappy_gaps_fit);
  RUN_TEST(test_dino_jumps);
  RUN_TEST(test_dino_runs);
  RUN_TEST(test_reaction_rounds);
  RUN_TEST(test_reaction_false_start_and_slow);
  RUN_TEST(test_reaction_waits_vary);
  RUN_TEST(test_maze_generation);
  RUN_TEST(test_maze_levels_fit_the_screen);
  RUN_TEST(test_maze_walls_hold);
  RUN_TEST(test_maze_time_runs_out);
  RUN_TEST(test_maze_rolls_downhill);
  RUN_TEST(test_balance);
  RUN_TEST(test_catch_rules);
  RUN_TEST(test_catch_basket_follows_tilt);
  RUN_TEST(test_catch_plays);
  return UNITY_END();
}
