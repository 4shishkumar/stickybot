// A small fixed-size FIFO that drops the oldest item when full (the stick's
// event queue). Header-only, native-testable.
#pragma once
#include <stddef.h>

template <typename T, size_t N>
class FixedQueue {
 public:
  void push(const T& v) {
    if (n == N) {  // full: drop the oldest
      for (size_t i = 1; i < n; i++) items[i - 1] = items[i];
      n--;
    }
    items[n++] = v;
  }
  bool pop(T& out) {
    if (!n) return false;
    out = items[0];
    for (size_t i = 1; i < n; i++) items[i - 1] = items[i];
    n--;
    return true;
  }
  size_t size() const { return n; }
  bool   empty() const { return n == 0; }
  static constexpr size_t capacity() { return N; }

 private:
  T      items[N];
  size_t n = 0;
};
