// The Wispr mic for an A hold: which path it takes (USB mic, Bluetooth to the
// Mac agent, or none), and for the Bluetooth path the key-down gate: the agent
// first makes the "stick-mic-bluetooth" device appear, so the key waits for its
// "ready" (or a timeout). Pure C++ (native-testable).
#pragma once
#include <stdint.h>

namespace wispr {

// The audio pipeline behind the key (mic chunk + frame + Bluetooth + the agent's
// jitter buffer) is ~150-200 ms. Wispr stops recording on key-up, so the key-up
// waits this long while streaming, or the last words would be cut off.
constexpr uint32_t KEYUP_TAIL_MS = 250;

// Bluetooth path with an agent that makes the mic device: the longest the key
// waits for "ready" before it goes down anyway. The agent needs ~100-170 ms to
// create the device plus its switch time (250 ms by default) and the link.
constexpr uint32_t READY_TIMEOUT_MS = 800;

enum class Path : uint8_t {
  NONE,       // no stick mic: the key at once, Wispr uses whatever mic it has
  USB,        // plugged into a computer: the USB mic ("stick-mic-usb"), the key at once — as before 1.7
  BT_DIRECT,  // 1.7.0 agent: the key at once + stream (Wispr must be set to BlackHole)
  BT_PREPARE  // 1.7.1+ agent: "prepare" + stream, the key once the agent says "ready"
};

struct Inputs {
  bool setting;        // the stick's own "Wireless mic" switch (menu / @wmic)
  bool agentWants;     // the agent's hello says it can play it (wireless_mic on + BlackHole found)
  bool agentPrepares;  // ... and answers "prepare" with "ready" ("wready")
  bool agentLinked;    // heard from the agent recently
  bool usbMounted;     // a host has enumerated the stick's USB device (see usbMounted())
  bool usbListening;   // a host has the USB microphone open right now
  bool micBusy;        // the mic is already recording for a B talk
};

Path choosePath(const Inputs& in);
inline bool streams(Path p) { return p == Path::BT_DIRECT || p == Path::BT_PREPARE; }

// "Plugged into a computer": TinyUSB says the host configured the device, and VBUS
// is there (TinyUSB can't see an unplug on the S3 without VBUS sensing, so the
// mounted flag may stay up on battery). A charger-only cable never mounts.
bool usbMounted(bool tudMounted, bool vbus);

// How long after A is released the key goes up (0 = right away, as on USB).
uint32_t keyUpDelayMs(bool streaming);

// The key-down gate of the BT_PREPARE path, one A press at a time.
class Gate {
 public:
  enum class Step : uint8_t { NONE, PRESS, CANCEL };

  void begin(uint32_t now, uint16_t id);  // A down: "prepare" sent with this id
  bool waiting() const { return waiting_; }
  uint16_t id() const { return id_; }
  bool ready(uint16_t id);                // the agent's "ready": true if it's for this press
  Step poll(uint32_t now);                // PRESS once ready, or after READY_TIMEOUT_MS
  Step release();                         // A up: CANCEL if the key never went down
  bool timedOut() const { return timedOut_; }
  uint32_t waitedMs() const { return waited_; }

 private:
  bool waiting_ = false, ready_ = false, timedOut_ = false;
  uint16_t id_ = 0;
  uint32_t since_ = 0, waited_ = 0;
};

}  // namespace wispr
