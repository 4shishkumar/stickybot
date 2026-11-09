// Remote modes (firmware 2.5.0): the stick as an air mouse, a presentation clicker or a
// media remote, over the same Bluetooth link (keyboard + consumer keys + mouse, hid.h).
//
//   Air mouse   turn the stick to move the pointer (gyro, logic/air_motion.*)
//               A = left button (hold A + move = drag), B click = right click,
//               hold B + tilt = scroll, A+B tap = pause / resume, hold A+B = back to Home
//               Set down still: the pointer stops (the gyro re-learns its bias); idle for
//               IDLE_OFF_MIN minutes: back to Home.
//   Presenter   A = next slide, B = previous, hold A = laser pointer (the mouse moves while
//               held), double-press B = black screen ('B'), hold B = restart the timer,
//               A+B tap = pause the timer, hold A+B = back to Home. The screen: a big talk
//               timer (starts on the first press; turns orange near the target, red past
//               it) and the battery.
//   Media       twist like a knob = volume, flick right / left = next / previous track,
//               A = play / pause, B = mute, hold A+B = back to Home.
//
// Opened from the stick's menu, the dashboard's Stick page or `@mode name=mouse|presenter|
// media|home`; settings with `@air` (both in PROTOCOL.md). While a mode runs it owns the
// buttons, the motion sensor and the screen; pairing requests still show.
#pragma once
#include <Arduino.h>
#include <functional>

namespace air {

enum class Mode : uint8_t { OFF, MOUSE, PRESENTER, MEDIA };

struct Settings {
  uint8_t  sens = 5;        // pointer sensitivity 1-10
  bool     invX = false, invY = false;
  uint8_t  scroll = 5;      // tilt-scroll speed 1-10
  bool     invScroll = false;
  uint16_t talkMin = 0;     // presenter target, minutes (0 = none)
  bool     pageKeys = false;  // presenter: Page Down / Page Up instead of the arrow keys
};

void        begin();                 // loads the settings
bool        active();                // a mode is running (or about to start)
Mode        mode();
const char* modeName(Mode m);        // "home", "mouse", "presenter", "media"
bool        parseMode(const String& s, Mode& out);
void        request(Mode m);         // start (or OFF: stop) at the next loop (safe from any task)
const Settings& settings();
void        save(const Settings& s);

// The main loop, while active(): reads the buttons and motion, sends the reports and
// draws the screen. Returns false once the mode has ended (back to Home).
bool loop();

// Protocol (PROTOCOL.md): `"air":{...}` for @state, and the @mode / @air actions.
String stateJson();
// action "mode" / "air"; arg(name) reads a parameter. Returns false for other actions.
bool command(const String& action, const std::function<String(const char*)>& arg, int& status, String& json);

}  // namespace air
