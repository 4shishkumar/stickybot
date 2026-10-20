// On-device settings menu.
//   BtnA (front) = next item
//   BtnB (side)  = change / select; hold on "Key" to scroll fast
// Changes to combo, Mac slot and standby are drafts until "Save & exit".
#pragma once
#include "combo.h"
#include "ui.h"

namespace menu {

enum class Action { NONE, REDRAW, TEST, WEB, FLIP, BT_TOGGLE, FORGET, BRIGHTNESS, VOLUME, POWER_OFF, SAVE, CANCEL };

struct Draft {
  Combo   combo;
  uint8_t slot;
  uint8_t standbyMin;
  uint8_t brightnessPct;
  uint16_t enterDelayMs;  // 0 = off
  bool    lift, faceDown, shake;
  bool    autoRotate;
  bool    devMode;
  bool    sound;
  uint8_t volume;  // 0-255
};

void         open(const Draft& current, bool btOff);
Action       next();      // BtnA pressed
Action       select();    // BtnB pressed
Action       repeat();    // BtnB held (auto-repeat)
bool         timedOut();
const Draft& draft();
ui::MenuView view();

}  // namespace menu
