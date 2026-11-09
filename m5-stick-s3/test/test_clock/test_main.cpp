// Clock and date from the Mac agent's time (epoch + UTC offset).
#include <unity.h>
#include "logic/clock.h"

void setUp() {}
void tearDown() {}

constexpr int64_t TUE_2026_10_20_0942_UTC = 1792489320;  // Tue 20 Oct 2026 09:42:00 UTC
constexpr long IST = 5 * 3600 + 30 * 60;

void test_not_set_yet() {
  clockfmt::Local t;
  TEST_ASSERT_FALSE(clockfmt::local(1000, 0, t));
  TEST_ASSERT_FALSE(clockfmt::local(clockfmt::VALID_AFTER - 1, 0, t));
}

void test_utc_and_offset() {
  clockfmt::Local t;
  TEST_ASSERT_TRUE(clockfmt::local(TUE_2026_10_20_0942_UTC, 0, t));
  TEST_ASSERT_EQUAL(9, t.hour);
  TEST_ASSERT_EQUAL(42, t.min);
  TEST_ASSERT_TRUE(clockfmt::local(TUE_2026_10_20_0942_UTC, IST, t));
  TEST_ASSERT_EQUAL_STRING("3:12", clockfmt::time12(t).c_str());
  TEST_ASSERT_EQUAL_STRING("PM", clockfmt::ampm(t).c_str());
  TEST_ASSERT_EQUAL_STRING("Tue 20 Oct", clockfmt::shortDate(t).c_str());
  TEST_ASSERT_EQUAL_STRING("Tuesday 20 October", clockfmt::longDate(t).c_str());
}

void test_offset_crosses_midnight() {
  clockfmt::Local t;
  // 20:00 UTC + 5:30 = 01:30 the next day
  TEST_ASSERT_TRUE(clockfmt::local(TUE_2026_10_20_0942_UTC + (10 * 3600 + 18 * 60), IST, t));
  TEST_ASSERT_EQUAL_STRING("1:30", clockfmt::time12(t).c_str());
  TEST_ASSERT_EQUAL_STRING("AM", clockfmt::ampm(t).c_str());
  TEST_ASSERT_EQUAL_STRING("Wed 21 Oct", clockfmt::shortDate(t).c_str());
  // negative offset
  TEST_ASSERT_TRUE(clockfmt::local(TUE_2026_10_20_0942_UTC, -10 * 3600, t));
  TEST_ASSERT_EQUAL_STRING("Mon 19 Oct", clockfmt::shortDate(t).c_str());
}

void test_twelve_hour_edges() {
  clockfmt::Local t = {0, 5, 0, 1, 0};
  TEST_ASSERT_EQUAL_STRING("12:05", clockfmt::time12(t).c_str());
  TEST_ASSERT_EQUAL_STRING("AM", clockfmt::ampm(t).c_str());
  t.hour = 12;
  TEST_ASSERT_EQUAL_STRING("12:05", clockfmt::time12(t).c_str());
  TEST_ASSERT_EQUAL_STRING("PM", clockfmt::ampm(t).c_str());
  t.hour = 23; t.min = 59;
  TEST_ASSERT_EQUAL_STRING("11:59", clockfmt::time12(t).c_str());
}

void test_twenty_four_hour() {
  clockfmt::Local t = {0, 5, 0, 1, 0};
  TEST_ASSERT_EQUAL_STRING("0:05", clockfmt::time24(t).c_str());
  t.hour = 15;
  TEST_ASSERT_EQUAL_STRING("15:05", clockfmt::time24(t).c_str());
  t.hour = 23; t.min = 59;
  TEST_ASSERT_EQUAL_STRING("23:59", clockfmt::time24(t).c_str());
}

void test_night_window() {
  TEST_ASSERT_TRUE(clockfmt::isNight(22));
  TEST_ASSERT_TRUE(clockfmt::isNight(23));
  TEST_ASSERT_TRUE(clockfmt::isNight(0));
  TEST_ASSERT_TRUE(clockfmt::isNight(5));
  TEST_ASSERT_FALSE(clockfmt::isNight(6));
  TEST_ASSERT_FALSE(clockfmt::isNight(21));
  TEST_ASSERT_FALSE(clockfmt::isNight(-1));  // clock not set
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_not_set_yet);
  RUN_TEST(test_utc_and_offset);
  RUN_TEST(test_offset_crosses_midnight);
  RUN_TEST(test_twelve_hour_edges);
  RUN_TEST(test_twenty_four_hour);
  RUN_TEST(test_night_window);
  return UNITY_END();
}
