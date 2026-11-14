// Board identity — see ../../PROTOCOL.md
#pragma once

#include "board.h"                    // STICK_BOARD: which stick model this build is for
#define FW_VERSION  "2.7.0"
#define DEFAULT_NAME "StickyBot"         // Bluetooth name until changed (same for all Mac slots)
#define HOSTNAME    "stickybot"          // http://stickybot.local
#define AP_SSID     "stickybot-setup"    // hotspot when no home Wi-Fi is saved
#define DEFAULT_PASSWORD "macro-setup"  // hotspot + OTA password until changed
