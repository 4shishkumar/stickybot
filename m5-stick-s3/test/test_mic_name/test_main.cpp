// The USB microphone's name follows the stick's name (same rule as the agent's Bluetooth mic).
#include <unity.h>
#include "logic/mic_name.h"

void setUp() {}
void tearDown() {}

static const char* usb(const char* name) {
  static char out[48];
  micname::usb(name, out, sizeof out);
  return out;
}

void test_named_after_the_stick() {
  TEST_ASSERT_EQUAL_STRING("stick-key-mic-usb", usb("stick-key"));
  TEST_ASSERT_EQUAL_STRING("arbaaz-sticks3-mic-usb", usb("Arbaaz StickS3"));
  TEST_ASSERT_EQUAL_STRING("ana-s-key-mic-usb", usb("  Ana's  key! "));
}

void test_nothing_usable_falls_back() {
  TEST_ASSERT_EQUAL_STRING("stick-mic-usb", usb(""));
  TEST_ASSERT_EQUAL_STRING("stick-mic-usb", usb("!!"));
  TEST_ASSERT_EQUAL_STRING("stick-mic-usb", usb(nullptr));
}

void test_a_20_character_name_fits() {
  TEST_ASSERT_EQUAL_STRING("abcdefghijklmnopqrst-mic-usb", usb("ABCDEFGHIJKLMNOPQRST"));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_named_after_the_stick);
  RUN_TEST(test_nothing_usable_falls_back);
  RUN_TEST(test_a_20_character_name_fits);
  return UNITY_END();
}
