// Context actions (logic/actions.cpp): each screen's button map, the state-dependent bindings,
// and the hint bar made from the same table.
#include <unity.h>
#include <stdio.h>
#include <string>
#include "logic/actions.h"

using namespace actions;

void setUp() {}
void tearDown() {}

static int chars(const std::string& s, void*) {
  int n = 0;
  for (unsigned char c : s) if ((c & 0xC0) != 0x80) n++;
  return n * 5;
}

static std::string bar(Id id, uint8_t flags) {
  std::string out;
  for (auto& h : hints(screen(id), flags)) out += (out.empty() ? "" : " | ") + h.key + " " + h.label;
  return out;
}

void test_reply_buttons_follow_its_state() {
  const Screen& r = screen(Id::REPLY);
  TEST_ASSERT_EQUAL((int)Do::NEXT_PAGE, (int)lookup(r, In::A_CLICK, PAGED | SPEECH));
  TEST_ASSERT_EQUAL((int)Do::NONE, (int)lookup(r, In::A_CLICK, SPEECH));  // one page: nothing to turn
  TEST_ASSERT_EQUAL((int)Do::LISTEN, (int)lookup(r, In::A_HOLD, SPEECH));
  TEST_ASSERT_EQUAL((int)Do::STOP_LISTEN, (int)lookup(r, In::A_HOLD, SPEECH | SPEAKING));
  TEST_ASSERT_EQUAL((int)Do::NONE, (int)lookup(r, In::A_HOLD, 0));  // an agent that can't speak
  TEST_ASSERT_EQUAL((int)Do::CLOSE, (int)lookup(r, In::B_CLICK, 0));
  TEST_ASSERT_EQUAL((int)Do::CLOSE, (int)lookup(r, In::SHAKE, PAGED));
  TEST_ASSERT_EQUAL((int)Do::SCROLL, (int)lookup(r, In::TILT, PAGED));
  TEST_ASSERT_EQUAL((int)Do::NONE, (int)lookup(r, In::TILT, 0));
  // the global gestures aren't taken: the caller does what it always does
  TEST_ASSERT_EQUAL((int)Do::NONE, (int)lookup(r, In::B_DOUBLE, PAGED));
  TEST_ASSERT_EQUAL((int)Do::NONE, (int)lookup(r, In::B_TRIPLE, PAGED));
}

void test_agent_screen_cancels_while_busy() {
  const Screen& a = screen(Id::AGENT);
  TEST_ASSERT_EQUAL((int)Do::CANCEL, (int)lookup(a, In::B_CLICK, BUSY));
  TEST_ASSERT_EQUAL((int)Do::CANCEL, (int)lookup(a, In::SHAKE, BUSY));
  TEST_ASSERT_EQUAL((int)Do::CLOSE, (int)lookup(a, In::B_CLICK, 0));
  TEST_ASSERT_EQUAL_STRING("B cancel", bar(Id::AGENT, BUSY).c_str());
  TEST_ASSERT_EQUAL_STRING("B close | hold B again", bar(Id::AGENT, 0).c_str());
}

void test_hint_bars() {
  TEST_ASSERT_EQUAL_STRING("A next | hold A listen | B close | tilt scroll", bar(Id::REPLY, PAGED | SPEECH).c_str());
  TEST_ASSERT_EQUAL_STRING("A next | hold A stop | B close | tilt scroll",
                           bar(Id::REPLY, PAGED | SPEECH | SPEAKING).c_str());
  TEST_ASSERT_EQUAL_STRING("hold A listen | B close | hold B ask", bar(Id::REPLY, SPEECH).c_str());
  TEST_ASSERT_EQUAL_STRING("B close | hold B ask", bar(Id::REPLY, 0).c_str());
  TEST_ASSERT_EQUAL_STRING("hold B talk | A dictate | 3×B apps | B enter", bar(Id::HOME, 0).c_str());
  TEST_ASSERT_EQUAL_STRING("let go send", bar(Id::TALK, 0).c_str());
  auto h = hints(screen(Id::REPLY), PAGED | SPEECH);
  TEST_ASSERT_TRUE(h[1].accent);  // listen stands out
  TEST_ASSERT_FALSE(h[0].accent);
}

void test_every_screen_has_one_action_per_input_and_state() {
  // no two bindings for the same input can apply at once (the first would hide the second)
  for (int id = 0; id < (int)Id::COUNT; id++) {
    const Screen& s = screen((Id)id);
    for (int f = 0; f < 16; f++)
      for (int i = 0; i < s.count; i++)
        for (int j = i + 1; j < s.count; j++) {
          const Binding &a = s.bindings[i], &b = s.bindings[j];
          bool ai = (f & a.need) == a.need && !(f & a.unless), bj = (f & b.need) == b.need && !(f & b.unless);
          if (a.in == b.in && ai && bj) {
            char msg[80];
            snprintf(msg, sizeof msg, "%s: %s twice (flags %d)", s.name, inName(a.in), f);
            TEST_FAIL_MESSAGE(msg);
          }
        }
  }
}

void test_layout_rows() {
  std::vector<Hint> h = {{"A", "next", false}, {"hold A", "listen", true}, {"B", "close", false},
                         {"tilt", "scroll", false}};
  // 5 px a character: "A next" 30, "hold A listen" 65, "B close" 35, "tilt scroll" 55
  auto rows = layout(h, 119, 2, 8, chars, nullptr);  // portrait: 135 - 2 * 8
  TEST_ASSERT_EQUAL(2, (int)rows.size());
  TEST_ASSERT_EQUAL(2, (int)rows[0].size());  // A next + hold A listen = 103
  TEST_ASSERT_EQUAL(0, rows[0][0]);
  TEST_ASSERT_EQUAL(1, rows[0][1]);
  TEST_ASSERT_EQUAL(2, (int)rows[1].size());  // B close + tilt scroll = 98
  auto wide = layout(h, 224, 1, 10, chars, nullptr);  // landscape: one row
  TEST_ASSERT_EQUAL(1, (int)wide.size());
  TEST_ASSERT_EQUAL(4, (int)wide[0].size());  // 30+65+35+55 + 3 gaps = 215
  auto tight = layout(h, 70, 1, 8, chars, nullptr);  // one short row: what fits, in order
  TEST_ASSERT_EQUAL(1, (int)tight.size());
  TEST_ASSERT_EQUAL(1, (int)tight[0].size());
  TEST_ASSERT_EQUAL(0, tight[0][0]);
  TEST_ASSERT_EQUAL(0, (int)layout(h, 20, 2, 8, chars, nullptr).size());  // nothing fits
}

void test_names() {
  TEST_ASSERT_EQUAL_STRING("hold A", inName(In::A_HOLD));
  TEST_ASSERT_EQUAL_STRING("next page", doName(Do::NEXT_PAGE));
  TEST_ASSERT_EQUAL_STRING("send", doName(Do::SEND));
  TEST_ASSERT_EQUAL_STRING("flick", inName(In::FLICK));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_reply_buttons_follow_its_state);
  RUN_TEST(test_agent_screen_cancels_while_busy);
  RUN_TEST(test_hint_bars);
  RUN_TEST(test_every_screen_has_one_action_per_input_and_state);
  RUN_TEST(test_layout_rows);
  RUN_TEST(test_names);
  return UNITY_END();
}
