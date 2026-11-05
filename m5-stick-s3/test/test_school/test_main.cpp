// The Aquarium: fish per Claude session (spawn, grow, leap out when done) and the tilt spring.
#include <unity.h>
#include <string.h>
#include "logic/school.h"
#include "logic/spring.h"

void setUp() {}
void tearDown() {}

static void run(school::School& s, uint32_t& now, uint32_t ms, float drift = 0, bool sleepy = false) {
  for (uint32_t t = 0; t < ms; t += 40) {
    now += 40;
    s.step(0.04f, drift, sleepy, now);
  }
}

void test_ambient_fish_are_always_there() {
  school::School s;
  s.begin(1, 120, 150, 0);
  TEST_ASSERT_EQUAL(school::AMBIENT, s.count());
  TEST_ASSERT_EQUAL(0, s.sessions());
  uint32_t now = 0;
  run(s, now, 5000);
  TEST_ASSERT_EQUAL(school::AMBIENT, s.count());
}

void test_sessions_spawn_grow_and_leap_out() {
  school::School s;
  s.begin(2, 120, 150, 0);
  uint32_t now = 1000;
  school::Session a[] = {{11, "web", 30}, {12, "api", 4000}};
  s.sync(a, 2, now);
  TEST_ASSERT_EQUAL(2, s.sessions());
  const school::Fish* web = nullptr;
  for (int i = 0; i < s.size(); i++)
    if (s.at(i).on && s.at(i).id == 11) web = &s.at(i);
  TEST_ASSERT_NOT_NULL(web);
  TEST_ASSERT_EQUAL_STRING("web", web->label);
  TEST_ASSERT_EQUAL(school::hueFor("web"), web->hue);
  float small = web->size;
  a[0].secs = 3000;
  s.sync(a, 2, now);  // the same session later: the same fish, bigger
  TEST_ASSERT_EQUAL(2, s.sessions());
  TEST_ASSERT_TRUE(web->size > small);
  run(s, now, 6000);
  TEST_ASSERT_TRUE(web->x > 0 && web->x < 120);  // swam in
  // web finishes: its fish jumps, then it's gone
  s.sync(&a[1], 1, now);
  TEST_ASSERT_EQUAL((int)school::State::JUMP, (int)web->st);
  run(s, now, 800);
  TEST_ASSERT_TRUE(school::School::jumpPhase(*web, now) > 0);
  TEST_ASSERT_TRUE(web->y < 0);  // in the air
  run(s, now, school::JUMP_MS);
  TEST_ASSERT_EQUAL(1, s.sessions());
  TEST_ASSERT_EQUAL(1 + school::AMBIENT, s.count());
}

void test_full_tank() {
  school::School s;
  s.begin(3, 120, 150, 0);
  school::Session many[10];
  for (int i = 0; i < 10; i++) many[i] = {(uint16_t)(100 + i), "p", 60};
  s.sync(many, 10, 0);
  TEST_ASSERT_EQUAL(school::MAX_FISH, s.count());
  TEST_ASSERT_EQUAL(school::MAX_FISH - school::AMBIENT, s.sessions());
  s.clear(0);
  TEST_ASSERT_EQUAL(school::AMBIENT, s.count());
}

void test_tilt_drifts_them_to_the_low_side() {
  school::School s;
  s.begin(4, 200, 150, 0);
  school::Session a[] = {{1, "a", 60}, {2, "b", 60}, {3, "c", 60}};
  s.sync(a, 3, 0);
  uint32_t now = 0;
  run(s, now, 5000);
  run(s, now, 8000, 1.0f);  // tipped to the right
  float sum = 0;
  int n = 0;
  for (int i = 0; i < s.size(); i++)
    if (s.at(i).on) { sum += s.at(i).x; n++; }
  TEST_ASSERT_TRUE(sum / n > 120);
}

void test_hue_and_size() {
  TEST_ASSERT_EQUAL(school::hueFor("web"), school::hueFor("web"));
  TEST_ASSERT_TRUE(school::hueFor("whatever") < school::HUES);
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 0.8f, school::sizeFor(0));
  TEST_ASSERT_FLOAT_WITHIN(0.01f, 1.6f, school::sizeFor(7200));
  TEST_ASSERT_TRUE(school::sizeFor(600) < school::sizeFor(1800));
}

void test_spring_settles() {
  spring::Spring sp;
  for (int i = 0; i < 100; i++) sp.step(1.0f, 0.02f);  // 2 s
  TEST_ASSERT_FLOAT_WITHIN(0.02f, 1.0f, sp.x);
  float peak = 0;
  spring::Spring q;
  for (int i = 0; i < 200; i++) { q.step(1.0f, 0.01f); peak = q.x > peak ? q.x : peak; }
  TEST_ASSERT_TRUE(peak < 1.1f);  // barely overshoots
  // a long frame moves it like many short ones
  spring::Spring a, b;
  a.step(1.0f, 0.2f);
  for (int i = 0; i < 10; i++) b.step(1.0f, 0.02f);
  TEST_ASSERT_FLOAT_WITHIN(0.02f, b.x, a.x);
  spring::Spring c;
  c.step(1.0f, 50.0f);  // after the screen was off: no fling
  TEST_ASSERT_TRUE(c.x <= 1.1f);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_ambient_fish_are_always_there);
  RUN_TEST(test_sessions_spawn_grow_and_leap_out);
  RUN_TEST(test_full_tank);
  RUN_TEST(test_tilt_drifts_them_to_the_low_side);
  RUN_TEST(test_hue_and_size);
  RUN_TEST(test_spring_settles);
  return UNITY_END();
}
