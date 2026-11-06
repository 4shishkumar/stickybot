// The clock faces: the time in words (Words), the hands (Analog), words stacked into lines.
#include <unity.h>
#include "logic/clockface.h"

void setUp() {}
void tearDown() {}

static void expectWords(int h, int m, const char* lead, const char* text, const char* hour) {
  clockface::Words w = clockface::words(h, m);
  char msg[32];
  snprintf(msg, sizeof msg, "%02d:%02d", h, m);
  TEST_ASSERT_EQUAL_STRING_MESSAGE(lead, w.lead.c_str(), msg);
  TEST_ASSERT_EQUAL_STRING_MESSAGE(text, w.text.c_str(), msg);
  TEST_ASSERT_EQUAL_STRING_MESSAGE(hour, w.hour.c_str(), msg);
}

void test_on_the_five() {
  expectWords(9, 0, "", "nine o'clock", "nine");
  expectWords(9, 5, "", "five past nine", "nine");
  expectWords(9, 10, "", "ten past nine", "nine");
  expectWords(9, 15, "", "quarter past nine", "nine");
  expectWords(9, 20, "", "twenty past nine", "nine");
  expectWords(9, 25, "", "twenty-five past nine", "nine");
  expectWords(9, 30, "", "half past nine", "nine");
  expectWords(9, 35, "", "twenty-five to ten", "ten");
  expectWords(9, 40, "", "twenty to ten", "ten");
  expectWords(9, 45, "", "quarter to ten", "ten");
  expectWords(9, 50, "", "ten to ten", "ten");
  expectWords(9, 55, "", "five to ten", "ten");
}

void test_between_the_fives() {
  expectWords(9, 41, "just after", "twenty to ten", "ten");
  expectWords(9, 42, "just after", "twenty to ten", "ten");
  expectWords(9, 43, "nearly", "quarter to ten", "ten");
  expectWords(9, 44, "nearly", "quarter to ten", "ten");
  expectWords(10, 28, "nearly", "half past ten", "ten");
  expectWords(10, 1, "just after", "ten o'clock", "ten");
}

void test_the_hour_rolls_over() {
  expectWords(9, 58, "nearly", "ten o'clock", "ten");
  expectWords(12, 58, "nearly", "one o'clock", "one");  // 12-hour words
  expectWords(15, 20, "", "twenty past three", "three");
  expectWords(23, 59, "nearly", "midnight", "midnight");
}

void test_midnight_and_noon() {
  expectWords(0, 0, "", "midnight", "midnight");
  expectWords(12, 0, "", "noon", "noon");
  expectWords(0, 10, "", "ten past midnight", "midnight");
  expectWords(23, 45, "", "quarter to midnight", "midnight");
  expectWords(11, 45, "", "quarter to twelve", "twelve");  // noon only on the hour
  expectWords(12, 30, "", "half past twelve", "twelve");
  expectWords(11, 58, "nearly", "noon", "noon");
  expectWords(0, 2, "just after", "midnight", "midnight");
}

void test_out_of_range_wraps() {
  expectWords(24, 0, "", "midnight", "midnight");
  expectWords(-1, 30, "", "half past eleven", "eleven");
}

void test_hands() {
  clockface::Hands h = clockface::hands(0, 0);
  TEST_ASSERT_EQUAL_FLOAT(0, h.hour);
  TEST_ASSERT_EQUAL_FLOAT(0, h.minute);
  TEST_ASSERT_EQUAL_FLOAT(0, h.second);
  h = clockface::hands(3 * 60, 0);  // 3:00: the hour hand at 90 degrees
  TEST_ASSERT_EQUAL_FLOAT(90, h.hour);
  TEST_ASSERT_EQUAL_FLOAT(0, h.minute);
  h = clockface::hands(15 * 60 + 30, 30);  // 3:30:30 in the afternoon
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 105.25f, h.hour);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 183.0f, h.minute);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 180.0f, h.second);
  h = clockface::hands(23 * 60 + 59, 59.5f);  // almost round again
  TEST_ASSERT_TRUE(h.hour < 360 && h.hour > 359.9f);
  TEST_ASSERT_TRUE(h.minute < 360 && h.minute > 359.9f);
  h = clockface::hands(1440 + 60, 75);  // out of range: wrapped, the seconds held under a minute
  TEST_ASSERT_TRUE(h.second < 360);
  TEST_ASSERT_FLOAT_WITHIN(0.1f, 30.5f, h.hour);  // 1:00 (and the clamped seconds)
}

// 10 px a character
static int chars(const std::string& s, void*) { return (int)s.size() * 10; }

void test_stack() {
  auto l = clockface::stack("twenty-five past eleven", 100, chars, nullptr);  // 10 characters a line
  TEST_ASSERT_EQUAL(3, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("twenty-", l[0].c_str());
  TEST_ASSERT_EQUAL_STRING("five past", l[1].c_str());
  TEST_ASSERT_EQUAL_STRING("eleven", l[2].c_str());
  l = clockface::stack("twenty-five past eleven", 130, chars, nullptr);
  TEST_ASSERT_EQUAL(2, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("twenty-five", l[0].c_str());
  TEST_ASSERT_EQUAL_STRING("past eleven", l[1].c_str());
  l = clockface::stack("  half   past ten ", 1000, chars, nullptr);
  TEST_ASSERT_EQUAL(1, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("half past ten", l[0].c_str());
  l = clockface::stack("midnight", 50, chars, nullptr);  // too wide, no hyphen: alone
  TEST_ASSERT_EQUAL(1, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("midnight", l[0].c_str());
  TEST_ASSERT_EQUAL(0, (int)clockface::stack("", 100, chars, nullptr).size());
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_on_the_five);
  RUN_TEST(test_between_the_fives);
  RUN_TEST(test_the_hour_rolls_over);
  RUN_TEST(test_midnight_and_noon);
  RUN_TEST(test_out_of_range_wraps);
  RUN_TEST(test_hands);
  RUN_TEST(test_stack);
  return UNITY_END();
}
