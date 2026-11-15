// Fixed, payload-free diagnostics go to USB and connected TCP viewers on port 23.
// Command replies go only to the Print destination that submitted the command.
#pragma once
#include <Arduino.h>
#include "logic/diagnostics.h"

namespace netlog {

constexpr uint16_t PORT = 23;

void loop();     // accept clients, read their commands; call every loop
int  clients();  // connected log viewers

}  // namespace netlog

// Only typed events are diagnostics. Inherited raw Print output fails closed.
class TeeLog : public Print {
 public:
  void event(diagnostics::Event event);
  void count(diagnostics::Counter counter, uint32_t value);
  size_t write(uint8_t c) override;
  size_t write(const uint8_t* buf, size_t len) override;
 private:
  bool omittedLine_ = false;
};
extern TeeLog Log;
