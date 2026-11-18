#include <unity.h>
#include "../../lib/HijelHID_BLEKeyboard/src/HeldReports.h"

void setUp() {}
void tearDown() {}

static void holdAll(hidreports::Held& held) {
  held.keyboard[0] = 0x02;  // Shift
  held.keyboard[1] = 0x01;  // Fn
  held.keyboard[2] = 0x04;  // A
  held.consumer = true;
  held.mouse = 0x03;       // left and right buttons
}

static void assertReleased(const hidreports::Held& held) {
  for (auto key : held.keyboard) TEST_ASSERT_EQUAL_UINT8(0, key);
  TEST_ASSERT_FALSE(held.consumer);
  TEST_ASSERT_EQUAL_UINT8(0, held.mouse);
}

void test_disconnected_host_cannot_carry_buttons_to_new_host_movement() {
  hidreports::Held held;
  holdAll(held);
  auto lost = held.release(false);  // A disappeared while a drag was held
  TEST_ASSERT_FALSE(lost.keyboard);
  TEST_ASSERT_FALSE(lost.consumer);
  TEST_ASSERT_FALSE(lost.mouse);
  assertReleased(held);

  held.release(false);             // selecting B after A is already disconnected
  auto movement = held.mouseReport(7, -2, 1, 0);
  TEST_ASSERT_EQUAL_UINT8(0, movement[0]);  // B must not see A's drag
  TEST_ASSERT_EQUAL_UINT8(7, movement[1]);
  TEST_ASSERT_EQUAL_UINT8((uint8_t)-2, movement[2]);
  TEST_ASSERT_EQUAL_UINT8(1, movement[3]);
}

void test_connected_switch_releases_all_held_reports_before_new_input() {
  hidreports::Held held;
  holdAll(held);
  auto oldHost = held.release(true);
  TEST_ASSERT_TRUE(oldHost.keyboard);
  TEST_ASSERT_TRUE(oldHost.consumer);
  TEST_ASSERT_TRUE(oldHost.mouse);
  assertReleased(held);
  TEST_ASSERT_EQUAL_UINT8(0, held.mouseReport(0, 0, 0, 0)[0]);

  held.mouse = 0x04;               // a new middle-button press belongs only to B
  TEST_ASSERT_EQUAL_UINT8(0x04, held.mouseReport(1, 0, 0, 0)[0]);
}

void test_idle_release_avoids_spurious_consumer_and_mouse_notifications() {
  hidreports::Held held;
  auto idle = held.release(true);
  TEST_ASSERT_TRUE(idle.keyboard);
  TEST_ASSERT_FALSE(idle.consumer);
  TEST_ASSERT_FALSE(idle.mouse);
  assertReleased(held);
}

int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(test_disconnected_host_cannot_carry_buttons_to_new_host_movement);
  RUN_TEST(test_connected_switch_releases_all_held_reports_before_new_input);
  RUN_TEST(test_idle_release_avoids_spurious_consumer_and_mouse_notifications);
  return UNITY_END();
}
