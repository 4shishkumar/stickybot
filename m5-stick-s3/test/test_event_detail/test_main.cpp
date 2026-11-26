#include <unity.h>
#include "logic/event_detail.h"

using events::Detail;
using R = events::InputResult;

void toast_press_only_enters_detail() {
  Detail v;
  TEST_ASSERT_TRUE(v.input(true, false, false, false, true, true) == R::DETAIL);
  TEST_ASSERT_TRUE(v.input(false, false, true, false, true, true) == R::NONE);
  TEST_ASSERT_TRUE(v.input(false, true, false, false, true, true) == R::NONE);
  TEST_ASSERT_TRUE(v.input(false, false, false, true, true, true) == R::CLOSE);
}

void detail_tap_opens_once_and_hold_reads_once() {
  Detail v;
  v.shown = true;
  for (int i = 0; i < 2; ++i) {
    v.input(true, false, false, false, true, true);
    TEST_ASSERT_TRUE(v.input(false, false, true, false, true, true) == (i ? R::NONE : R::READ));
    TEST_ASSERT_TRUE(v.input(false, true, false, false, true, true) == R::NONE);
    v.input(true, false, false, false, true, true);
    TEST_ASSERT_TRUE(v.input(false, true, false, false, true, true) == (i ? R::NONE : R::OPEN));
  }
}

void missing_link_or_capability_disables_action_and_hint() {
  Detail v;
  v.shown = true;
  v.input(true, false, false, false, true, false);
  TEST_ASSERT_TRUE(v.input(false, false, true, false, true, false) == R::NONE);
  TEST_ASSERT_TRUE(v.input(false, true, false, false, true, false) == R::NONE);
  v.input(true, false, false, false, false, true);
  TEST_ASSERT_TRUE(v.input(false, true, false, false, false, true) == R::NONE);
  const auto hints = actions::hints(actions::screen(actions::Id::EVENT_DETAIL), 0);
  TEST_ASSERT_EQUAL(1, hints.size());
  TEST_ASSERT_EQUAL_STRING("close", hints[0].label.c_str());
}

void a_long_message_says_tilt_scrolls() {
  const auto& screen = actions::screen(actions::Id::EVENT_DETAIL);
  TEST_ASSERT_TRUE(actions::lookup(screen, actions::In::TILT, actions::PAGED) == actions::Do::SCROLL);
  TEST_ASSERT_TRUE(actions::lookup(screen, actions::In::TILT, 0) == actions::Do::NONE);
  bool says = false;
  for (const auto& h : actions::hints(screen, actions::PAGED)) says |= h.label == "scroll";
  TEST_ASSERT_TRUE(says);
}

void source_is_same_ble_connection_and_host_session_even_after_focus_switch() {
  hosts::Registry peers;
  peers.hello(1,"bond-a",true,"a","Laptop A","A","old",1);
  peers.hello(2,"bond-b",true,"b","Laptop B","B","other",1);
  auto lease = peers.lease(1);
  peers.select("b", false);
  TEST_ASSERT_TRUE(events::sameSender(peers, true, lease, 9, 1, 9));
  TEST_ASSERT_FALSE(events::sameSender(peers, true, lease, 9, 2, 9));
  peers.hello(1,"bond-a",true,"a","Laptop A","A","new",2);
  TEST_ASSERT_FALSE(events::sameSender(peers, true, lease, 9, 1, 9));
  TEST_ASSERT_TRUE(events::sameSender(peers, false, lease, 9, 1, 9));
  TEST_ASSERT_FALSE(events::sameSender(peers, false, lease, 9, 1, 10));
  TEST_ASSERT_FALSE(events::sameSender(peers, false, {}, 0, hosts::NONE, 0));
}

void urls_and_bounded_utf8_fields() {
  for (const char* url : {"https://example.org/repo?q=x", "http://[::1]/"}) TEST_ASSERT_TRUE(events::webLink(url));
  for (const char* url : {"", "file:///etc", "https://", "https:///a", "http://x/\ny", "https://x/a b"})
    TEST_ASSERT_FALSE(events::webLink(url));
  std::string s = "Aé😀Z";
  TEST_ASSERT_EQUAL(7, events::prefixBytes(s.data(), s.size(), 3));
  TEST_ASSERT_EQUAL(8, events::prefixBytes(s.data(), s.size(), 4));
  TEST_ASSERT_EQUAL(0, events::prefixBytes("a\0b",3,24));
  TEST_ASSERT_EQUAL(0, events::prefixBytes("\xed\xa0\x80",3,24));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(toast_press_only_enters_detail);
  RUN_TEST(detail_tap_opens_once_and_hold_reads_once);
  RUN_TEST(missing_link_or_capability_disables_action_and_hint);
  RUN_TEST(source_is_same_ble_connection_and_host_session_even_after_focus_switch);
  RUN_TEST(urls_and_bounded_utf8_fields);
  RUN_TEST(a_long_message_says_tilt_scrolls);
  return UNITY_END();
}
