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
namespace mic {

void begin();
void loop();                 // call every loop: starts/stops the mic hardware

void usbTalk(bool on);       // button A held
void preRoll();              // button B went down: start buffering
void agentTalk();            // the B press is a hold: stream to the agent
void agentEnd();             // B released (or cancelled): finish and stop

void suspend(bool on);       // pause the mic hardware (keeps the talk going) so the speaker can use the chip
bool usbStreaming();         // the Mac has the USB input open
bool live();                 // the mic hardware is running
int  peak();                 // last chunk's peak level, 0..32767

}  // namespace mic
