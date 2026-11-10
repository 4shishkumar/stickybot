// Drawing the Launcher (firmware 2.5.1): the app grid, Quick settings and the Faces picker,
// on ui::canvas, portrait or landscape (as the reading screens: ui::turnToHeldSide()). Only
// drawing: the session (buttons, settings) is launcher_session.cpp. The host renderer
// (tools/face_shots, `make launcher`) draws the same screens on the Mac.
#pragma once
#include <Arduino.h>
#include "logic/launcher.h"

namespace launcherui {

struct GridView {
  launcher::Layout layout;
  int    sel = 0;
  String clock;  // "9:41", "" until the agent has sent the time
};
void drawGrid(const GridView& v);

struct QuickView {
  int     sel = 0;  // launcher::Quick
  uint8_t brightness = 75, volume = 192;
  bool    sound = true, wmic = false, clock24 = false, rotate = true, gestures = true;
};
void drawQuick(const QuickView& v);

struct FacesView {
  int sel = 0;      // in faces::at()
  int current = 0;  // the face in use
};
void drawFaces(const FacesView& v);

}  // namespace launcherui
