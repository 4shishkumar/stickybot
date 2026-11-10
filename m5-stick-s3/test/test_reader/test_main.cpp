// The reply view (logic/reader.cpp): the full reply arriving in parts, wrapping a long text,
// paging, the scroll thumb and tilt scrolling. Widths: 6 px a character (UTF-8 aware).
#include <unity.h>
#include <string>
#include "logic/reader.h"

void setUp() {}
void tearDown() {}

static int chars(const std::string& s, void*) {
  int n = 0;
  for (unsigned char c : s) if ((c & 0xC0) != 0x80) n++;
  return n * 6;
}

static bool validUtf8(const std::string& s) {
  for (size_t i = 0; i < s.size();) {
    unsigned char c = s[i];
    int n = c < 0x80 ? 1 : (c >> 5) == 6 ? 2 : (c >> 4) == 14 ? 3 : (c >> 3) == 30 ? 4 : 0;
    if (!n || i + n > s.size()) return false;
    for (int k = 1; k < n; k++)
      if (((unsigned char)s[i + k] & 0xC0) != 0x80) return false;
    i += n;
  }
  return true;
}

void test_cap_keeps_characters_whole() {
  std::string s = "28°C";  // ° is 2 bytes: 2,8,0xC2,0xB0,C
  reader::capUtf8(s, 3);
  TEST_ASSERT_EQUAL_STRING("28", s.c_str());
  s = "28°C";
  reader::capUtf8(s, 4);
  TEST_ASSERT_EQUAL_STRING("28°", s.c_str());
  s = "short";
  reader::capUtf8(s, 100);
  TEST_ASSERT_EQUAL_STRING("short", s.c_str());
  std::string dashes;
  for (int i = 0; i < 2000; i++) dashes += "—";  // 3 bytes each
  reader::capUtf8(dashes, reader::MAX_BYTES);
  TEST_ASSERT_TRUE(dashes.size() <= reader::MAX_BYTES && dashes.size() % 3 == 0);
  TEST_ASSERT_TRUE(validUtf8(dashes));
}

void test_wrap_paragraphs_and_blank_lines() {
  auto l = reader::wrapAll("the quick brown fox\n\n\nnext one\nsame para", 60, chars, nullptr);  // 10 chars a line
  TEST_ASSERT_EQUAL(5, (int)l.size());
  TEST_ASSERT_EQUAL_STRING("the quick", l[0].c_str());
  TEST_ASSERT_EQUAL_STRING("brown fox", l[1].c_str());
  TEST_ASSERT_EQUAL_STRING("", l[2].c_str());  // three newlines: one blank line
  TEST_ASSERT_EQUAL_STRING("next one", l[3].c_str());
  TEST_ASSERT_EQUAL_STRING("same para", l[4].c_str());  // a single newline: no blank line
}

void test_wrap_no_leading_or_trailing_blank() {
  auto l = reader::wrapAll("\n\n  hello  world \n\n", 60, chars, nullptr);
  TEST_ASSERT_EQUAL(2, (int)l.size());  // "hello world" is 11 characters: two lines, no blank ones
  TEST_ASSERT_EQUAL_STRING("hello", l[0].c_str());
  TEST_ASSERT_EQUAL_STRING("world", l[1].c_str());
  TEST_ASSERT_TRUE(reader::wrapAll("", 60, chars, nullptr).empty());
}

void test_wrap_breaks_long_words_utf8_safe() {
  auto l = reader::wrapAll("see https://example.com/a/very/long/path ok", 60, chars, nullptr);
  for (auto& s : l) {
    TEST_ASSERT_TRUE(chars(s, nullptr) <= 60);
    TEST_ASSERT_TRUE(validUtf8(s));
  }
  TEST_ASSERT_EQUAL_STRING("see", l[0].c_str());
  TEST_ASSERT_EQUAL_STRING("https://ex", l[1].c_str());
  std::string joined;
  for (size_t i = 1; i < l.size(); i++) joined += l[i];
  TEST_ASSERT_TRUE(joined.find("https://example.com/a/very/long/path") == 0);
  auto u = reader::wrapAll("°°°°°°°°°°°°°°°°°°°°°°°°", 30, chars, nullptr);  // 5 a line
  TEST_ASSERT_EQUAL(5, (int)u.size());
  for (auto& s : u) TEST_ASSERT_TRUE(validUtf8(s));
  TEST_ASSERT_EQUAL_STRING("°°°°°", u[0].c_str());
}

void test_wrap_every_word_kept() {
  std::string t;
  for (int i = 0; i < 300; i++) t += "word" + std::to_string(i) + (i % 37 == 36 ? "\n" : " ");
  auto l = reader::wrapAll(t, 100, chars, nullptr);
  std::string back;
  for (auto& s : l) {
    TEST_ASSERT_TRUE(chars(s, nullptr) <= 100);
    if (!s.empty()) back += s + " ";
  }
  for (int i = 0; i < 300; i += 17) TEST_ASSERT_TRUE(back.find("word" + std::to_string(i) + " ") != std::string::npos);
}

void test_assembly_in_order() {
  reader::Assembly a;
  a.begin(7, 3);
  TEST_ASSERT_TRUE(a.waiting());
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::IGNORED, (int)a.add(6, 0, "old reply"));
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::ADDED, (int)a.add(7, 0, "one "));
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::ADDED, (int)a.add(7, 1, "two "));
  TEST_ASSERT_FALSE(a.complete());
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::COMPLETE, (int)a.add(7, 2, "three"));
  TEST_ASSERT_TRUE(a.complete());
  TEST_ASSERT_EQUAL_STRING("one two three", a.text().c_str());
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::IGNORED, (int)a.add(7, 3, "extra"));
  a.begin(8, 0);  // a short reply: nothing follows
  TEST_ASSERT_FALSE(a.waiting());
  TEST_ASSERT_FALSE(a.complete());
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::IGNORED, (int)a.add(8, 0, "x"));
}

void test_assembly_lost_part_gives_up() {
  reader::Assembly a;
  a.begin(9, 3);
  a.add(9, 0, "one ");
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::BROKEN, (int)a.add(9, 2, "three"));
  TEST_ASSERT_FALSE(a.complete());
  TEST_ASSERT_FALSE(a.waiting());
  TEST_ASSERT_EQUAL((int)reader::Assembly::Step::IGNORED, (int)a.add(9, 1, "two"));
}

void test_assembly_caps_the_size() {
  reader::Assembly a;
  a.begin(1, 10);
  std::string part(500, 'x');
  part += "°";
  for (int i = 0; i < 10; i++) a.add(1, i, part.c_str());
  TEST_ASSERT_TRUE(a.complete());
  TEST_ASSERT_TRUE(a.text().size() <= reader::MAX_BYTES);
  TEST_ASSERT_TRUE(validUtf8(a.text()));
}

void test_pager_pages_and_wraps_to_top() {
  reader::Pager p{25, 10, 0};
  TEST_ASSERT_TRUE(p.paged());
  TEST_ASSERT_EQUAL(15, p.maxTop());
  TEST_ASSERT_EQUAL(3, p.pages());  // 0, 9, 15
  TEST_ASSERT_EQUAL(1, p.page());
  p.next();
  TEST_ASSERT_EQUAL(9, p.top);  // a line of the last page stays
  TEST_ASSERT_EQUAL(2, p.page());
  p.next();
  TEST_ASSERT_EQUAL(15, p.top);
  TEST_ASSERT_EQUAL(3, p.page());
  p.next();
  TEST_ASSERT_EQUAL(0, p.top);  // after the last: the top
  reader::Pager one{4, 10, 0};
  TEST_ASSERT_FALSE(one.paged());
  one.next();
  TEST_ASSERT_EQUAL(0, one.top);
  TEST_ASSERT_EQUAL(1, one.pages());
}

void test_pager_scroll_and_thumb() {
  reader::Pager p{40, 10, 0};
  TEST_ASSERT_FALSE(p.scroll(-3));
  TEST_ASSERT_TRUE(p.scroll(5));
  TEST_ASSERT_EQUAL(5, p.top);
  TEST_ASSERT_TRUE(p.scroll(100));
  TEST_ASSERT_EQUAL(30, p.top);
  int y, h;
  p.thumb(100, y, h);
  TEST_ASSERT_EQUAL(25, h);  // 10 of 40 lines
  TEST_ASSERT_EQUAL(75, y);  // at the bottom
  p.top = 15;
  p.thumb(100, y, h);
  TEST_ASSERT_EQUAL(37, y);  // half way
  reader::Pager big{1000, 10, 0};
  big.thumb(100, y, h);
  TEST_ASSERT_EQUAL(10, h);  // never thinner than 10 px
  p.total = 8;
  p.clamp();
  TEST_ASSERT_EQUAL(0, p.top);  // the text got shorter
}

void test_pitch() {
  TEST_ASSERT_FLOAT_WITHIN(0.5f, 90.0f, reader::pitchDeg(1.0f, 0.0f));  // upright
  TEST_ASSERT_FLOAT_WITHIN(0.5f, 0.0f, reader::pitchDeg(0.0f, 1.0f));   // face up
  TEST_ASSERT_FLOAT_WITHIN(0.5f, 45.0f, reader::pitchDeg(0.707f, 0.707f));
}

void test_tilt_dead_zone_and_direction() {
  reader::TiltScroll t;
  t.begin();
  TEST_ASSERT_EQUAL(0, t.feed(0.04f, 45));  // where it's held
  int n = 0;
  for (int i = 0; i < 50; i++) n += t.feed(0.04f, 45 - 8);  // 8° forward: in the dead zone
  TEST_ASSERT_EQUAL(0, n);
  t.begin();
  t.feed(0.04f, 45);
  for (int i = 0; i < 25; i++) n += t.feed(0.04f, 45 - 32);  // 32° forward, 1 s: 20° past it, 6 lines/s
  TEST_ASSERT_INT_WITHIN(1, 6, n);
  n = 0;
  for (int i = 0; i < 25; i++) n += t.feed(0.04f, 45 + 32);  // back: up
  TEST_ASSERT_INT_WITHIN(1, -6, n);
  n = 0;
  for (int i = 0; i < 25; i++) n += t.feed(0.04f, 45 - 80);  // far: capped
  TEST_ASSERT_INT_WITHIN(1, (int)reader::TiltScroll::MAX_RATE, n);
}

void test_tilt_follows_a_slow_change_of_grip() {
  reader::TiltScroll t;
  t.begin();
  t.feed(0.04f, 45);
  int n = 0;
  float deg = 45;
  for (int i = 0; i < 750; i++) {  // 30 s drifting 20° flatter, slowly
    deg -= 20.0f / 750;
    n += t.feed(0.04f, deg);
  }
  TEST_ASSERT_EQUAL(0, n);
  TEST_ASSERT_TRUE(t.offset() < reader::TiltScroll::DEAD_DEG);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_cap_keeps_characters_whole);
  RUN_TEST(test_wrap_paragraphs_and_blank_lines);
  RUN_TEST(test_wrap_no_leading_or_trailing_blank);
  RUN_TEST(test_wrap_breaks_long_words_utf8_safe);
  RUN_TEST(test_wrap_every_word_kept);
  RUN_TEST(test_assembly_in_order);
  RUN_TEST(test_assembly_lost_part_gives_up);
  RUN_TEST(test_assembly_caps_the_size);
  RUN_TEST(test_pager_pages_and_wraps_to_top);
  RUN_TEST(test_pager_scroll_and_thumb);
  RUN_TEST(test_pitch);
  RUN_TEST(test_tilt_dead_zone_and_direction);
  RUN_TEST(test_tilt_follows_a_slow_change_of_grip);
  return UNITY_END();
}
