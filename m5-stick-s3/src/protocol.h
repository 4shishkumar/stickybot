// The command set from ../../PROTOCOL.md, shared by USB serial and the
// Wi-Fi settings page so both behave identically.
#pragma once
#include <Arduino.h>
#include <functional>
#include "logic/launcher.h"

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
void serialLine(String line, Print& reply = Serial);

// The legacy machine frame, shared with native output-boundary tests.
inline void machineReply(Print& reply, const String& json) {
  reply.print('@');
  reply.println(json);
}
inline void humanReply(Print& reply, const String& json) { reply.println(json); }

// A machine line ("@state", "@combo text=Option+Cmd") run without printing:
// used by the Mac agent over Bluetooth. If the result asks for a restart, call
// finish() once the reply has been sent.
// Every caller states its transport; Bluetooth cannot erase shared host records.
enum class Source { Serial, Bluetooth, BackgroundBluetooth };
// Only the selected Bluetooth connection supplies the main-loop navigation callback.
using OpenTile = launcher::OpenResult (*)(launcher::Tile);
Result runLine(const String& line, Source source, OpenTile openTile = nullptr);
void   finish(const Result& r);

}  // namespace protocol
