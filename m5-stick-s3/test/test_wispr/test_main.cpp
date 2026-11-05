// The Wispr mic for an A hold: the path (USB / Bluetooth / none) and the key-down gate.
#include <unity.h>
#include "logic/wispr.h"

using wispr::Gate;
using wispr::Path;

void setUp() {}
void tearDown() {}

// On battery, agent linked and able: the new Bluetooth flow
static wispr::Inputs ready() { return {true, true, true, true, false, false, false}; }

void test_bluetooth_prepare_when_everything_is_ready() {
  TEST_ASSERT_EQUAL(Path::BT_PREPARE, wispr::choosePath(ready()));
  TEST_ASSERT_TRUE(wispr::streams(Path::BT_PREPARE));
}

void test_an_agent_without_ready_gets_the_old_flow() {
  auto in = ready();
  in.agentPrepares = false;  // 1.7.0 agent
  TEST_ASSERT_EQUAL(Path::BT_DIRECT, wispr::choosePath(in));
  TEST_ASSERT_TRUE(wispr::streams(Path::BT_DIRECT));
}

void test_usb_mounted_always_wins() {
  auto in = ready();
  in.usbMounted = true;  // plugged into the Mac, Wispr hasn't opened the mic yet
  TEST_ASSERT_EQUAL(Path::USB, wispr::choosePath(in));
  TEST_ASSERT_FALSE(wispr::streams(Path::USB));
  in = ready();
  in.usbListening = true;
  TEST_ASSERT_EQUAL(Path::USB, wispr::choosePath(in));
  in.setting = false;  // even with the wireless switch off: the USB label
  TEST_ASSERT_EQUAL(Path::USB, wispr::choosePath(in));
}

void test_no_stick_mic() {
  auto in = ready();
  in.setting = false;
  TEST_ASSERT_EQUAL(Path::NONE, wispr::choosePath(in));
  in = ready();
  in.agentWants = false;  // wireless_mic off, BlackHole missing, or an older agent
  TEST_ASSERT_EQUAL(Path::NONE, wispr::choosePath(in));
  in = ready();
  in.agentLinked = false;
  TEST_ASSERT_EQUAL(Path::NONE, wispr::choosePath(in));
  in = ready();
  in.micBusy = true;  // a B talk is recording
  TEST_ASSERT_EQUAL(Path::NONE, wispr::choosePath(in));
  TEST_ASSERT_FALSE(wispr::streams(Path::NONE));
}

void test_usb_mounted_needs_vbus() {
  TEST_ASSERT_TRUE(wispr::usbMounted(true, true));
  TEST_ASSERT_FALSE(wispr::usbMounted(true, false));   // unplugged: TinyUSB may not have noticed
  TEST_ASSERT_FALSE(wispr::usbMounted(false, true));   // charger-only cable / power bank
  TEST_ASSERT_FALSE(wispr::usbMounted(false, false));  // battery
}

void test_key_up_tail_only_when_streaming() {
  TEST_ASSERT_EQUAL_UINT32(0, wispr::keyUpDelayMs(false));
  TEST_ASSERT_EQUAL_UINT32(wispr::KEYUP_TAIL_MS, wispr::keyUpDelayMs(true));
  TEST_ASSERT_TRUE(wispr::KEYUP_TAIL_MS > 0 && wispr::KEYUP_TAIL_MS <= 400);  // short enough not to feel laggy
}

void test_gate_key_down_on_ready() {
  Gate g;
  g.begin(1000, 7);
  TEST_ASSERT_TRUE(g.waiting());
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.poll(1100));
  TEST_ASSERT_TRUE(g.ready(7));
  TEST_ASSERT_EQUAL(Gate::Step::PRESS, g.poll(1420));
  TEST_ASSERT_FALSE(g.waiting());
  TEST_ASSERT_FALSE(g.timedOut());
  TEST_ASSERT_EQUAL_UINT32(420, g.waitedMs());
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.poll(1500));  // once
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.release());   // the key is down: the normal key-up
}

void test_gate_times_out_and_presses_anyway() {
  Gate g;
  g.begin(1000, 1);
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.poll(1000 + wispr::READY_TIMEOUT_MS - 1));
  TEST_ASSERT_EQUAL(Gate::Step::PRESS, g.poll(1000 + wispr::READY_TIMEOUT_MS));
  TEST_ASSERT_TRUE(g.timedOut());
  TEST_ASSERT_FALSE(g.ready(1));  // too late: nothing to do
}

void test_gate_ignores_a_ready_for_an_earlier_press() {
  Gate g;
  g.begin(0, 2);
  TEST_ASSERT_FALSE(g.ready(1));
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.poll(100));
  TEST_ASSERT_TRUE(g.ready(2));
  TEST_ASSERT_EQUAL(Gate::Step::PRESS, g.poll(101));
}

void test_gate_release_before_ready_cancels() {
  Gate g;
  g.begin(0, 3);
  TEST_ASSERT_EQUAL(Gate::Step::CANCEL, g.release());  // a quick tap: no key at all
  TEST_ASSERT_FALSE(g.waiting());
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.poll(5000));    // and no key later either
  TEST_ASSERT_FALSE(g.ready(3));
}

void test_gate_survives_millis_wrap() {
  Gate g;
  g.begin(0xFFFFFF00u, 4);
  TEST_ASSERT_EQUAL(Gate::Step::NONE, g.poll(0x00000010u));  // 0x110 ms later
  TEST_ASSERT_EQUAL(Gate::Step::PRESS, g.poll(0xFFFFFF00u + wispr::READY_TIMEOUT_MS));
}

void test_timeout_is_long_enough_for_the_agent() {
  // ~170 ms to create the device + 250 ms switch time + the link both ways
  TEST_ASSERT_TRUE(wispr::READY_TIMEOUT_MS >= 500 && wispr::READY_TIMEOUT_MS <= 1000);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_bluetooth_prepare_when_everything_is_ready);
  RUN_TEST(test_an_agent_without_ready_gets_the_old_flow);
  RUN_TEST(test_usb_mounted_always_wins);
  RUN_TEST(test_no_stick_mic);
  RUN_TEST(test_usb_mounted_needs_vbus);
  RUN_TEST(test_key_up_tail_only_when_streaming);
  RUN_TEST(test_gate_key_down_on_ready);
  RUN_TEST(test_gate_times_out_and_presses_anyway);
  RUN_TEST(test_gate_ignores_a_ready_for_an_earlier_press);
  RUN_TEST(test_gate_release_before_ready_cancels);
  RUN_TEST(test_gate_survives_millis_wrap);
  RUN_TEST(test_timeout_is_long_enough_for_the_agent);
  return UNITY_END();
}
