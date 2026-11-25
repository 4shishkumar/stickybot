#include <unity.h>
#include "logic/button_map.h"
using namespace buttons;
using actions::Do;

void setUp() {}
void tearDown() {}

static void remaps_are_atomic_and_round_trip() {
  Table map = defaults(), restored;
  TEST_ASSERT_EQUAL_STRING("unlock\nlauncher\nreply\ntalk", pack(map).c_str());
  std::string rows[COUNT]{"launcher", "unlock", "app:Music", "dictation"};
  TEST_ASSERT_TRUE(parse(rows, map) == ParseError::NONE);
  TEST_ASSERT_TRUE(unpack(pack(map), restored));
  TEST_ASSERT_EQUAL_STRING("Music", restored[Slot::CHORD].label.c_str());
  TEST_ASSERT_EQUAL_INT((int)Do::COMBO, (int)restored[Slot::HOLD].action);
  std::string saved = pack(map);
  for (auto invalid : {"none", "unknown", "reply", "app:X"}) {
    rows[3] = invalid;
    TEST_ASSERT_TRUE(parse(rows, map) == ParseError::BINDING);
    TEST_ASSERT_EQUAL_STRING(saved.c_str(), pack(map).c_str());
  }
  TEST_ASSERT_FALSE(unpack("unlock\nlauncher", map));
  TEST_ASSERT_FALSE(unpack(saved + "\n", map));
}

static void choices_have_their_own_targets() {
  std::string rows[COUNT]{"none", "launcher", "reply", "talk"};
  Table t;
  for (int i = 0; i < launcher::TILE_COUNT; ++i) {
    const auto tile = static_cast<launcher::Tile>(i);
    rows[0] = std::string("tile:") + launcher::info(tile).id;
    TEST_ASSERT_TRUE(parse(rows, t) == ParseError::NONE);
    TEST_ASSERT_EQUAL_INT(i, (int)t[Slot::DOUBLE].tile);
  }
  for (auto v : {"app:Notes", "quick:Music & news", "app:Café", "none", "talk"}) {
    rows[0] = v;
    TEST_ASSERT_TRUE(parse(rows, t) == ParseError::NONE);
    TEST_ASSERT_EQUAL_STRING(v, t[Slot::DOUBLE].wire.c_str());
  }
  for (auto v : {"tile:g", "dictation", "quick:", "app:../evil", "app:a\\b", "app:n\nq", "quick: n", "app:N ", "app:Name\u3000", "quick:\u00a0Name",
                 "app:\xc2\x80", "app:\xc0\x80", "app:\xf4\x90\x80\x80"}) {
    rows[0] = v;
    TEST_ASSERT_TRUE(parse(rows, t) == ParseError::BINDING);
  }
  rows[0] = "app:" + std::string(32, 'a');
  TEST_ASSERT_TRUE(parse(rows, t) == ParseError::NONE);
  rows[0] += "a";
  TEST_ASSERT_TRUE(parse(rows, t) == ParseError::BINDING);
}

static void settings_must_stay_reachable_after_save_and_reboot() {
  Table t = defaults();
  const std::string initial = pack(t);
  std::string rows[COUNT]{"unlock", "tile:games", "reply", "talk"};
  TEST_ASSERT_TRUE(parse(rows, t) == ParseError::NO_SETTINGS);
  TEST_ASSERT_EQUAL_STRING(initial.c_str(), pack(t).c_str());
  TEST_ASSERT_FALSE(unpack("app:Settings\nquick:Launcher\ntile:quick\ndictation", t));
  TEST_ASSERT_EQUAL_STRING(initial.c_str(), pack(t).c_str());
  for (int i = 0; i < 3; ++i) {
    const std::string saved = rows[i];
    for (const char* route : {"launcher", "tile:settings"}) {
      rows[i] = route;
      TEST_ASSERT_TRUE(parse(rows, t) == ParseError::NONE);
      TEST_ASSERT_TRUE(unpack(pack(t), t));
    }
    rows[i] = saved;
  }
}

static void chord_talk_hint_matches_the_gesture_name() {
  std::string rows[COUNT]{"unlock", "launcher", "talk", "dictation"};
  Table map = defaults();
  TEST_ASSERT_TRUE(parse(rows, map) == ParseError::NONE);
  TEST_ASSERT_EQUAL_STRING("A+B to ask", talkHint(map));
}

static void hints_use_the_effective_binding() {
  std::string rows[COUNT]{"launcher", "quick:Focus", "tile:games", "dictation"};
  Table t;
  TEST_ASSERT_TRUE(parse(rows, t) == ParseError::NONE);
  auto& home = actions::screen(actions::Id::HOME);
  auto hints = actions::hints(home, 0, &t);
  auto label = [&](const char* key) {
    for (const auto& h : hints) if (h.key == key) return h.label;
    return std::string();
  };
  TEST_ASSERT_EQUAL_STRING("apps", label("2×B").c_str());
  TEST_ASSERT_EQUAL_STRING("Focus", label("3×B").c_str());
  TEST_ASSERT_EQUAL_STRING("dictate", label("hold B").c_str());
  TEST_ASSERT_EQUAL_STRING("Games", label("A+B").c_str());
  TEST_ASSERT_EQUAL_INT((int)Do::LAUNCHER, (int)t[Slot::DOUBLE].action);
  auto talk = actions::hints(actions::screen(actions::Id::TALK), actions::LATCHED, &t);
  TEST_ASSERT_EQUAL_STRING("B", talk[0].key.c_str());
}

static void chord_owns_both_releases_and_cancels_on_modal() {
  Chord c;
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(true, true, true, false, false)); // A alone
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(true, false, true, true, false)); // A was already held
  TEST_ASSERT_EQUAL_INT((int)ChordStep::CONSUME, (int)c.update(true, true, true, true, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::CONSUME, (int)c.update(true, false, false, true, true));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::FIRE, (int)c.update(true, false, false, false, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(true, false, false, false, false));
  c.update(true, true, true, true, false);
  TEST_ASSERT_EQUAL_INT((int)ChordStep::CONSUME, (int)c.update(false, false, true, true, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::CONSUME, (int)c.update(true, false, true, false, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::DISCARD, (int)c.update(true, false, false, false, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(true, true, true, true, true)); // B hold already owns it
}

// B releases first. Its delayed click decision can arrive long before A is released.
static void delayed_click_of_chord_cannot_cancel_or_close_reply() {
  for (auto screen : {actions::Id::AGENT, actions::Id::REPLY}) {
    Chord c;
    c.update(true, true, true, true, false);
    c.update(true, false, true, false, false);
    auto delayed = c.update(true, false, true, false, false);
    auto action = delayed == ChordStep::PASS
        ? actions::lookup(actions::screen(screen), actions::In::B_CLICK, actions::BUSY) : Do::NONE;
    TEST_ASSERT_EQUAL_INT((int)Do::NONE, (int)action);
    TEST_ASSERT_EQUAL_INT((int)ChordStep::FIRE, (int)c.update(true, false, false, false, false));
  }
}

static void talk_follows_the_input_that_started_it() {
  TEST_ASSERT_TRUE(finishTalk(false, false, false, true, 50)); // normal B release
  TEST_ASSERT_FALSE(finishTalk(true, false, false, true, 50)); // discrete gesture already released
  TEST_ASSERT_TRUE(finishTalk(true, true, true, true, 50)); // next press
  TEST_ASSERT_TRUE(finishTalk(true, false, false, false, 50)); // disconnected
  TEST_ASSERT_TRUE(finishTalk(true, false, false, true, TAP_TALK_LIMIT_MS));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(remaps_are_atomic_and_round_trip);
  RUN_TEST(choices_have_their_own_targets);
  RUN_TEST(settings_must_stay_reachable_after_save_and_reboot);
  RUN_TEST(chord_talk_hint_matches_the_gesture_name);
  RUN_TEST(hints_use_the_effective_binding);
  RUN_TEST(chord_owns_both_releases_and_cancels_on_modal);
  RUN_TEST(delayed_click_of_chord_cannot_cancel_or_close_reply);
  RUN_TEST(talk_follows_the_input_that_started_it);
  return UNITY_END();
}
