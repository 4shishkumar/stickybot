// Held inputs belong to one selected connection. Clearing them must not depend
// on that connection still being available to receive release notifications.
#pragma once
#include <array>
#include <cstdint>

namespace hidreports {
constexpr unsigned KEYBOARD_SIZE = 8;
struct Releases {
  bool keyboard, consumer, mouse;
};

struct Held {
  uint8_t keyboard[KEYBOARD_SIZE]{};
  bool consumer = false;
  uint8_t mouse = 0;

  Releases release(bool canNotify) {
    Releases releases{canNotify, canNotify && consumer, canNotify && mouse != 0};
    for (auto& key : keyboard) key = 0;
    consumer = false;
    mouse = 0;
    return releases;
  }

  std::array<uint8_t, 5> mouseReport(int8_t x, int8_t y, int8_t wheel, int8_t pan) const {
    return {mouse, (uint8_t)x, (uint8_t)y, (uint8_t)wheel, (uint8_t)pan};
  }
};
}  // namespace hidreports
