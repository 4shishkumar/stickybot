// Screen orientation from synthetic IMU samples at 50 Hz: hysteresis, dwell, flat and
// moving keep the last one, and the settings (auto-rotate, landscape screens).
#include <unity.h>
#include <math.h>
#include "logic/orientation.h"

using namespace orientation;

void setUp() {}
void tearDown() {}

static const float UPRIGHT[3] = {0, 1, 0};
static const float UPSIDE[3] = {0, -1, 0};
static const float RIGHT_UP[3] = {1, 0, 0};    // right edge up: button A on the right
static const float LEFT_UP[3] = {-1, 0, 0};
static const float FLAT[3] = {0, 0, 1};
static const float FACEDOWN[3] = {0, 0, -1};
static const float STILL[3] = {0, 0, 0};

struct Sim {
  Tracker t;
  uint32_t now = 1000;
  void hold(const float a[3], uint32_t ms, const float g[3] = STILL) {
    for (uint32_t e = 0; e < ms; e += 20) t.feed(now += 20, a, g);
  }
};

// Gravity tilted `deg` degrees from upright towards the right edge (in the screen's plane).
static void tilted(float deg, float out[3]) {
  out[0] = sinf(deg * (float)M_PI / 180);
  out[1] = cosf(deg * (float)M_PI / 180);
  out[2] = 0;
}

void test_starts_portrait_and_follows_each_side() {
  Sim s;
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
  s.hold(RIGHT_UP, 700);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
  s.hold(UPSIDE, 700);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT_FLIPPED, (int)s.t.current());
  s.hold(LEFT_UP, 700);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_LEFT, (int)s.t.current());
  s.hold(UPRIGHT, 700);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
}

void test_dwell_needs_about_a_quarter_second() {
  Sim s;
  s.hold(RIGHT_UP, 160);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());  // not yet
  s.hold(RIGHT_UP, 160);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
}

void test_brief_turns_are_ignored() {
  Sim s;
  for (int i = 0; i < 5; i++) {  // flicking back and forth: never long enough on one side
    s.hold(RIGHT_UP, 160);
    s.hold(UPRIGHT, 160);
  }
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
}

void test_hysteresis_around_45_degrees() {
  Sim s;
  float a[3];
  tilted(50, a);  // just past 45°: still portrait (inside the 10° margin)
  s.hold(a, 2000);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
  tilted(65, a);  // clearly past: landscape
  s.hold(a, 2000);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
  tilted(40, a);  // back to 40°: stays landscape (needs < 35°)
  s.hold(a, 2000);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
  tilted(25, a);
  s.hold(a, 2000);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
}

void test_flat_keeps_the_last_one() {
  Sim s;
  s.hold(LEFT_UP, 700);
  s.hold(FLAT, 3000);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_LEFT, (int)s.t.current());
  s.hold(FACEDOWN, 3000);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_LEFT, (int)s.t.current());
  const float tiltedFlat[3] = {0.2f, 0.2f, 0.96f};  // a little tilt on the desk: still flat
  s.hold(tiltedFlat, 3000);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_LEFT, (int)s.t.current());
}

void test_leaning_back_still_turns() {
  Sim s;
  const float leanRight[3] = {0.42f, 0, 0.9f};  // held leaning back ~25° above flat, right edge up
  s.hold(leanRight, 700);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
}

void test_moving_holds_off() {
  Sim s;
  const float spin[3] = {0, 150, 0};
  s.hold(RIGHT_UP, 2000, spin);  // turning fast: no change however long
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
  const float swing[3] = {1.4f, 0, 0};  // swung (1.4 g): no change either
  s.hold(swing, 2000);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
  s.hold(RIGHT_UP, 160);  // a moving sample restarts the dwell
  s.t.feed(s.now += 20, RIGHT_UP, spin);
  s.hold(RIGHT_UP, 160);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT, (int)s.t.current());
  s.hold(RIGHT_UP, 160);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
}

void test_last_portrait_is_kept_while_landscape() {
  Sim s;
  s.hold(UPSIDE, 700);
  s.hold(RIGHT_UP, 700);
  TEST_ASSERT_EQUAL((int)Orient::LANDSCAPE_RIGHT, (int)s.t.current());
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT_FLIPPED, (int)s.t.lastPortrait());
}

void test_choose_follows_the_settings() {
  Sim s;
  s.hold(UPSIDE, 700);
  s.hold(LEFT_UP, 700);
  Screen sc = choose(s.t, true, true, false);  // auto-rotate + landscape: rotation 3
  TEST_ASSERT_EQUAL(-1, sc.side);
  TEST_ASSERT_TRUE(sc.flipped);  // portrait-only screens: the last portrait way up
  sc = choose(s.t, true, false, false);  // landscape screens off: portrait, as before
  TEST_ASSERT_EQUAL(0, sc.side);
  TEST_ASSERT_TRUE(sc.flipped);
  sc = choose(s.t, false, true, false);  // auto-rotate off: the manual flip, never landscape
  TEST_ASSERT_EQUAL(0, sc.side);
  TEST_ASSERT_FALSE(sc.flipped);
  sc = choose(s.t, false, true, true);
  TEST_ASSERT_TRUE(sc.flipped);
  s.hold(RIGHT_UP, 700);
  TEST_ASSERT_EQUAL(1, choose(s.t, true, true, false).side);
  TEST_ASSERT_TRUE(buttonAOnRight(1));
  TEST_ASSERT_FALSE(buttonAOnRight(-1));
}

void test_reset() {
  Tracker t(Orient::PORTRAIT_FLIPPED);
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT_FLIPPED, (int)t.current());
  TEST_ASSERT_EQUAL((int)Orient::PORTRAIT_FLIPPED, (int)t.lastPortrait());
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_starts_portrait_and_follows_each_side);
  RUN_TEST(test_dwell_needs_about_a_quarter_second);
  RUN_TEST(test_brief_turns_are_ignored);
  RUN_TEST(test_hysteresis_around_45_degrees);
  RUN_TEST(test_flat_keeps_the_last_one);
  RUN_TEST(test_leaning_back_still_turns);
  RUN_TEST(test_moving_holds_off);
  RUN_TEST(test_last_portrait_is_kept_while_landscape);
  RUN_TEST(test_choose_follows_the_settings);
  RUN_TEST(test_reset);
  return UNITY_END();
}
