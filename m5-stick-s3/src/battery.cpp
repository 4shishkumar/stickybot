#include "battery.h"
#include "netlog.h"
#include "board.h"
#include <M5Unified.h>
#include <algorithm>
#include "logic/battery_math.h"

namespace battery {

constexpr uint32_t SAMPLE_MS       = 2000;
constexpr int      WINDOW          = 5;      // median over the last N samples
constexpr float    EMA_ALPHA       = 0.15f;  // then a slow moving average
// Charge status comes from the LGS4056 charger's CHRG pin (via PM1 G0, low =
// charging). It flickers while the stick also draws power, so: sample it
// often, and count as charging if it was low at any point in the last 10 s.
constexpr uint32_t CHG_SAMPLE_MS    = 200;
constexpr uint32_t CHG_HOLD_MS      = 10000;
constexpr uint32_t FULL_AFTER_MS    = 60000;  // pin high this long at >= FULL_MV = full
constexpr int      FULL_MV          = 4150;

// Voltage -> charge curve, median, trend and ETA maths: logic/battery_math.cpp (unit-tested).

static int      samples[WINDOW];
static int      count = 0, next = 0;
static float    smoothMv = 0;
static int      level_ = -1;
static bool     charging_ = false, full_ = false;
static uint32_t lastSample = 0, lastChgSample = 0, lastLowAt = 0, highSince = 0;

// Safety net: if USB is present but the charger isn't charging and the cell
// isn't full, toggle charge-enable once in a while (like re-plugging).
constexpr uint32_t KICK_AFTER_MS = 2 * 60 * 1000;
constexpr uint32_t KICK_EVERY_MS = 30 * 60 * 1000;
static bool     usb_ = false;
static uint32_t notChargingSince = 0, lastKick = 0;
static int      kicks = 0;

// Level history for the time estimate: one point a minute, last 30 minutes.
constexpr uint32_t HIST_EVERY_MS = 60000;
constexpr int      HIST = 30;
constexpr int      MIN_POINTS = 10;
static float    histMin[HIST], histPct[HIST];  // minutes since start, level
static int      histN = 0, histHead = 0;
static uint32_t lastHist = 0, histStart = 0;

// Last-hour chart: RTC memory survives standby (deep sleep). Samples carry the
// wall-clock time when known, so a long standby drops the stale ones.
constexpr uint32_t RECENT_EVERY_MS = 60000;
RTC_DATA_ATTR static batt::RecentRing<RECENT> recentRing;
static uint32_t lastRecent = 0;

constexpr uint32_t CHART_EVERY_MS = 5 * 60 * 1000;
static uint16_t chartMv[HISTORY];
static bool     chartChg[HISTORY];
static int      chartN = 0, chartHead = 0;
static uint32_t lastChart = 0;

static int median() { return batt::median(samples, count); }

static void setLed(bool on) {
  static int8_t ledState = -1;
  if (ledState == (int8_t)on) return;
  ledState = on;
  board::statusLed(on);
}

void update() {
  uint32_t now = millis();

  // Charger pin, sampled fast
  if (!lastChgSample || now - lastChgSample >= CHG_SAMPLE_MS) {
    lastChgSample = now;
    if (M5.Power.isCharging() == m5::Power_Class::is_charging_t::is_charging) {
      lastLowAt = now;
      highSince = 0;
    } else if (!highSince) {
      highSince = now;
    }
  }

  if (lastSample && now - lastSample < SAMPLE_MS) return;
  lastSample = now;

  usb_ = M5.Power.getVBUSVoltage() > 4000;
  bool c = usb_ && lastLowAt && now - lastLowAt < CHG_HOLD_MS;
  bool chargeFlipped = c != charging_;
  charging_ = c;
  full_ = usb_ && !charging_ && highSince && now - highSince > FULL_AFTER_MS && smoothMv >= FULL_MV;
  setLed(charging_);  // on while charging; off when full or unplugged

  if (board::HAS_POWER_CHIP && usb_ && !charging_ && !full_ && level_ >= 0 && level_ < 95) {
    if (!notChargingSince) notChargingSince = now;
    if (now - notChargingSince > KICK_AFTER_MS && (!lastKick || now - lastKick > KICK_EVERY_MS)) {
      lastKick = now;
      notChargingSince = 0;
      kicks++;
      board::chargerRestart();
      Log.println("Charger: on USB but not charging - toggled charge-enable");
    }
  } else {
    notChargingSince = 0;
  }

  int mv = M5.Power.getBatteryVoltage();
  if (mv <= 0) return;
  samples[next] = mv;
  next = (next + 1) % WINDOW;
  if (count < WINDOW) count++;

  float m = median();
  smoothMv = smoothMv == 0 ? m : smoothMv + EMA_ALPHA * (m - smoothMv);
  int pct = batt::percentFromMv(smoothMv);

  // Only move in the direction that makes sense, so the number doesn't
  // bounce: up while charging, down on battery. Re-sync when the charger
  // is plugged in or unplugged.
  if (chargeFlipped) histN = 0;  // trend restarts when plugged in / unplugged
  if (!lastHist || now - lastHist >= HIST_EVERY_MS) {
    lastHist = now;
    if (!histN) { histStart = now; histHead = 0; }
    histMin[histHead] = (now - histStart) / 60000.0f;
    histPct[histHead] = batt::percentFromMvF(smoothMv);
    histHead = (histHead + 1) % HIST;
    if (histN < HIST) histN++;
  }

  if ((!lastChart && count >= WINDOW) || (lastChart && now - lastChart >= CHART_EVERY_MS)) {
    lastChart = now;
    chartMv[chartHead] = (uint16_t)smoothMv;
    chartChg[chartHead] = charging_;
    chartHead = (chartHead + 1) % HISTORY;
    if (chartN < HISTORY) chartN++;
  }

  if (level_ >= 0 && (!lastRecent || now - lastRecent >= RECENT_EVERY_MS)) {
    lastRecent = now;
    uint32_t epoch = (uint32_t)time(nullptr);
    recentRing.add((uint8_t)level_, charging_, epoch, epoch > 1700000000);
  }

  if (level_ < 0 || chargeFlipped) level_ = pct;
  else if (charging_ && pct > level_) level_ = pct;
  else if (!charging_ && pct < level_) level_ = pct;
}

int etaMinutes() {
  float s = 0;
  bool ok = batt::slope(histMin, histPct, histN, MIN_POINTS, s);  // % per minute
  return batt::etaMinutes(level_, charging_, ok, s);
}

String etaLabel() {
  int m = etaMinutes();
  if (m == -1) return "Estimating...";
  if (m == -2) return "Charging slowly";
  if (charging_ && m == 0) return "Full";
  if (m >= 24 * 60) return charging_ ? "Full in 24h+" : "1 day+ left";
  String t = m >= 60 ? String(m / 60) + "h " + String(m % 60) + "m" : String(m) + "m";
  return charging_ ? "Full in ~" + t : "~" + t + " left";
}

bool usbPower() { return usb_; }
int  chargerRestarts() { return kicks; }

bool full() { return full_; }

int history(uint16_t mv[HISTORY], bool charging[HISTORY]) {
  int start = (chartHead - chartN + HISTORY) % HISTORY;
  for (int i = 0; i < chartN; i++) {
    mv[i] = chartMv[(start + i) % HISTORY];
    charging[i] = chartChg[(start + i) % HISTORY];
  }
  return chartN;
}

int recent(uint8_t pct[RECENT], bool charging[RECENT]) { return recentRing.read(pct, charging); }

String line() {
  String v = String(smoothMv / 1000.0f, 2) + " V";
  int m = etaMinutes();
  if (m > 0 && m < 24 * 60) {
    String t = m >= 60 ? String(m / 60) + "h " + String(m % 60) + "m" : String(m) + "m";
    return v + (charging_ ? " · full ~" : " · ~") + t + (charging_ ? "" : " left");
  }
  if (charging_) return v + " · charging";
  if (full_) return v + " · full";
  return usb_ ? v + " · not charging" : v;
}

int  level()    { return level_; }
int  millivolts() { return (int)smoothMv; }
bool charging() { return charging_; }

}  // namespace battery
