// The command set from ../../PROTOCOL.md, shared by USB serial and the
// Wi-Fi settings page so both behave identically.
#pragma once
#include <Arduino.h>
#include <functional>

namespace protocol {

struct Result {
  int    status = 200;      // HTTP status
  String json;              // response body
  bool   restart = false;   // caller should restart after replying
};

using Args = std::function<String(const char* name)>;

// action: state, keys, combo, test, slot, disconnect, reconnect, forget,
//         wifi, password, web-on, web-off
Result run(const String& action, const Args& arg);

// One line from USB serial: "@action k=v&..." (machine) or human commands.
void serialLine(String line);

}  // namespace protocol
