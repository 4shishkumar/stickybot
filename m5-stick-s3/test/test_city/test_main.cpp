// StickyWorld's city: deterministic buildings, towers, and a skyline that always fits.
#include <unity.h>
#include "logic/city.h"

void setUp() {}
void tearDown() {}

void test_same_seed_same_city() {
  for (int i = 0; i < 50; i++) {
    city::Building a = city::building(42, i), b = city::building(42, i);
    TEST_ASSERT_EQUAL(a.w, b.w);
    TEST_ASSERT_EQUAL(a.h, b.h);
    TEST_ASSERT_EQUAL(a.roof, b.roof);
    TEST_ASSERT_EQUAL(a.windows, b.windows);
    TEST_ASSERT_TRUE(a.w >= 3 && a.w <= 6);
    TEST_ASSERT_TRUE(a.h >= 3 && a.h <= 12);
    TEST_ASSERT_TRUE(a.color < 6 && a.roof < city::ROOF_COUNT);
    TEST_ASSERT_FALSE(a.tower);
  }
  int differ = 0;  // another seed, another skyline
  for (int i = 0; i < 20; i++) differ += city::building(42, i).h != city::building(43, i).h;
  TEST_ASSERT_TRUE(differ > 5);
  city::Building t = city::tower(42, 0);
  TEST_ASSERT_TRUE(t.tower);
  TEST_ASSERT_TRUE(t.h >= 15);
}

void test_varied_heights() {
  int lo = 99, hi = 0;
  for (int i = 0; i < 40; i++) {
    int h = city::building(7, i).h;
    lo = h < lo ? h : lo;
    hi = h > hi ? h : hi;
  }
  TEST_ASSERT_TRUE(hi - lo >= 6);
}

void test_few_fit_centred_with_gaps() {
  city::Placed p[64];
  int n = city::layout(1, 3, nullptr, 0, 135, 2, p, 64);
  TEST_ASSERT_EQUAL(3, n);
  int right = p[2].x + p[2].w;
  TEST_ASSERT_TRUE(p[0].x >= 0 && right <= 135);
  TEST_ASSERT_INT_WITHIN(2, 135 - right, p[0].x);  // centred
  TEST_ASSERT_EQUAL(p[0].x + p[0].w + 2, p[1].x);  // a 1-cell gap
  TEST_ASSERT_EQUAL(city::building(1, 0).w * 2, p[0].w);
  TEST_ASSERT_EQUAL(0, city::layout(1, 0, nullptr, 0, 135, 2, p, 64));
}

void test_towers_go_after_their_building() {
  city::Placed p[64];
  const int towers[] = {2};
  int n = city::layout(5, 4, towers, 1, 240, 2, p, 64);
  TEST_ASSERT_EQUAL(5, n);
  TEST_ASSERT_FALSE(p[1].b.tower);
  TEST_ASSERT_TRUE(p[2].b.tower);
  TEST_ASSERT_FALSE(p[3].b.tower);
  const int late[] = {9};  // a release recorded after more buildings than there are: at the end
  n = city::layout(5, 4, late, 1, 240, 2, p, 64);
  TEST_ASSERT_TRUE(p[n - 1].b.tower);
}

void test_many_squeeze_then_scroll() {
  city::Placed p[96];
  // squeezed: all of them, no overlap, inside the width
  int n = city::layout(3, 25, nullptr, 0, 135, 2, p, 96);
  TEST_ASSERT_EQUAL(25, n);
  for (int i = 1; i < n; i++) TEST_ASSERT_TRUE(p[i].x >= p[i - 1].x + p[i - 1].w);
  TEST_ASSERT_TRUE(p[0].x >= 0 && p[n - 1].x + p[n - 1].w <= 135);
  for (int i = 0; i < n; i++) TEST_ASSERT_TRUE(p[i].w >= city::MIN_W * 2);
  // too many to squeeze: the newest show, the oldest scrolled off
  n = city::layout(3, 300, nullptr, 0, 135, 2, p, 96);
  TEST_ASSERT_TRUE(n > 10 && n < 300);
  TEST_ASSERT_TRUE(p[n - 1].x + p[n - 1].w <= 135);
  city::Building newest = city::building(3, 299);
  TEST_ASSERT_EQUAL(newest.h, p[n - 1].b.h);
  TEST_ASSERT_EQUAL(newest.windows, p[n - 1].b.windows);
  // stable: the same call, the same picture
  city::Placed q[96];
  int m = city::layout(3, 300, nullptr, 0, 135, 2, q, 96);
  TEST_ASSERT_EQUAL(n, m);
  TEST_ASSERT_EQUAL(p[5].x, q[5].x);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_same_seed_same_city);
  RUN_TEST(test_varied_heights);
  RUN_TEST(test_few_fit_centred_with_gaps);
  RUN_TEST(test_towers_go_after_their_building);
  RUN_TEST(test_many_squeeze_then_scroll);
  return UNITY_END();
}
