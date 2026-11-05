// The wireless Wispr mic decision (A hold -> stream to the agent or not).
#include <unity.h>
#include "logic/wispr.h"

void setUp() {}
void tearDown() {}

static wispr::Inputs ready() { return {true, true, true, false, false}; }

void test_streams_when_everything_is_ready() { TEST_ASSERT_TRUE(wispr::shouldStream(ready())); }

void test_stick_setting_off() {
  auto in = ready();
  in.setting = false;
  TEST_ASSERT_FALSE(wispr::shouldStream(in));
}

void test_agent_doesnt_want_it_or_is_gone() {
  auto in = ready();
  in.agentWants = false;  // wireless_mic off, BlackHole missing, or an older agent
  TEST_ASSERT_FALSE(wispr::shouldStream(in));
  in = ready();
  in.agentLinked = false;
  TEST_ASSERT_FALSE(wispr::shouldStream(in));
}

void test_usb_mic_in_use_wins() {
  auto in = ready();
  in.usbListening = true;
  TEST_ASSERT_FALSE(wispr::shouldStream(in));
}

void test_not_during_a_talk() {
  auto in = ready();
  in.micBusy = true;
  TEST_ASSERT_FALSE(wispr::shouldStream(in));
}

void test_key_up_tail_only_when_streaming() {
  TEST_ASSERT_EQUAL_UINT32(0, wispr::keyUpDelayMs(false));
  TEST_ASSERT_EQUAL_UINT32(wispr::KEYUP_TAIL_MS, wispr::keyUpDelayMs(true));
  TEST_ASSERT_TRUE(wispr::KEYUP_TAIL_MS > 0 && wispr::KEYUP_TAIL_MS <= 400);  // short enough not to feel laggy
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_streams_when_everything_is_ready);
  RUN_TEST(test_stick_setting_off);
  RUN_TEST(test_agent_doesnt_want_it_or_is_gone);
  RUN_TEST(test_usb_mic_in_use_wins);
  RUN_TEST(test_not_during_a_talk);
  RUN_TEST(test_key_up_tail_only_when_streaming);
  return UNITY_END();
}
