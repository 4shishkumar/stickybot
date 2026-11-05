// The wireless Wispr mic: whether an A hold (the Wispr shortcut) also streams
// the mic to the Mac agent over Bluetooth, and how long the key-up waits for
// the audio still on its way. Pure C++ (native-testable).
#pragma once
#include <stdint.h>

namespace wispr {

// The audio pipeline behind the key (mic chunk + frame + Bluetooth + the agent's
// jitter buffer) is ~150-200 ms. Wispr stops recording on key-up, so the key-up
// waits this long while streaming, or the last words would be cut off.
constexpr uint32_t KEYUP_TAIL_MS = 250;

struct Inputs {
  bool setting;       // the stick's own "Wireless mic" switch (menu / @wmic)
  bool agentWants;    // the agent's hello says it can play it (wireless_mic on + BlackHole found)
  bool agentLinked;   // heard from the agent recently
  bool usbListening;  // a host has the stick's USB microphone open: that's the mic already
  bool micBusy;       // the mic is already recording for a B talk
};

bool shouldStream(const Inputs& in);

// How long after A is released the key goes up (0 = right away, as on USB).
uint32_t keyUpDelayMs(bool streaming);

}  // namespace wispr
