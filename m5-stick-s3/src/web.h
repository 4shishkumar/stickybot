// Wi-Fi settings page + OTA updates. Off by default; on demand only.
//
// If home Wi-Fi is saved: joins it, page at http://stick-key.local
// Otherwise (or if joining fails): hotspot "stick-key-setup", password =
// device password, page at http://192.168.4.1
#pragma once
#include <Arduino.h>

namespace web {

void        start(bool shown = true);  // shown = user opened it (stick shows the info screen)
bool        shown();
void        hide();   // leave Wi-Fi on but close the info screen
void        stop();
void        loop();
bool        active();
const char* mode();   // off | joining | home | hotspot
String      ssid();   // network the board is on / offering
String      url();
String      password();
bool        busy();
void        setKeepAlive(bool on);  // developer mode: no idle auto-off      // firmware update in progress  // shown on the stick's screen in hotspot mode

}  // namespace web
