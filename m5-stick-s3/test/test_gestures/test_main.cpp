// Motion gestures from synthetic IMU samples at 50 Hz.
#include <unity.h>
#include <vector>
#include "logic/gestures.h"

using gestures::Detector;
using gestures::Event;

void setUp() {}
void tearDown() {}

static const float FLAT[3] = {0, 0, 1};      // lying face-up on the desk
static const float UPRIGHT[3] = {0, 1, 0};   // held upright
static const float FACEDOWN[3] = {0, 0, -1};
static const float NOGYRO[3] = {0, 0, 0};

struct Sim {
  Detector d;
  uint32_t t = 1000;
  std::vector<Event> events;
  Sim() { d.begin(t); }
  // feed `ms` worth of the same sample
  void hold(const float a[3], uint32_t ms, const float g[3] = NOGYRO) {
    for (uint32_t e = 0; e < ms; e += 20) step(a, g);
  }
  Event step(const float a[3], const float g[3] = NOGYRO) {
    t += 20;
    Event ev = d.feed(t, a, g);
    if (ev != Event::NONE) events.push_back(ev);
    return ev;
  }
  int count(Event e) const {
    int n = 0;
    for (auto x : events) n += x == e;
    return n;
  }
};

void test_still_is_quiet() {
  Sim s;
  s.hold(FLAT, 10000);
  TEST_ASSERT_EQUAL(0, (int)s.events.size());
}

void test_lift_after_rest() {
  Sim s;
  s.hold(FLAT, 4000);                 // resting > 3 s
  const float moved[3] = {0.3f, 0.2f, 0.9f};
  TEST_ASSERT_TRUE(s.step(moved) == Event::LIFT);
  s.hold(FLAT, 500);                  // moving again soon after: no second lift
  s.step(moved);
  TEST_ASSERT_EQUAL(1, s.count(Event::LIFT));
}

void test_face_down_once_after_3s() {
  Sim s;
  s.hold(FACEDOWN, 2900);
  TEST_ASSERT_EQUAL(0, s.count(Event::FACE_DOWN));
  s.hold(FACEDOWN, 3000);
  TEST_ASSERT_EQUAL(1, s.count(Event::FACE_DOWN));  // fires once, not repeatedly
  s.hold(FLAT, 200);
  s.hold(FACEDOWN, 3200);
  TEST_ASSERT_EQUAL(2, s.count(Event::FACE_DOWN));  // again after being turned over
}

void test_shake_needs_three_jolts_in_window() {
  Sim s;
  s.hold(FLAT, 2000);
  const float jolt[3] = {2.5f, 0, 1};
  for (int i = 0; i < 3; i++) { s.step(jolt); s.hold(FLAT, 160); }  // 3 jolts ~0.5 s
  TEST_ASSERT_EQUAL(1, s.count(Event::SHAKE));

  Sim slow;
  slow.hold(FLAT, 2000);
  for (int i = 0; i < 3; i++) { slow.step(jolt); slow.hold(FLAT, 600); }  // too spread out
  TEST_ASSERT_EQUAL(0, slow.count(Event::SHAKE));
}

void test_shake_cooldown() {
  Sim s;
  s.hold(FLAT, 2000);
  const float jolt[3] = {2.5f, 0, 1};
  for (int i = 0; i < 6; i++) { s.step(jolt); s.hold(FLAT, 100); }  // one long shake
  TEST_ASSERT_EQUAL(1, s.count(Event::SHAKE));
}

void test_flick_direction_and_cooldown() {
  Sim s;
  s.hold(UPRIGHT, 2000);
  const float right[3] = {0, 400, 0}, left[3] = {0, -400, 0};
  TEST_ASSERT_TRUE(s.step(UPRIGHT, right) == Event::FLICK_RIGHT);
  TEST_ASSERT_TRUE(s.step(UPRIGHT, right) == Event::NONE);  // cooldown
  s.hold(UPRIGHT, 800);
  TEST_ASSERT_TRUE(s.step(UPRIGHT, left) == Event::FLICK_LEFT);
}

void test_orientation_debounced() {
  Sim s;
  s.hold(UPRIGHT, 300);
  TEST_ASSERT_EQUAL(0, s.d.upright());   // not yet: needs 600 ms
  s.hold(UPRIGHT, 500);
  TEST_ASSERT_EQUAL(1, s.d.upright());
  const float down[3] = {0, -1, 0};
  s.hold(down, 800);
  TEST_ASSERT_EQUAL(-1, s.d.upright());
  s.hold(FLAT, 2000);
  TEST_ASSERT_EQUAL(-1, s.d.upright());  // flat keeps the last orientation
}

void test_sideways() {
  Sim s;
  const float side[3] = {-1, 0.1f, 0.1f};
  s.hold(side, 500);
  TEST_ASSERT_EQUAL(0, s.d.sideways());
  s.hold(side, 500);
  TEST_ASSERT_EQUAL(-1, s.d.sideways());
  s.hold(FLAT, 1000);
  TEST_ASSERT_EQUAL(0, s.d.sideways());
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_still_is_quiet);
  RUN_TEST(test_lift_after_rest);
  RUN_TEST(test_face_down_once_after_3s);
  RUN_TEST(test_shake_needs_three_jolts_in_window);
  RUN_TEST(test_shake_cooldown);
  RUN_TEST(test_flick_direction_and_cooldown);
  RUN_TEST(test_orientation_debounced);
  RUN_TEST(test_sideways);
  return UNITY_END();
}
