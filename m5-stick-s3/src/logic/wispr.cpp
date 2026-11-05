#include "wispr.h"

namespace wispr {

Path choosePath(const Inputs& in) {
  if (in.usbMounted || in.usbListening) return Path::USB;  // the cable wins: exactly as before 1.7
  if (!in.setting || !in.agentWants || !in.agentLinked || in.micBusy) return Path::NONE;
  return in.agentPrepares ? Path::BT_PREPARE : Path::BT_DIRECT;
}

bool usbMounted(bool tudMounted, bool vbus) { return tudMounted && vbus; }

uint32_t keyUpDelayMs(bool streaming) { return streaming ? KEYUP_TAIL_MS : 0; }

void Gate::begin(uint32_t now, uint16_t id) {
  waiting_ = true;
  ready_ = timedOut_ = false;
  id_ = id;
  since_ = now;
  waited_ = 0;
}

bool Gate::ready(uint16_t id) {
  if (!waiting_ || id != id_) return false;  // late, for an earlier press
  ready_ = true;
  return true;
}

Gate::Step Gate::poll(uint32_t now) {
  if (!waiting_) return Step::NONE;
  bool late = now - since_ >= READY_TIMEOUT_MS;
  if (!ready_ && !late) return Step::NONE;
  waiting_ = false;
  timedOut_ = !ready_;
  waited_ = now - since_;
  return Step::PRESS;
}

Gate::Step Gate::release() {
  if (!waiting_) return Step::NONE;  // the key is down (or there was no gate): the normal key-up
  waiting_ = false;
  return Step::CANCEL;
}

}  // namespace wispr
