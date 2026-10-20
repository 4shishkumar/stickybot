// Battery maths: charge from voltage, the trend behind "full in ...", and the
// last-hour ring that survives standby.
#include <unity.h>
#include <string.h>
#include "logic/battery_math.h"

void setUp() {}
void tearDown() {}

void test_curve_ends_and_points() {
  TEST_ASSERT_EQUAL(100, batt::percentFromMv(4200));
  TEST_ASSERT_EQUAL(100, batt::percentFromMv(4150));
  TEST_ASSERT_EQUAL(80, batt::percentFromMv(3970));
  TEST_ASSERT_EQUAL(50, batt::percentFromMv(3800));
  TEST_ASSERT_EQUAL(0, batt::percentFromMv(3300));
  TEST_ASSERT_EQUAL(0, batt::percentFromMv(3000));
}

void test_curve_interpolates_and_is_monotonic() {
  TEST_ASSERT_EQUAL(85, batt::percentFromMv(4010));  // halfway 3970..4050
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 85.0f, batt::percentFromMvF(4010));
  int prev = -1;
  for (int mv = 3200; mv <= 4250; mv += 5) {
    int p = batt::percentFromMv(mv);
    TEST_ASSERT_TRUE(p >= prev);
    prev = p;
  }
}

void test_median() {
  int a[] = {4000, 3000, 5000};
  TEST_ASSERT_EQUAL(4000, batt::median(a, 3));
  TEST_ASSERT_EQUAL(4000, a[0]);  // input untouched
  int b[] = {1, 9, 3, 7, 5};
  TEST_ASSERT_EQUAL(5, batt::median(b, 5));
  int c[] = {42};
  TEST_ASSERT_EQUAL(42, batt::median(c, 1));
}

void test_slope() {
  float x[12], y[12], s = 0;
  for (int i = 0; i < 12; i++) { x[i] = i; y[i] = 50 + 0.5f * i; }
  TEST_ASSERT_TRUE(batt::slope(x, y, 12, 10, s));
  TEST_ASSERT_FLOAT_WITHIN(1e-4f, 0.5f, s);
  TEST_ASSERT_FALSE(batt::slope(x, y, 9, 10, s));  // not enough points
  float flat[12] = {};
  TEST_ASSERT_FALSE(batt::slope(flat, y, 12, 10, s));  // no spread in x
}

void test_eta() {
  TEST_ASSERT_EQUAL(-1, batt::etaMinutes(-1, false, true, -0.1f));  // no level yet
  TEST_ASSERT_EQUAL(-1, batt::etaMinutes(50, false, false, 0));     // no trend yet
  TEST_ASSERT_EQUAL(500, batt::etaMinutes(50, false, true, -0.1f));  // 50% at 0.1%/min
  TEST_ASSERT_EQUAL(24 * 60, batt::etaMinutes(50, false, true, -0.001f));  // barely dropping
  TEST_ASSERT_EQUAL(100, batt::etaMinutes(80, true, true, 0.2f));   // 20% at 0.2%/min
  TEST_ASSERT_EQUAL(-2, batt::etaMinutes(80, true, true, 0.01f));   // charging too slowly
  TEST_ASSERT_EQUAL(0, batt::etaMinutes(99, true, true, 0.5f));     // full
}

static batt::RecentRing<5> ring;

void test_ring_cold_boot_and_wrap() {
  memset(&ring, 0xA5, sizeof ring);  // garbage, like RTC memory after power-up
  uint8_t p[5];
  bool c[5];
  TEST_ASSERT_EQUAL(0, ring.read(p, c));
  for (int i = 0; i < 7; i++) ring.add(10 + i, i % 2, 0, false);  // no wall clock: always add
  TEST_ASSERT_EQUAL(5, ring.read(p, c));
  TEST_ASSERT_EQUAL_UINT8(12, p[0]);  // oldest two dropped
  TEST_ASSERT_EQUAL_UINT8(16, p[4]);
  TEST_ASSERT_TRUE(c[4] == false && c[3] == true);
}

void test_ring_dedupes_and_resets_after_long_sleep() {
  ring.magic = 0;  // cold boot
  uint8_t p[5];
  bool c[5];
  ring.add(50, false, 1800000000, true);
  ring.add(51, false, 1800000020, true);   // 20 s later (just woke): skipped
  TEST_ASSERT_EQUAL(1, ring.read(p, c));
  ring.add(52, false, 1800000060, true);
  TEST_ASSERT_EQUAL(2, ring.read(p, c));
  ring.add(60, true, 1800000060 + 4000, true);  // more than an hour later: start over
  TEST_ASSERT_EQUAL(1, ring.read(p, c));
  TEST_ASSERT_EQUAL_UINT8(60, p[0]);
  TEST_ASSERT_TRUE(c[0]);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_curve_ends_and_points);
  RUN_TEST(test_curve_interpolates_and_is_monotonic);
  RUN_TEST(test_median);
  RUN_TEST(test_slope);
  RUN_TEST(test_eta);
  RUN_TEST(test_ring_cold_boot_and_wrap);
  RUN_TEST(test_ring_dedupes_and_resets_after_long_sleep);
  return UNITY_END();
}
