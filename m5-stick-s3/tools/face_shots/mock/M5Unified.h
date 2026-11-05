// Just enough of M5Unified for the screens on a Mac (face_shots): the display is a plain
// LovyanGFX sprite, so ui.cpp draws exactly as on the stick and the pictures can be saved.
#pragma once
#include <Arduino.h>
#include <lgfx/v1/LGFXBase.hpp>
#include <lgfx/v1/LGFX_Sprite.hpp>

using namespace lgfx::v1::textdatum;
using namespace lgfx::v1::colors;
using lgfx::v1::textdatum_t;
using M5Canvas = lgfx::LGFX_Sprite;

struct FakeDisplay : public lgfx::LGFX_Sprite {
  FakeDisplay() { setColorDepth(16); createSprite(135, 240); }
  void setBrightness(uint8_t) {}
  void sleep() {}
  void wakeup() {}
};

struct FakeM5 {
  FakeDisplay Display;
};
extern FakeM5 M5;
