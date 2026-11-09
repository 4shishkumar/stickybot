// The remote modes' screens (air mouse, presenter, media remote): drawn from a View, so
// face_shots can draw them on the Mac too. Portrait, and landscape when held sideways.
#pragma once
#include <Arduino.h>

namespace airui {

enum class Screen : uint8_t { MOUSE, PRESENTER, MEDIA };

struct View {
  Screen   screen = Screen::MOUSE;
  bool     connected = true;
  bool     mouseUnknown = false;  // connected a while and the Mac never subscribed to the mouse
  int      battery = -1;          // 0-100, -1 unknown
  bool     charging = false;
  // air mouse
  uint8_t  state = 0;             // MouseState below
  uint8_t  sens = 5;
  // presenter
  uint32_t elapsedMs = 0;
  uint16_t targetMin = 0;
  uint8_t  phase = 0;             // remote::Phase: 0 idle, 1 running, 2 warn, 3 over
  bool     timerRunning = false;
  bool     laser = false, black = false;
  int      slide = 0;             // net slides moved (next - previous) this talk
  // media
  uint8_t  lastAction = 0;        // MediaAction below
  float    knob = 0;              // the twist towards the next volume step, -1..1
  // a word that flashes for a moment after an action ("Next", "Paused")
  String   flash;
  float    flashT = 1;            // 0 just now .. 1 gone
};

enum MouseState : uint8_t { POINTING, PAUSED, RESTING, SCROLLING, DRAGGING };
enum MediaAction : uint8_t { NONE, PLAY_PAUSE, NEXT, PREVIOUS, VOL_UP, VOL_DOWN, MUTE };

void draw(const View& v);

}  // namespace airui
