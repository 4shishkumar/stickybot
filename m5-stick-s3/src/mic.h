#pragma once
#include <Arduino.h>

// Built-in microphone, push-to-talk, sent to one of two places:
//   USB   - button A: the stick is also a USB microphone on the Mac, for
//           dictation apps like Wispr Flow (silence unless A is held).
//   AGENT - button B: compressed (IMA-ADPCM) frames over Bluetooth to the Mac
//           agent (tools/mac-agent), so this part works without a cable.
//
// For the agent, recording starts as soon as B goes down (preRoll) and the
// buffered audio is sent once the press turns into a hold, so the first words
// aren't lost while the hold is being detected.
//
// Wireless Wispr mic: while A holds the Wispr shortcut away from USB, the same
// agent frames go to the Mac, marked "wispr" instead of "talk"; the agent plays
// them into the BlackHole virtual mic that Wispr listens to.
namespace mic {

void begin();
void loop();                 // call every loop: starts/stops the mic hardware

void usbTalk(bool on);       // button A held
void preRoll();              // button B went down: start buffering
void agentTalk();            // the B press is a hold: stream to the agent
void agentEnd();             // B released (or cancelled): finish and stop
bool wisprStart();           // A went down: stream to the agent for Wispr right away (false: mic busy)
void wisprEnd();             // A's key went up: finish and stop
bool wisprStreaming();       // a Wispr stream is running (or finishing)

void suspend(bool on);       // pause the mic hardware (keeps the talk going) so the speaker can use the chip
bool usbStreaming();         // the Mac has the USB input open
bool live();                 // the mic hardware is running
int  peak();                 // last chunk's peak level, 0..32767

}  // namespace mic
