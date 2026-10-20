// The event queue: first in, first out, dropping the oldest when full.
#include <unity.h>
#include <string>
#include "logic/fixed_queue.h"

void setUp() {}
void tearDown() {}

void test_fifo() {
  FixedQueue<int, 5> q;
  int v;
  TEST_ASSERT_TRUE(q.empty());
  TEST_ASSERT_FALSE(q.pop(v));
  q.push(1); q.push(2); q.push(3);
  TEST_ASSERT_EQUAL(3, (int)q.size());
  TEST_ASSERT_TRUE(q.pop(v)); TEST_ASSERT_EQUAL(1, v);
  TEST_ASSERT_TRUE(q.pop(v)); TEST_ASSERT_EQUAL(2, v);
  q.push(4);
  TEST_ASSERT_TRUE(q.pop(v)); TEST_ASSERT_EQUAL(3, v);
  TEST_ASSERT_TRUE(q.pop(v)); TEST_ASSERT_EQUAL(4, v);
  TEST_ASSERT_TRUE(q.empty());
}

void test_full_drops_oldest() {
  FixedQueue<std::string, 3> q;
  for (const char* s : {"a", "b", "c", "d", "e"}) q.push(s);
  TEST_ASSERT_EQUAL(3, (int)q.size());
  TEST_ASSERT_EQUAL(3, (int)q.capacity());
  std::string v;
  q.pop(v); TEST_ASSERT_EQUAL_STRING("c", v.c_str());
  q.pop(v); TEST_ASSERT_EQUAL_STRING("d", v.c_str());
  q.pop(v); TEST_ASSERT_EQUAL_STRING("e", v.c_str());
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_fifo);
  RUN_TEST(test_full_drops_oldest);
  return UNITY_END();
}
