// Pet progress (levels, streak, unlocks), moods and the world's changes (faces).
#include <unity.h>
#include <string.h>
#include "logic/pet.h"
#include "logic/world.h"

void setUp() {}
void tearDown() {}

void test_levels() {
  TEST_ASSERT_EQUAL(1, pet::levelFor(0).level);
  TEST_ASSERT_EQUAL(20, pet::levelFor(0).need);
  TEST_ASSERT_EQUAL(1, pet::levelFor(19).level);
  TEST_ASSERT_EQUAL(19, pet::levelFor(19).into);
  TEST_ASSERT_EQUAL(2, pet::levelFor(20).level);       // 20 for level 2
  TEST_ASSERT_EQUAL(0, pet::levelFor(20).into);
  TEST_ASSERT_EQUAL(30, pet::levelFor(20).need);
  TEST_ASSERT_EQUAL(3, pet::levelFor(50).level);       // + 30
  TEST_ASSERT_EQUAL(4, pet::levelFor(90).level);       // + 40
  TEST_ASSERT_EQUAL(1, pet::levelFor(-5).level);
  pet::Level l = pet::levelFor(1000000);               // huge: still sane
  TEST_ASSERT_TRUE(l.level > 10 && l.into < l.need);
}

void test_accessories() {
  TEST_ASSERT_EQUAL(0, pet::accessories(4, 9, 99));
  TEST_ASSERT_EQUAL(pet::HAT, pet::accessories(5, 0, 0));
  TEST_ASSERT_EQUAL(pet::SHADES, pet::accessories(0, 10, 0));
  TEST_ASSERT_EQUAL(pet::CROWN, pet::accessories(0, 0, 100));
  TEST_ASSERT_EQUAL(pet::HAT | pet::SHADES | pet::CROWN, pet::accessories(30, 50, 300));
}

void test_streak() {
  pet::Streak s;
  TEST_ASSERT_EQUAL(0, pet::shownStreak(s, 100));
  pet::markActive(s, 100);
  pet::markActive(s, 100);  // twice the same day counts once
  TEST_ASSERT_EQUAL(1, s.days);
  pet::markActive(s, 101);
  pet::markActive(s, 102);
  TEST_ASSERT_EQUAL(3, s.days);
  TEST_ASSERT_EQUAL(3, pet::shownStreak(s, 103));  // yesterday still counts
  TEST_ASSERT_EQUAL(0, pet::shownStreak(s, 104));  // a whole day missed
  pet::markActive(s, 105);
  TEST_ASSERT_EQUAL(1, s.days);
  TEST_ASSERT_EQUAL(3, s.best);
}

void test_steady_mood_priority() {
  using pet::Mood;
  pet::MoodInputs in{false, false, false, 0, false};
  TEST_ASSERT_EQUAL((int)Mood::IDLE, (int)pet::steadyMood(in));
  in.ciGreen = true;
  TEST_ASSERT_EQUAL((int)Mood::HAPPY, (int)pet::steadyMood(in));
  in.night = true;
  TEST_ASSERT_EQUAL((int)Mood::SLEEPY, (int)pet::steadyMood(in));
  in.claudeRunning = 2;
  TEST_ASSERT_EQUAL((int)Mood::WORKING, (int)pet::steadyMood(in));
  in.ciRed = true;
  TEST_ASSERT_EQUAL((int)Mood::SAD, (int)pet::steadyMood(in));
  in.urgent = true;
  TEST_ASSERT_EQUAL((int)Mood::ALERT, (int)pet::steadyMood(in));
  TEST_ASSERT_EQUAL(0, (int)pet::reactionMs(Mood::ALERT));
  TEST_ASSERT_TRUE(pet::reactionMs(Mood::EATING) > 1000);
}

static world::Snapshot base() {
  world::Snapshot w;
  w.valid = true;
  w.merged = 10;
  w.xp = 15;
  w.ci = world::Ci::GREEN;
  w.nSessions = 1;
  w.sessions[0].id = 7;
  world::setProject(w.sessions[0], "a-very-long-project-name");
  return w;
}

void test_project_is_cut() {
  world::Snapshot w = base();
  TEST_ASSERT_EQUAL(12, (int)strlen(w.sessions[0].project));
  TEST_ASSERT_EQUAL((int)world::Ci::RED, (int)world::parseCi("red"));
  TEST_ASSERT_EQUAL((int)world::Ci::NONE, (int)world::parseCi(nullptr));
}

void test_diff() {
  world::Kick k[8];
  world::Snapshot a = base(), b = base();
  TEST_ASSERT_EQUAL(0, world::diff(a, b, k, 8));
  world::Snapshot none;
  TEST_ASSERT_EQUAL(0, world::diff(none, b, k, 8));  // the first message isn't news

  b.merged = 11;
  b.xp = 25;  // level 1 -> 2
  b.ci = world::Ci::RED;
  b.sessions[0].id = 8;  // 7 finished, 8 started
  int n = world::diff(a, b, k, 8);
  TEST_ASSERT_EQUAL(5, n);
  TEST_ASSERT_EQUAL((int)world::Kick::LEVEL_UP, (int)k[0]);
  TEST_ASSERT_EQUAL((int)world::Kick::MERGED, (int)k[1]);
  TEST_ASSERT_EQUAL((int)world::Kick::CI_RED, (int)k[2]);
  TEST_ASSERT_EQUAL((int)world::Kick::CLAUDE_DONE, (int)k[3]);
  TEST_ASSERT_EQUAL((int)world::Kick::CLAUDE_START, (int)k[4]);
  TEST_ASSERT_EQUAL(2, world::diff(a, b, k, 2));  // capped

  world::Snapshot c = b;
  c.ci = world::Ci::GREEN;  // fixed
  c.nTowers = 1;
  c.towers[0] = 11;
  n = world::diff(b, c, k, 8);
  TEST_ASSERT_EQUAL(2, n);
  TEST_ASSERT_EQUAL((int)world::Kick::RELEASE, (int)k[0]);
  TEST_ASSERT_EQUAL((int)world::Kick::CI_GREEN, (int)k[1]);
}

void test_event_kinds() {
  TEST_ASSERT_EQUAL((int)world::Kick::MERGED, (int)world::kickForEvent("pr_merged"));
  TEST_ASSERT_EQUAL((int)world::Kick::REVIEW, (int)world::kickForEvent("review_requested"));
  TEST_ASSERT_EQUAL((int)world::Kick::NONE, (int)world::kickForEvent("pr_created"));
  TEST_ASSERT_EQUAL((int)world::Kick::NONE, (int)world::kickForEvent(nullptr));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_levels);
  RUN_TEST(test_accessories);
  RUN_TEST(test_streak);
  RUN_TEST(test_steady_mood_priority);
  RUN_TEST(test_project_is_cut);
  RUN_TEST(test_diff);
  RUN_TEST(test_event_kinds);
  return UNITY_END();
}
