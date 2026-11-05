#include "wispr.h"

namespace wispr {

bool shouldStream(const Inputs& in) {
  return in.setting && in.agentWants && in.agentLinked && !in.usbListening && !in.micBusy;
}

uint32_t keyUpDelayMs(bool streaming) { return streaming ? KEYUP_TAIL_MS : 0; }

}  // namespace wispr
