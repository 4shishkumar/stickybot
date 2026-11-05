// Word wrap and "..." cutting for the small screen (width measured by a fake:
// 6 px per character, like a fixed-width font).
#include <unity.h>
#include <string>
#include "logic/text.h"

void setUp() {}
void tearDown() {}

static int chars(const std::string& s, void*) {  // UTF-8 aware character count * 6
  int n = 0;
  for (unsigned char c : s) if ((c & 0xC0) != 0x80) n++;
  return n * 6;
}

void test_ellipsize() {
  TEST_ASSERT_EQUAL_STRING("Hacker News", text::ellipsize("Hacker News", 66, chars, nullptr).c_str());
  std::string cut = text::ellipsize("Wikipedia: London Monster", 60, chars, nullptr);  // 10 chars max
  TEST_ASSERT_TRUE(chars(cut, nullptr) <= 60);
  TEST_ASSERT_EQUAL_STRING("Wikiped...", cut.c_str());  // 7 letters + "..." = 10
}

void test_ellipsize_keeps_utf8_whole() {
  std::string cut = text::ellipsize("28° partly cloudy", 36, chars, nullptr);  // 6 chars max
  TEST_ASSERT_EQUAL_STRING("28°...", cut.c_str());
}

void test_wrap_basic() {
  auto l = text::wrap("the quick brown fox jumps", 5, 60, chars, nullptr);  // 10 chars a line
  TEST_ASSERT_EQUAL(3, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("the quick", l[0].c_str());
  TEST_ASSERT_EQUAL_STRING("brown fox", l[1].c_str());
  TEST_ASSERT_EQUAL_STRING("jumps", l[2].c_str());
}

void test_wrap_overflow_ends_with_ellipsis() {
  auto l = text::wrap("one two three four five six seven", 2, 60, chars, nullptr);
  TEST_ASSERT_EQUAL(2, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("one two", l[0].c_str());
  TEST_ASSERT_TRUE(l[1].size() >= 3 && l[1].substr(l[1].size() - 3) == "...");
  TEST_ASSERT_TRUE(chars(l[1], nullptr) <= 60);
}

void test_wrap_edge_cases() {
  TEST_ASSERT_EQUAL(0, (int)text::wrap("", 3, 60, chars, nullptr).size());
  TEST_ASSERT_EQUAL(0, (int)text::wrap("hello", 0, 60, chars, nullptr).size());
  auto l = text::wrap("supercalifragilistic", 2, 60, chars, nullptr);  // one long word stays whole
  TEST_ASSERT_EQUAL(1, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("supercalifragilistic", l[0].c_str());
  auto exact = text::wrap("abc def", 1, 42, chars, nullptr);  // fits exactly in one line
  TEST_ASSERT_EQUAL(1, (int)exact.size());
  TEST_ASSERT_EQUAL_STRING("abc def", exact[0].c_str());
}

void test_fits() {
  // 10 chars a line at 60 px: 3 lines here; a wider (landscape) line needs fewer
  TEST_ASSERT_TRUE(text::fits("the quick brown fox jumps", 3, 60, chars, nullptr));
  TEST_ASSERT_FALSE(text::fits("the quick brown fox jumps", 2, 60, chars, nullptr));
  TEST_ASSERT_TRUE(text::fits("the quick brown fox jumps", 2, 120, chars, nullptr));
  TEST_ASSERT_TRUE(text::fits("the quick brown fox jumps", 1, 150, chars, nullptr));
  TEST_ASSERT_FALSE(text::fits("supercalifragilistic", 3, 60, chars, nullptr));  // a word too wide
  TEST_ASSERT_TRUE(text::fits("", 0, 60, chars, nullptr));
  TEST_ASSERT_FALSE(text::fits("x", 0, 60, chars, nullptr));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_ellipsize);
  RUN_TEST(test_ellipsize_keeps_utf8_whole);
  RUN_TEST(test_wrap_basic);
  RUN_TEST(test_wrap_overflow_ends_with_ellipsis);
  RUN_TEST(test_wrap_edge_cases);
  RUN_TEST(test_fits);
  return UNITY_END();
}
