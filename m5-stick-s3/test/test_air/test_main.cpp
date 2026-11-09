// The air mouse / presenter / media remote logic (logic/air_motion, logic/remote) from
// synthetic IMU samples at 100 Hz and button timelines.
#include <unity.h>
#include <math.h>
#include <stdlib.h>
#include <vector>
#include "logic/air_motion.h"
#include "logic/remote.h"

using namespace airmotion;
using remote::Btn;

void setUp() {}
void tearDown() {}

static const float FLAT[3] = {0, 0, 1};     // screen up, like a TV remote
static const float UPRIGHT[3] = {0, 1, 0};  // standing up, screen towards you
static const float X_UP[3] = {1, 0, 0};     // on its side, right edge up (landscape, rotation 1)
constexpr float DT = 0.01f;

static Rates ratesFor(const float a[3], const float g[3], int side) {
  Frame f;
  f.feedAccel(a, DT);
  return f.rates(g, side);
}

// --- Frame -----------------------------------------------------------------------------

void test_frame_remote_hold() {
  const float right[3] = {0, 0, -50};  // turning right = clockwise seen from above = - around z
  Rates r = ratesFor(FLAT, right, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 50, r.yaw);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 0, r.pitch);
  const float tipUp[3] = {30, 0, 0};  // the tip (y) towards up (z): + around x
  r = ratesFor(FLAT, tipUp, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, -30, r.pitch);  // pointer up
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 0, r.yaw);
  const float twist[3] = {0, 40, 0};  // around the stick: roll only, the pointer stays
  r = ratesFor(FLAT, twist, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 0, r.yaw);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 0, r.pitch);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 40, r.roll);
}

void test_frame_upright_hold() {
  const float right[3] = {0, -50, 0};  // the long axis is vertical now: turning it = yaw
  Rates r = ratesFor(UPRIGHT, right, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 50, r.yaw);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 0, r.pitch);
  const float tipBack[3] = {30, 0, 0};
  r = ratesFor(UPRIGHT, tipBack, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, -30, r.pitch);
}

void test_frame_tilted_hold_is_the_same_world() {
  // Held halfway between remote and upright: turning around gravity is still pure yaw
  const float a[3] = {0, 0.7071f, 0.7071f};
  const float g[3] = {0, -0.7071f * 40, -0.7071f * 40};
  Rates r = ratesFor(a, g, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.05f, 40, r.yaw);
  TEST_ASSERT_FLOAT_WITHIN(0.05f, 0, r.pitch);
}

void test_frame_landscape() {
  const float right[3] = {-50, 0, 0};  // x is up
  Rates r = ratesFor(X_UP, right, 1);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 50, r.yaw);
  const float tipUp[3] = {0, -30, 0};  // around the screen's right (-y)
  r = ratesFor(X_UP, tipUp, 1);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, -30, r.pitch);
  // Same hold while the screen still says portrait: the fallback finds the same axes
  Rates p = ratesFor(X_UP, tipUp, 0);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, -30, p.pitch);
}

void test_frame_portrait_flipped() {
  const float down[3] = {0, -1, 0};  // upside down, screen towards you
  const float right[3] = {0, 50, 0};
  Rates r = ratesFor(down, right, 2);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 50, r.yaw);
  const float tip[3] = {-30, 0, 0};  // around the flipped screen's right (-x)
  r = ratesFor(down, tip, 2);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, -30, r.pitch);
}

void test_frame_gravity_follows_a_turn() {
  Frame f;
  f.feedAccel(FLAT, DT);
  for (int i = 0; i < 200; i++) f.feedAccel(UPRIGHT, DT);  // 2 s held upright
  float u[3];
  f.up(u);
  TEST_ASSERT_FLOAT_WITHIN(0.02f, 1, u[1]);
}

// --- Stillness ----------------------------------------------------------------------------

static float noise(float amp) { return amp * ((rand() % 2001) / 1000.0f - 1.0f); }

void test_still_on_the_desk_learns_the_bias() {
  srand(1);
  Stillness s;
  const float bias[3] = {0.6f, -0.9f, 0.35f};
  uint32_t t = 1000;
  bool still = false;
  for (int i = 0; i < 150; i++, t += 10) {
    float g[3] = {bias[0] + noise(0.1f), bias[1] + noise(0.1f), bias[2] + noise(0.1f)};
    float a[3] = {noise(0.003f), noise(0.003f), 1 + noise(0.003f)};
    still = s.feed(t, a, g, DT);
  }
  TEST_ASSERT_TRUE(still);
  TEST_ASSERT_TRUE(s.calibrated());
  float b[3];
  s.bias(b);
  for (int i = 0; i < 3; i++) TEST_ASSERT_FLOAT_WITHIN(0.1f, bias[i], b[i]);
  TEST_ASSERT_TRUE(s.stillFor(t) > 0);
}

void test_still_needs_the_time() {
  Stillness s;
  const float g[3] = {0, 0, 0};
  uint32_t t = 1000;
  for (int i = 0; i < 40; i++, t += 10) TEST_ASSERT_FALSE(s.feed(t, FLAT, g, DT));  // 0.4 s
  for (int i = 0; i < 40; i++, t += 10) s.feed(t, FLAT, g, DT);
  TEST_ASSERT_TRUE(s.still());
}

void test_hand_is_never_still() {
  srand(2);
  Stillness s;
  uint32_t t = 1000;
  int stillSamples = 0;
  for (int i = 0; i < 500; i++, t += 10) {  // 5 s of a hand: tremor + a slow wander
    float ph = i * DT;
    float g[3] = {3 * sinf(ph * 50) + noise(1.5f), 2.5f * sinf(ph * 37) + noise(1.5f), 3 * sinf(ph * 23) + noise(1.5f)};
    float a[3] = {0.03f * sinf(ph * 9), 0.02f * sinf(ph * 7), 1};
    stillSamples += s.feed(t, a, g, DT);
  }
  TEST_ASSERT_EQUAL(0, stillSamples);
}

void test_slow_steady_turn_is_not_bias() {
  Stillness s;
  uint32_t t = 1000;
  const float zero[3] = {0, 0, 0};
  for (int i = 0; i < 100; i++, t += 10) s.feed(t, FLAT, zero, DT);  // calibrated at rest
  TEST_ASSERT_TRUE(s.still());
  const float slow[3] = {0, 0, 5};  // a slow 5 deg/s turn on the desk (gravity doesn't change)
  bool any = false;
  for (int i = 0; i < 300; i++, t += 10) any |= s.feed(t, FLAT, slow, DT);
  TEST_ASSERT_FALSE(any);
  float b[3];
  s.bias(b);
  TEST_ASSERT_FLOAT_WITHIN(0.1f, 0, b[2]);  // the turn didn't leak into the bias
}

void test_still_ends_on_a_lift() {
  Stillness s;
  uint32_t t = 1000;
  const float zero[3] = {0, 0, 0};
  for (int i = 0; i < 100; i++, t += 10) s.feed(t, FLAT, zero, DT);
  TEST_ASSERT_TRUE(s.still());
  const float lift[3] = {0.05f, -0.1f, 1.2f};
  const float spin[3] = {20, 5, -3};
  TEST_ASSERT_FALSE(s.feed(t, lift, spin, DT));
}

// --- Pointer ------------------------------------------------------------------------------

struct Totals { long dx = 0, dy = 0; int maxStep = 0; };

static Totals run(Pointer& p, float yaw, float pitch, int samples, uint32_t& t) {
  Totals tot;
  for (int i = 0; i < samples; i++, t += 10) {
    int dx, dy;
    p.feed(t, DT, yaw, pitch, dx, dy);
    tot.dx += dx;
    tot.dy += dy;
    if (abs(dx) > tot.maxStep) tot.maxStep = abs(dx);
    if (abs(dy) > tot.maxStep) tot.maxStep = abs(dy);
  }
  return tot;
}

void test_pointer_ignores_tremor() {
  Pointer p;
  p.configure(5, false, false);
  uint32_t t = 1000;
  Totals tot = run(p, 1.5f, -1.2f, 300, t);  // under the dead zone, 3 s
  TEST_ASSERT_EQUAL(0, tot.dx);
  TEST_ASSERT_EQUAL(0, tot.dy);
}

void test_pointer_moves_with_yaw_and_pitch() {
  Pointer p;
  p.configure(5, false, false);
  uint32_t t = 1000;
  Totals tot = run(p, 20, 0, 100, t);  // 20 deg/s for 1 s
  TEST_ASSERT_TRUE(tot.dx > 60);
  TEST_ASSERT_TRUE(tot.dx < 400);
  TEST_ASSERT_EQUAL(0, tot.dy);
  Pointer q;
  q.configure(5, false, false);
  Totals down = run(q, 0, 20, 100, t);
  TEST_ASSERT_EQUAL(0, down.dx);
  TEST_ASSERT_TRUE(down.dy > 60);
}

void test_pointer_is_symmetric() {
  Pointer a, b;
  a.configure(5, false, false);
  b.configure(5, false, false);
  uint32_t t1 = 1000, t2 = 1000;
  Totals r = run(a, 35, 0, 80, t1), l = run(b, -35, 0, 80, t2);
  TEST_ASSERT_INT_WITHIN(1, r.dx, -l.dx);
}

void test_pointer_sensitivity_and_invert() {
  uint32_t t = 1000;
  Pointer lo, hi, inv;
  lo.configure(2, false, false);
  hi.configure(9, false, false);
  inv.configure(5, true, true);
  Totals a = run(lo, 30, 0, 100, t), b = run(hi, 30, 0, 100, t);
  TEST_ASSERT_TRUE(b.dx > a.dx * 2);
  Totals c = run(inv, 30, 30, 100, t);
  TEST_ASSERT_TRUE(c.dx < 0);
  TEST_ASSERT_TRUE(c.dy < 0);
  TEST_ASSERT_TRUE(gainFor(1) < gainFor(5) && gainFor(5) < gainFor(10));
}

void test_pointer_accelerates() {
  // twice the speed for the same angle gets further: fast = across the screen, slow = precise
  uint32_t t = 1000;
  Pointer slow, fast;
  slow.configure(5, false, false);
  fast.configure(5, false, false);
  Totals s = run(slow, 20, 0, 200, t);  // 40 deg in 2 s
  Totals f = run(fast, 160, 0, 25, t);  // 40 deg in 0.25 s
  TEST_ASSERT_TRUE(f.dx > s.dx * 2);
  TEST_ASSERT_TRUE(accelFactor(5) == 1.0f);
  TEST_ASSERT_TRUE(accelFactor(300) > 3.0f);
}

void test_pointer_keeps_sub_counts() {
  // 0.4 counts a sample still adds up instead of rounding to nothing
  Pointer p;
  p.configure(1, false, false);
  uint32_t t = 1000;
  Totals tot = run(p, 4, 0, 500, t);  // just over the dead zone, 5 s
  TEST_ASSERT_TRUE(tot.dx > 10);
  TEST_ASSERT_TRUE(tot.maxStep <= 1);
}

void test_pointer_clamps_a_report() {
  Pointer p;
  p.configure(10, false, false);
  uint32_t t = 1000;
  int dx, dy;
  for (int i = 0; i < 10; i++, t += 10) p.feed(t, DT, 900, 0, dx, dy);
  p.feed(t, 0.2f, 900, 0, dx, dy);  // a long stall: one big sample
  TEST_ASSERT_EQUAL(127, dx);
  p.giveBack(dx, dy);
  int dx2, dy2;
  p.feed(t + 10, DT, 0, 0, dx2, dy2);
  TEST_ASSERT_EQUAL(127, dx2);  // what couldn't be sent comes next
}

void test_pointer_freeze_after_click() {
  Pointer p;
  p.configure(5, false, false);
  uint32_t t = 1000;
  p.freeze(t);
  Totals tot = run(p, 30, 10, 12, t);  // 120 ms of a hand settling after the press
  TEST_ASSERT_EQUAL(0, tot.dx);
  TEST_ASSERT_EQUAL(0, tot.dy);
  Totals after = run(p, 30, 0, 50, t);
  TEST_ASSERT_TRUE(after.dx > 0);
  // a clear move breaks the freeze at once
  p.freeze(t);
  Totals big = run(p, 200, 0, 5, t);
  TEST_ASSERT_TRUE(big.dx > 0);
}

// --- Scroller -----------------------------------------------------------------------------

static void tilt(Scroller& s, float pitchDps, int samples, long& wheel, long& pan) {
  for (int i = 0; i < samples; i++) {
    int w, p;
    s.feed(DT, 0, pitchDps, w, p);
    wheel += w;
    pan += p;
  }
}

void test_scroll_dead_zone() {
  Scroller s;
  s.configure(5, false);
  s.begin();
  long w = 0, p = 0;
  tilt(s, -30, 10, w, p);  // tip up 3 deg
  tilt(s, 0, 200, w, p);   // and hold 2 s
  TEST_ASSERT_EQUAL(0, w);
  TEST_ASSERT_EQUAL(0, p);
}

void test_scroll_tilt_up_scrolls_up_faster_further() {
  Scroller s;
  s.configure(5, false);
  s.begin();
  long w = 0, p = 0;
  tilt(s, -100, 15, w, p);  // tip up 15 deg
  long w0 = w;
  tilt(s, 0, 100, w, p);    // hold 1 s
  long at15 = w - w0;
  TEST_ASSERT_TRUE(at15 > 2);
  tilt(s, -100, 15, w, p);  // 30 deg
  long w1 = w;
  tilt(s, 0, 100, w, p);
  TEST_ASSERT_TRUE(w - w1 > at15 * 2);
  TEST_ASSERT_EQUAL(0, p);
  TEST_ASSERT_TRUE(s.rateFor(-20) < 0);
}

void test_scroll_invert_and_speed() {
  Scroller a, b;
  a.configure(5, true);
  b.configure(9, false);
  a.begin();
  b.begin();
  long wa = 0, pa = 0, wb = 0, pb = 0;
  tilt(a, -100, 20, wa, pa);
  tilt(a, 0, 100, wa, pa);
  tilt(b, -100, 20, wb, pb);
  tilt(b, 0, 100, wb, pb);
  TEST_ASSERT_TRUE(wa < 0);
  TEST_ASSERT_TRUE(wb > -wa);
}

void test_scroll_sideways_needs_more() {
  Scroller s;
  s.configure(5, false);
  s.begin();
  int w, p;
  long pan = 0;
  for (int i = 0; i < 6; i++) { s.feed(DT, 100, 0, w, p); pan += p; }    // 6 deg right
  for (int i = 0; i < 100; i++) { s.feed(DT, 0, 0, w, p); pan += p; }
  TEST_ASSERT_EQUAL(0, pan);
  for (int i = 0; i < 20; i++) { s.feed(DT, 100, 0, w, p); pan += p; }   // 26 deg
  for (int i = 0; i < 100; i++) { s.feed(DT, 0, 0, w, p); pan += p; }
  TEST_ASSERT_TRUE(pan > 0);
}

// --- Knob ----------------------------------------------------------------------------------

static std::vector<KnobEvent> knob(Knob& k, uint32_t& t, float yaw, float roll, int samples) {
  std::vector<KnobEvent> out;
  for (int i = 0; i < samples; i++, t += 10) {
    Rates r;
    r.yaw = yaw;
    r.roll = roll;
    KnobEvent e = k.feed(t, DT, r);
    if (e != KnobEvent::NONE) out.push_back(e);
  }
  return out;
}

void test_knob_twist_steps_volume() {
  Knob k;
  uint32_t t = 1000;
  auto up = knob(k, t, 0, 60, 80);  // 48 deg clockwise
  TEST_ASSERT_EQUAL(3, (int)up.size());
  for (auto e : up) TEST_ASSERT_TRUE(e == KnobEvent::VOLUME_UP);
  knob(k, t, 0, 0, 60);
  auto down = knob(k, t, 0, -90, 35);  // 31 deg back
  TEST_ASSERT_EQUAL(2, (int)down.size());
  for (auto e : down) TEST_ASSERT_TRUE(e == KnobEvent::VOLUME_DOWN);
}

void test_knob_ignores_drift_and_partial_turns() {
  Knob k;
  uint32_t t = 1000;
  TEST_ASSERT_EQUAL(0, (int)knob(k, t, 0, 15, 300).size());  // slow drift, 45 deg in 3 s
  TEST_ASSERT_EQUAL(0, (int)knob(k, t, 0, 60, 20).size());   // 12 deg
  knob(k, t, 0, 0, 60);                                       // a pause: the part-step resets
  TEST_ASSERT_EQUAL(0, (int)knob(k, t, 0, 60, 20).size());   // 12 deg again: still no step
}

void test_knob_flick_next_and_previous() {
  Knob k;
  uint32_t t = 1000;
  std::vector<KnobEvent> ev = knob(k, t, 400, 30, 6);  // a flick right
  auto back = knob(k, t, -350, -20, 6);                // and the wrist swinging back
  ev.insert(ev.end(), back.begin(), back.end());
  auto rest = knob(k, t, 0, 0, 80);
  ev.insert(ev.end(), rest.begin(), rest.end());
  TEST_ASSERT_EQUAL(1, (int)ev.size());
  TEST_ASSERT_TRUE(ev[0] == KnobEvent::NEXT);
  auto left = knob(k, t, -400, 0, 6);
  TEST_ASSERT_EQUAL(1, (int)left.size());
  TEST_ASSERT_TRUE(left[0] == KnobEvent::PREVIOUS);
}

// --- Buttons ------------------------------------------------------------------------------

struct Press {
  remote::Buttons b;
  uint32_t t = 1000;
  std::vector<Btn> out;
  explicit Press(remote::ButtonTiming tm = {}) { b.configure(tm); b.reset(false, false); }
  void hold(bool a, bool bb, uint32_t ms) {
    for (uint32_t e = 0; e < ms; e += 10) {
      t += 10;
      b.feed(t, a, bb);
      for (Btn x = b.next(); x != Btn::NONE; x = b.next()) out.push_back(x);
    }
  }
  bool has(Btn x) const { for (auto y : out) if (y == x) return true; return false; }
  int count(Btn x) const { int n = 0; for (auto y : out) n += y == x; return n; }
};

void test_buttons_a_click() {
  Press p;
  p.hold(true, false, 40);
  p.hold(false, false, 100);
  TEST_ASSERT_EQUAL(3, (int)p.out.size());
  TEST_ASSERT_TRUE(p.out[0] == Btn::A_PRESS);
  TEST_ASSERT_TRUE(p.out[1] == Btn::A_RELEASE);
  TEST_ASSERT_TRUE(p.out[2] == Btn::A_CLICK);
}

void test_buttons_a_press_waits_the_chord_window_then_holds() {
  Press p;
  p.hold(true, false, 60);
  TEST_ASSERT_EQUAL(0, (int)p.out.size());  // still inside the chord window
  p.hold(true, false, 20);
  TEST_ASSERT_TRUE(p.has(Btn::A_PRESS));
  p.hold(true, false, 400);
  TEST_ASSERT_EQUAL(1, p.count(Btn::A_HOLD));
  p.hold(false, false, 20);
  TEST_ASSERT_TRUE(p.has(Btn::A_RELEASE));
  TEST_ASSERT_FALSE(p.has(Btn::A_CLICK));
}

void test_buttons_b_click_waits_for_a_double() {
  remote::ButtonTiming tm;
  tm.doubleMsB = 300;
  Press p(tm);
  p.hold(false, true, 50);
  p.hold(false, false, 200);
  TEST_ASSERT_FALSE(p.has(Btn::B_CLICK));  // could still become a double
  p.hold(false, false, 200);
  TEST_ASSERT_EQUAL(1, p.count(Btn::B_CLICK));
  TEST_ASSERT_FALSE(p.has(Btn::B_DOUBLE));
}

void test_buttons_b_double() {
  remote::ButtonTiming tm;
  tm.doubleMsB = 300;
  Press p(tm);
  p.hold(false, true, 50);
  p.hold(false, false, 120);
  p.hold(false, true, 50);
  TEST_ASSERT_EQUAL(1, p.count(Btn::B_DOUBLE));  // reported on the second press, no waiting
  p.hold(false, false, 600);
  TEST_ASSERT_EQUAL(0, p.count(Btn::B_CLICK));
  TEST_ASSERT_EQUAL(0, p.count(Btn::B_HOLD));
}

void test_buttons_b_hold_isnt_a_click() {
  remote::ButtonTiming tm;
  tm.doubleMsB = 300;
  Press p(tm);
  p.hold(false, true, 500);
  TEST_ASSERT_EQUAL(1, p.count(Btn::B_HOLD));
  p.hold(false, false, 500);
  TEST_ASSERT_EQUAL(0, p.count(Btn::B_CLICK));
}

void test_buttons_chord_tap_and_hold() {
  Press p;
  p.hold(true, false, 30);
  p.hold(true, true, 200);
  p.hold(false, true, 50);
  p.hold(false, false, 50);
  TEST_ASSERT_EQUAL(1, (int)p.out.size());
  TEST_ASSERT_TRUE(p.out[0] == Btn::CHORD);

  Press q;
  q.hold(true, true, 1300);
  TEST_ASSERT_EQUAL(1, q.count(Btn::CHORD_HOLD));
  q.hold(false, false, 50);
  TEST_ASSERT_EQUAL(1, (int)q.out.size());  // no CHORD after the hold, no A / B events
}

void test_buttons_late_b_is_its_own_press() {
  Press p;
  p.hold(true, false, 200);   // A is down (dragging)
  p.hold(true, true, 50);     // B joins later: not a chord
  p.hold(true, false, 50);
  p.hold(false, false, 50);
  TEST_ASSERT_TRUE(p.has(Btn::A_PRESS));
  TEST_ASSERT_TRUE(p.has(Btn::B_PRESS));
  TEST_ASSERT_TRUE(p.has(Btn::B_CLICK));
  TEST_ASSERT_FALSE(p.has(Btn::CHORD));
}

void test_buttons_press_that_opened_the_mode_is_ignored() {
  remote::Buttons b;
  b.reset(true, false);
  uint32_t t = 1000;
  for (int i = 0; i < 50; i++) b.feed(t += 10, true, false);
  b.feed(t += 10, false, false);
  TEST_ASSERT_TRUE(b.next() == Btn::NONE);
  for (int i = 0; i < 10; i++) b.feed(t += 10, true, false);  // a new press counts
  TEST_ASSERT_TRUE(b.next() == Btn::A_PRESS);
}

// --- Talk timer ---------------------------------------------------------------------------

void test_timer_runs_pauses_and_warns() {
  remote::TalkTimer tm;
  TEST_ASSERT_TRUE(tm.phase(0, 10) == remote::Phase::IDLE);
  tm.start(1000);
  TEST_ASSERT_EQUAL(0, tm.elapsedMs(1000));
  TEST_ASSERT_EQUAL(60000, tm.elapsedMs(61000));
  tm.pause(61000);
  TEST_ASSERT_EQUAL(60000, tm.elapsedMs(500000));
  tm.start(500000);
  TEST_ASSERT_EQUAL(70000, tm.elapsedMs(510000));
  // 10 min target: the last minute warns, then over
  uint32_t base = 500000 - 60000;  // elapsed = now - base (after the pause)
  TEST_ASSERT_TRUE(tm.phase(base + 8 * 60000, 10) == remote::Phase::RUNNING);
  TEST_ASSERT_TRUE(tm.phase(base + 9 * 60000 + 1, 10) == remote::Phase::WARN);
  TEST_ASSERT_TRUE(tm.phase(base + 10 * 60000, 10) == remote::Phase::OVER);
  TEST_ASSERT_TRUE(tm.phase(base + 10 * 60000, 0) == remote::Phase::RUNNING);
  // 60 min: warns for the last 5 (10 % = 6, capped)
  remote::TalkTimer h;
  h.start(0);
  TEST_ASSERT_TRUE(h.phase(54 * 60000, 60) == remote::Phase::RUNNING);
  TEST_ASSERT_TRUE(h.phase(55 * 60000, 60) == remote::Phase::WARN);
}

void test_format_clock() {
  char b[16];
  remote::formatClock(247000, b, sizeof b);
  TEST_ASSERT_EQUAL_STRING("4:07", b);
  remote::formatClock(0, b, sizeof b);
  TEST_ASSERT_EQUAL_STRING("0:00", b);
  remote::formatClock(3723000, b, sizeof b);
  TEST_ASSERT_EQUAL_STRING("1:02:03", b);
  remote::formatClock(-65000, b, sizeof b);
  TEST_ASSERT_EQUAL_STRING("-1:05", b);
}

int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(test_frame_remote_hold);
  RUN_TEST(test_frame_upright_hold);
  RUN_TEST(test_frame_tilted_hold_is_the_same_world);
  RUN_TEST(test_frame_landscape);
  RUN_TEST(test_frame_portrait_flipped);
  RUN_TEST(test_frame_gravity_follows_a_turn);
  RUN_TEST(test_still_on_the_desk_learns_the_bias);
  RUN_TEST(test_still_needs_the_time);
  RUN_TEST(test_hand_is_never_still);
  RUN_TEST(test_slow_steady_turn_is_not_bias);
  RUN_TEST(test_still_ends_on_a_lift);
  RUN_TEST(test_pointer_ignores_tremor);
  RUN_TEST(test_pointer_moves_with_yaw_and_pitch);
  RUN_TEST(test_pointer_is_symmetric);
  RUN_TEST(test_pointer_sensitivity_and_invert);
  RUN_TEST(test_pointer_accelerates);
  RUN_TEST(test_pointer_keeps_sub_counts);
  RUN_TEST(test_pointer_clamps_a_report);
  RUN_TEST(test_pointer_freeze_after_click);
  RUN_TEST(test_scroll_dead_zone);
  RUN_TEST(test_scroll_tilt_up_scrolls_up_faster_further);
  RUN_TEST(test_scroll_invert_and_speed);
  RUN_TEST(test_scroll_sideways_needs_more);
  RUN_TEST(test_knob_twist_steps_volume);
  RUN_TEST(test_knob_ignores_drift_and_partial_turns);
  RUN_TEST(test_knob_flick_next_and_previous);
  RUN_TEST(test_buttons_a_click);
  RUN_TEST(test_buttons_a_press_waits_the_chord_window_then_holds);
  RUN_TEST(test_buttons_b_click_waits_for_a_double);
  RUN_TEST(test_buttons_b_double);
  RUN_TEST(test_buttons_b_hold_isnt_a_click);
  RUN_TEST(test_buttons_chord_tap_and_hold);
  RUN_TEST(test_buttons_late_b_is_its_own_press);
  RUN_TEST(test_buttons_press_that_opened_the_mode_is_ignored);
  RUN_TEST(test_timer_runs_pauses_and_warns);
  RUN_TEST(test_format_clock);
  return UNITY_END();
}
