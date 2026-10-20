// Wi-Fi settings page + OTA updates. Off by default; on demand only.
//
// If home Wi-Fi is saved: joins it, page at http://xiao-key.local
// Otherwise (or if joining fails): hotspot "xiao-key-setup", password =
// device password, page at http://192.168.4.1
#pragma once
#include <Arduino.h>

namespace web {

void        start();
void        stop();
void        loop();
bool        active();
const char* mode();   // off | joining | home | hotspot
String      ssid();   // network the board is on / offering
String      url();

}  // namespace web
