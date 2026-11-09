// On-device settings menu.
//   BtnA (front) = next item
//   BtnB (side)  = change / select; hold on "Key" to scroll fast
// Changes to combo, Mac slot and standby are drafts until "Save & exit".
#pragma once
#include "combo.h"
#include "ui.h"

namespace menu {

enum class Action { NONE, REDRAW, TEST, WEB, FLIP, BT_TOGGLE, FORGET, BRIGHTNESS, VOLUME, POWER_OFF, SAVE, CANCEL, REMOTE };

struct Draft {
  Combo   combo;
  uint8_t slot;
  uint8_t standbyMin;
  uint8_t brightnessPct;
  uint16_t enterDelayMs;  // 0 = off
  bool    lift, faceDown, shake;
  bool    autoRotate;
  bool    landscape;   // landscape screens when held sideways (with auto-rotate)
  bool    clock24;     // 24-hour time
  bool    devMode;
  bool    sound;
  uint8_t volume;  // 0-255
  bool    wmic;    // wireless Wispr mic
  uint8_t face;    // the Home face (index in faces::at)
};

void         open(const Draft& current, bool btOff);
Action       next();      // BtnA pressed
Action       select();    // BtnB pressed
Action       repeat();    // BtnB held (auto-repeat)
bool         timedOut();
const Draft& draft();
ui::MenuView view();
uint8_t      remoteMode();  // after Action::REMOTE: 1 air mouse, 2 presenter, 3 media (air.h)

}  // namespace menu
