#include "battery.h"
#include "netlog.h"
#include <M5Unified.h>
#include <algorithm>

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

// Typical single-cell Li-ion resting voltage -> charge.
static const struct { int mv, pct; } CURVE[] = {
  {4150, 100}, {4050, 90}, {3970, 80}, {3900, 70}, {3840, 60}, {3800, 50},
  {3770, 40},  {3740, 30}, {3700, 20}, {3650, 10}, {3500, 5},  {3300, 0},
};

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
static struct { float min, pct; } hist[HIST];
static int      histN = 0, histHead = 0;
static uint32_t lastHist = 0, histStart = 0;

// Last-hour chart: RTC memory survives standby (deep sleep). Samples carry the
// wall-clock time when known, so a long standby drops the stale ones.
constexpr uint32_t RECENT_EVERY_MS = 60000;
constexpr uint32_t RECENT_MAGIC = 0x5EC0BA77;
RTC_DATA_ATTR static uint32_t recentMagic = 0;
RTC_DATA_ATTR static uint8_t  recentPct[RECENT];
RTC_DATA_ATTR static uint8_t  recentChg[RECENT];
RTC_DATA_ATTR static uint8_t  recentN = 0, recentHead = 0;
RTC_DATA_ATTR static uint32_t recentLastEpoch = 0;
static uint32_t lastRecent = 0;

constexpr uint32_t CHART_EVERY_MS = 5 * 60 * 1000;
static uint16_t chartMv[HISTORY];
static bool     chartChg[HISTORY];
static int      chartN = 0, chartHead = 0;
static uint32_t lastChart = 0;

static float toPercentF(float mv) {
  if (mv >= CURVE[0].mv) return 100;
  for (size_t i = 1; i < sizeof(CURVE) / sizeof(CURVE[0]); i++) {
    if (mv >= CURVE[i].mv) {
      const auto &hi = CURVE[i - 1], &lo = CURVE[i];
      return lo.pct + (mv - lo.mv) * (hi.pct - lo.pct) / (float)(hi.mv - lo.mv);
    }
  }
  return 0;
}

static int toPercent(float mv) {
  if (mv >= CURVE[0].mv) return 100;
  for (size_t i = 1; i < sizeof(CURVE) / sizeof(CURVE[0]); i++) {
    if (mv >= CURVE[i].mv) {
      const auto &hi = CURVE[i - 1], &lo = CURVE[i];
      return lo.pct + (mv - lo.mv) * (hi.pct - lo.pct) / (hi.mv - lo.mv);
    }
  }
  return 0;
}

static int median() {
  int sorted[WINDOW];
  std::copy(samples, samples + count, sorted);
  std::sort(sorted, sorted + count);
  return sorted[count / 2];
}

static void setLed(bool on) {
  static int8_t ledState = -1;
  if (ledState == (int8_t)on) return;
  ledState = on;
  M5.Power.M5pm1.setLedEnLevel(on);  // green status LED (PY_STATUS_LED)
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

  if (usb_ && !charging_ && !full_ && level_ >= 0 && level_ < 95) {
    if (!notChargingSince) notChargingSince = now;
    if (now - notChargingSince > KICK_AFTER_MS && (!lastKick || now - lastKick > KICK_EVERY_MS)) {
      lastKick = now;
      notChargingSince = 0;
      kicks++;
      M5.Power.M5pm1.setBatteryCharge(false);
      delay(300);
      M5.Power.M5pm1.setBatteryCharge(true);
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
  int pct = toPercent(smoothMv);

  // Only move in the direction that makes sense, so the number doesn't
  // bounce: up while charging, down on battery. Re-sync when the charger
  // is plugged in or unplugged.
  if (chargeFlipped) histN = 0;  // trend restarts when plugged in / unplugged
  if (!lastHist || now - lastHist >= HIST_EVERY_MS) {
    lastHist = now;
    if (!histN) { histStart = now; histHead = 0; }
    hist[histHead] = {(now - histStart) / 60000.0f, toPercentF(smoothMv)};
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

  if (recentMagic != RECENT_MAGIC) {
    recentMagic = RECENT_MAGIC;
    recentN = recentHead = 0;
    recentLastEpoch = 0;
  }
  if (level_ >= 0 && (!lastRecent || now - lastRecent >= RECENT_EVERY_MS)) {
    lastRecent = now;
    uint32_t epoch = (uint32_t)time(nullptr);
    bool timeOk = epoch > 1700000000;
    if (timeOk && recentLastEpoch && epoch - recentLastEpoch > 3600) recentN = recentHead = 0;  // stale after a long sleep
    // after a standby the first sample comes right away; don't double up within a minute
    if (!(timeOk && recentLastEpoch && epoch - recentLastEpoch < 50)) {
      recentPct[recentHead] = (uint8_t)level_;
      recentChg[recentHead] = charging_;
      recentHead = (recentHead + 1) % RECENT;
      if (recentN < RECENT) recentN++;
      if (timeOk) recentLastEpoch = epoch;
    }
  }

  if (level_ < 0 || chargeFlipped) level_ = pct;
  else if (charging_ && pct > level_) level_ = pct;
  else if (!charging_ && pct < level_) level_ = pct;
}

// Least-squares slope of level over time, in % per minute.
static bool slope(float& perMin) {
  if (histN < MIN_POINTS) return false;
  float mx = 0, my = 0;
  for (int i = 0; i < histN; i++) { mx += hist[i].min; my += hist[i].pct; }
  mx /= histN; my /= histN;
  float num = 0, den = 0;
  for (int i = 0; i < histN; i++) {
    num += (hist[i].min - mx) * (hist[i].pct - my);
    den += (hist[i].min - mx) * (hist[i].min - mx);
  }
  if (den <= 0) return false;
  perMin = num / den;
  return true;
}

int etaMinutes() {
  float s;
  if (level_ < 0 || !slope(s)) return -1;
  if (charging_) {
    if (level_ >= 99) return 0;
    if (s < 0.02f) return -2;  // under ~1.2 %/h: too slow to give a time
    return (int)((100 - level_) / s);
  }
  if (s > -0.005f) return 24 * 60;  // barely dropping: say "a day or more"
  return (int)(level_ / -s);
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

int recent(uint8_t pct[RECENT], bool charging[RECENT]) {
  if (recentMagic != RECENT_MAGIC) return 0;
  int start = (recentHead - recentN + RECENT) % RECENT;
  for (int i = 0; i < recentN; i++) {
    pct[i] = recentPct[(start + i) % RECENT];
    charging[i] = recentChg[(start + i) % RECENT];
  }
  return recentN;
}

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
