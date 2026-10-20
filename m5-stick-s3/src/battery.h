// Battery level, smoothed. The StickS3 has no fuel gauge — only a voltage
// reading, which jumps with radio/charger load. This filters it into a
// steady percentage.
#pragma once
#include <Arduino.h>

namespace battery {

void update();     // call often; samples every couple of seconds
int  level();      // 0-100, -1 until the first reading
int  millivolts(); // smoothed battery voltage
bool charging();   // debounced

// Time estimate from the level trend over the last ~30 min (no current
// sensor, so it's a rough guide). Minutes to empty (on battery) or to full
// (charging); -1 = still estimating, -2 = charging too slowly to tell.
int    etaMinutes();
bool   usbPower();    // USB plugged in
bool   full();        // on USB, charger finished
String line();        // "4.02 V · charging" / "3.98 V · ~3h 20m left" for the status screen
int    chargerRestarts();

// Voltage history for the battery chart: one point every 5 min, last 6 h.
// Copies oldest -> newest into the arrays; returns the number of points.
constexpr int HISTORY = 72;
int    history(uint16_t mv[HISTORY], bool charging[HISTORY]);
String etaLabel();  // "~3h 20m left", "Full in ~1h 5m", "Estimating...", ...

// Last hour: one level sample a minute, kept in RTC memory so standby doesn't
// wipe it. Copies oldest -> newest; returns the number of points (0-60).
constexpr int RECENT = 60;
int    recent(uint8_t pct[RECENT], bool charging[RECENT]);

}  // namespace battery
