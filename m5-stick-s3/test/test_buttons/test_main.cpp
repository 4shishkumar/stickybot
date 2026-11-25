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
  TEST_ASSERT_TRUE(parse(rows, map));
  TEST_ASSERT_TRUE(unpack(pack(map), restored));
  TEST_ASSERT_EQUAL_STRING("Music", restored[Slot::CHORD].label.c_str());
  TEST_ASSERT_EQUAL_INT((int)Do::COMBO, (int)restored[Slot::HOLD].action);
  std::string saved = pack(map);
  for (auto invalid : {"none", "unknown", "reply", "app:X"}) {
    rows[3] = invalid;
    TEST_ASSERT_FALSE(parse(rows, map));
    TEST_ASSERT_EQUAL_STRING(saved.c_str(), pack(map).c_str());
  }
  TEST_ASSERT_FALSE(unpack("unlock\nlauncher", map));
  TEST_ASSERT_FALSE(unpack(saved + "\n", map));
}

static void choices_have_their_own_targets() {
  std::string rows[COUNT]{"none", "talk", "reply", "talk"};
  Table t;
  for (int i = 0; i < launcher::TILE_COUNT; ++i) {
    const auto tile = static_cast<launcher::Tile>(i);
    rows[0] = std::string("tile:") + launcher::info(tile).id;
    TEST_ASSERT_TRUE(parse(rows, t));
    TEST_ASSERT_EQUAL_INT(i, (int)t[Slot::DOUBLE].tile);
  }
  for (auto v : {"app:Notes", "quick:Music & news", "app:Café", "none", "talk"}) {
    rows[0] = v;
    TEST_ASSERT_TRUE(parse(rows, t));
    TEST_ASSERT_EQUAL_STRING(v, t[Slot::DOUBLE].wire.c_str());
  }
  for (auto v : {"tile:g", "dictation", "quick:", "app:../evil", "app:a\\b", "app:n\nq", "quick: n", "app:N ", "app:Name\u3000", "quick:\u00a0Name",
                 "app:\xc2\x80", "app:\xc0\x80", "app:\xf4\x90\x80\x80"}) {
    rows[0] = v;
    TEST_ASSERT_FALSE(parse(rows, t));
  }
  rows[0] = "app:" + std::string(32, 'a');
  TEST_ASSERT_TRUE(parse(rows, t));
  rows[0] += "a";
  TEST_ASSERT_FALSE(parse(rows, t));
}

static void hints_use_the_effective_binding() {
  std::string rows[COUNT]{"launcher", "quick:Focus", "tile:games", "dictation"};
  Table t;
  TEST_ASSERT_TRUE(parse(rows, t));
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
  TEST_ASSERT_EQUAL_INT((int)Do::LAUNCHER, (int)actions::lookup(home, actions::In::B_DOUBLE, 0, &t));
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
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(false, false, true, true, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(true, false, false, false, false));
  TEST_ASSERT_EQUAL_INT((int)ChordStep::PASS, (int)c.update(true, true, true, true, true)); // B hold already owns it
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
  RUN_TEST(hints_use_the_effective_binding);
  RUN_TEST(chord_owns_both_releases_and_cancels_on_modal);
  RUN_TEST(talk_follows_the_input_that_started_it);
  return UNITY_END();
}
