// Key combos: parsing the text people type and showing it back.
#include <unity.h>
#include <string>
#include "logic/combo_logic.h"

void setUp() {}
void tearDown() {}

static Combo parsed(const char* s) {
  Combo c;
  TEST_ASSERT_TRUE_MESSAGE(combo::parse(s, c), s);
  return c;
}

void test_aliases_and_case() {
  Combo c = parsed("cmd+opt");
  TEST_ASSERT_EQUAL_HEX8(combo::MOD_GUI | combo::MOD_ALT, c.mods);
  TEST_ASSERT_EQUAL_HEX8(0, c.key);
  TEST_ASSERT_EQUAL_HEX8(parsed("Command + Option").mods, parsed("META+ALT").mods);
  TEST_ASSERT_EQUAL_HEX8(combo::MOD_CTRL, parsed("control+a").mods);
}

void test_main_keys() {
  TEST_ASSERT_EQUAL_HEX8(0x2C, parsed("ctrl + space").key);
  TEST_ASSERT_EQUAL_HEX8(0x21, parsed("Cmd+Shift+4").key);
  TEST_ASSERT_EQUAL_HEX8(0x28, parsed("enter").key);
  TEST_ASSERT_EQUAL_HEX8(0x29, parsed("escape").key);
  TEST_ASSERT_EQUAL_HEX8(0x4C, parsed("del").key);
  TEST_ASSERT_EQUAL_HEX8(0x4B, parsed("pgup").key);
  TEST_ASSERT_EQUAL_HEX8(0x31, parsed("cmd+\\").key);  // backslash
  TEST_ASSERT_EQUAL_HEX8(0x6F, parsed("F20").key);
}

void test_invalid() {
  Combo c;
  TEST_ASSERT_FALSE(combo::parse("", c));
  TEST_ASSERT_FALSE(combo::parse("cmd+", c));      // trailing +
  TEST_ASSERT_FALSE(combo::parse("+a", c));
  TEST_ASSERT_FALSE(combo::parse("a+b", c));       // two main keys
  TEST_ASSERT_FALSE(combo::parse("hyper+a", c));   // unknown token
  TEST_ASSERT_FALSE(combo::parse("(none)", c));
  c.mods = 7;
  TEST_ASSERT_FALSE(combo::parse("nope", c));
  TEST_ASSERT_EQUAL_HEX8(7, c.mods);  // out untouched on failure
}

void test_to_string_order() {
  Combo c;
  c.mods = combo::MOD_GUI | combo::MOD_SHIFT | combo::MOD_ALT | combo::MOD_CTRL;
  c.key = 0x04;
  TEST_ASSERT_EQUAL_STRING("Ctrl+Option+Shift+Cmd+A", combo::toString(c).c_str());
  TEST_ASSERT_EQUAL_STRING("(none)", combo::toString(Combo()).c_str());
  Combo unknown;
  unknown.key = 0xEE;
  TEST_ASSERT_EQUAL_STRING("?", combo::toString(unknown).c_str());
}

void test_every_key_round_trips() {
  for (size_t i = 1; i < KEY_COUNT; i++) {
    Combo c;
    c.mods = combo::MOD_GUI;
    c.key = KEYS[i].code;
    std::string s = combo::toString(c);
    Combo back;
    TEST_ASSERT_TRUE_MESSAGE(combo::parse(s, back), s.c_str());
    TEST_ASSERT_EQUAL_HEX8(c.key, back.key);
    TEST_ASSERT_EQUAL_HEX8(c.mods, back.mods);
  }
}

void test_fn_key() {
  Combo c = parsed("fn");
  TEST_ASSERT_EQUAL_HEX8(combo::MOD_FN, c.mods);
  TEST_ASSERT_EQUAL_HEX8(0, c.key);
  TEST_ASSERT_TRUE(c.valid());
  TEST_ASSERT_EQUAL_HEX8(combo::MOD_FN, parsed("Globe").mods);
  Combo f = parsed("fn+f");
  TEST_ASSERT_EQUAL_HEX8(combo::MOD_FN, f.mods);
  TEST_ASSERT_EQUAL_HEX8(0x09, f.key);
  TEST_ASSERT_EQUAL_STRING("Fn", combo::toString(c).c_str());
  Combo all = parsed("cmd+fn+ctrl");
  TEST_ASSERT_EQUAL_STRING("Fn+Ctrl+Cmd", combo::toString(all).c_str());  // fn first, like macOS
  TEST_ASSERT_EQUAL_HEX8(combo::MOD_MASK, combo::MOD_MASK & (all.mods | combo::MOD_SHIFT | combo::MOD_ALT));
}

void test_key_index() {
  TEST_ASSERT_EQUAL(0, keyIndex(0));
  TEST_ASSERT_EQUAL(-1, keyIndex(0xEE));
  TEST_ASSERT_EQUAL_STRING("A", KEYS[keyIndex(0x04)].name);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_aliases_and_case);
  RUN_TEST(test_fn_key);
  RUN_TEST(test_main_keys);
  RUN_TEST(test_invalid);
  RUN_TEST(test_to_string_order);
  RUN_TEST(test_every_key_round_trips);
  RUN_TEST(test_key_index);
  return UNITY_END();
}
