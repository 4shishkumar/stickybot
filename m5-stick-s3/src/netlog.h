// Logs over the network, like ESPHome: everything printed with `Log` goes to
// USB serial AND to TCP clients on port 23 while Wi-Fi is up. Lines typed by a
// TCP client run as serial commands (show, imulog, chglog, ...).
//
//   pio device monitor --port socket://stickybot.local:23
//   nc stickybot.local 23
#pragma once
#include <Arduino.h>

namespace netlog {

constexpr uint16_t PORT = 23;

void loop();     // accept clients, read their commands; call every loop
int  clients();  // connected log viewers

}  // namespace netlog

// Print to serial + network. Use instead of Serial.print*/printf.
class TeeLog : public Print {
 public:
  size_t write(uint8_t c) override;
  size_t write(const uint8_t* buf, size_t len) override;
};
extern TeeLog Log;
