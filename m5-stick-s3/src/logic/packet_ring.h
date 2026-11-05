// A fixed-size queue of variable-length packets ([len u16][bytes], wrapping),
// for handing Bluetooth writes from the BLE task to the main loop without
// allocating. No locking here: the caller guards push / pop. Header-only,
// native-testable.
#pragma once
#include <stddef.h>
#include <stdint.h>

template <size_t N>
class PacketRing {
 public:
  // false (and nothing stored) when it doesn't fit
  bool push(const uint8_t* p, uint16_t len) {
    if (!len || used_ + 2 + (size_t)len > N) return false;
    put((uint8_t)(len & 0xFF));
    put((uint8_t)(len >> 8));
    for (uint16_t i = 0; i < len; i++) put(p[i]);
    return true;
  }
  // the oldest packet into out (up to max bytes; a longer one is dropped); false when empty
  bool pop(uint8_t* out, size_t max, uint16_t& len) {
    while (used_) {
      uint8_t lo = take();
      len = (uint16_t)(lo | take() << 8);
      bool fits = len <= max;
      for (uint16_t i = 0; i < len; i++) {
        uint8_t b = take();
        if (fits) out[i] = b;
      }
      if (fits) return true;
    }
    return false;
  }
  size_t used() const { return used_; }
  size_t free() const { return N - used_; }
  bool   empty() const { return used_ == 0; }
  void   clear() { head_ = tail_ = used_ = 0; }
  static constexpr size_t capacity() { return N; }

 private:
  void put(uint8_t b) {
    buf_[tail_] = b;
    tail_ = (tail_ + 1) % N;
    used_++;
  }
  uint8_t take() {
    uint8_t b = buf_[head_];
    head_ = (head_ + 1) % N;
    used_--;
    return b;
  }
  uint8_t buf_[N];
  size_t  head_ = 0, tail_ = 0, used_ = 0;
};
