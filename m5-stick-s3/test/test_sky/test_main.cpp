// The sky by time of day and weather (StickyWorld, Aquarium).
#include <unity.h>
#include "logic/sky.h"

void setUp() {}
void tearDown() {}

static int H(int h, int m = 0) { return h * 60 + m; }

void test_phases() {
  TEST_ASSERT_EQUAL((int)sky::Phase::NIGHT, (int)sky::phase(H(2)));
  TEST_ASSERT_EQUAL((int)sky::Phase::DAWN, (int)sky::phase(H(6, 30)));
  TEST_ASSERT_EQUAL((int)sky::Phase::DAY, (int)sky::phase(H(12)));
  TEST_ASSERT_EQUAL((int)sky::Phase::DUSK, (int)sky::phase(H(18, 30)));
  TEST_ASSERT_EQUAL((int)sky::Phase::NIGHT, (int)sky::phase(H(23)));
  TEST_ASSERT_EQUAL((int)sky::Phase::NIGHT, (int)sky::phase(-60));       // wraps
  TEST_ASSERT_EQUAL((int)sky::Phase::DAY, (int)sky::phase(H(12) + 1440));
}

void test_light_follows_the_day() {
  TEST_ASSERT_TRUE(sky::colors(H(1), false).light < 0.1f);
  TEST_ASSERT_TRUE(sky::colors(H(12), false).light > 0.95f);
  float dawn = sky::colors(H(6, 30), false).light;
  TEST_ASSERT_TRUE(dawn > 0.2f && dawn < 0.9f);
  // smooth: no jumps minute to minute
  for (int m = 0; m < 1440; m++) {
    sky::Colors a = sky::colors(m, false), b = sky::colors(m + 1, false);
    TEST_ASSERT_TRUE(abs(a.top.r - b.top.r) <= 3 && abs(a.bottom.g - b.bottom.g) <= 3);
  }
  // day is blue at the top, night dark
  sky::Colors noon = sky::colors(H(12), false), night = sky::colors(H(1), false);
  TEST_ASSERT_TRUE(noon.top.b > noon.top.r);
  TEST_ASSERT_TRUE(night.top.r + night.top.g + night.top.b < 90);
}

void test_overcast_is_greyer() {
  sky::Colors c = sky::colors(H(12), false), o = sky::colors(H(12), true);
  int satC = c.top.b - c.top.r, satO = o.top.b - o.top.r;
  TEST_ASSERT_TRUE(satO < satC);
  TEST_ASSERT_TRUE(o.light < c.light);
  TEST_ASSERT_TRUE(sky::overcast("rain"));
  TEST_ASSERT_FALSE(sky::overcast("clear"));
  TEST_ASSERT_FALSE(sky::overcast(nullptr));
  TEST_ASSERT_EQUAL(0, sky::clouds("clear"));
  TEST_ASSERT_EQUAL(2, sky::clouds("partly"));
  TEST_ASSERT_TRUE(sky::clouds("storm") > sky::clouds("partly"));
}

void test_sun_and_moon() {
  float t;
  TEST_ASSERT_TRUE(sky::sunAt(H(12, 30), t));
  TEST_ASSERT_FLOAT_WITHIN(0.02f, 0.5f, t);
  TEST_ASSERT_FALSE(sky::sunAt(H(2), t));
  TEST_ASSERT_TRUE(sky::sunAt(H(6, 10), t));
  TEST_ASSERT_TRUE(t < 0.05f);
  TEST_ASSERT_TRUE(sky::moonAt(H(0, 30), t));
  TEST_ASSERT_FLOAT_WITHIN(0.02f, 0.5f, t);
  TEST_ASSERT_TRUE(sky::moonAt(H(19), t));
  TEST_ASSERT_TRUE(t < 0.1f);
  TEST_ASSERT_FALSE(sky::moonAt(H(12), t));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_phases);
  RUN_TEST(test_light_follows_the_day);
  RUN_TEST(test_overcast_is_greyer);
  RUN_TEST(test_sun_and_moon);
  return UNITY_END();
}
