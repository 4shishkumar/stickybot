#pragma once
#include <Arduino.h>

// Full-screen event screens sent by the Mac agent (GitHub, Claude Code...):
//   celebrate - confetti + fanfare, ~6 s
//   nice      - teal toast, ~4 s
//   headsup   - orange, stays until a button press (or 60 s)
// Queued (up to 5) and shown one at a time, never over an approval prompt,
// pairing, dictation (A held) or talking (B held). See PROTOCOL.md "Events".
namespace events {

enum class Level : uint8_t { CELEBRATE = 0, NICE = 1, HEADSUP = 2 };

struct Event {
  String id, title, sub, icon, sound;
  Level  level = Level::NICE;
};

bool  parseLevel(const String& s, Level& out);
void  push(const Event& e);        // drops the oldest if the queue is full
bool  pending();                   // something waiting to be shown
bool  active();                    // one is on screen
const Event& current();
void  start();                     // shows the next one: returns its id via current()
uint32_t elapsed();                // ms since the current one started
bool  expired();                   // its time is up (headsup: 60 s)
void  finish();                    // done (timeout or dismissed)
const char* defaultSound(Level l);

}  // namespace events
