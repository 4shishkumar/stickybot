// Battery maths (pure C++, native-testable): voltage -> charge, the median /
// trend helpers behind the time estimate, and the last-hour ring kept in RTC
// memory. battery.cpp owns the hardware side.
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace batt {

int   percentFromMv(float mv);   // typical single-cell Li-ion resting curve, 0..100
float percentFromMvF(float mv);  // same, without rounding (for trends)
int   median(const int* v, int n);  // n >= 1; doesn't modify v

// Least-squares slope of y over x; false if fewer than minPoints or x has no spread.
bool slope(const float* x, const float* y, int n, int minPoints, float& out);

// Minutes until full (charging) or empty, from the level and its slope (%/min):
//   -1 = not enough data, -2 = charging too slowly to say, 0 = full, 24*60 = a day or more.
int etaMinutes(int level, bool charging, bool haveSlope, float slopePerMin);

// Last hour: one sample a minute. Plain data (no constructor) so it can live in
// RTC memory across standby; `magic` tells a cold boot from a wake.
template <int N>
struct RecentRing {
  static constexpr uint32_t MAGIC = 0x5EC0BA77;
  uint32_t magic;
  uint8_t  pct[N], chg[N];
  uint8_t  n, head;
  uint32_t lastEpoch;  // wall-clock time of the last sample (0 = unknown)

  void ensure() {
    if (magic != MAGIC) { magic = MAGIC; n = head = 0; lastEpoch = 0; }
  }
  // epoch: current wall-clock seconds; timeOk: whether epoch is real.
  void add(uint8_t p, bool charging, uint32_t epoch, bool timeOk) {
    ensure();
    if (timeOk && lastEpoch && epoch - lastEpoch > 3600) n = head = 0;  // stale after a long sleep
    if (timeOk && lastEpoch && epoch - lastEpoch < 50) return;          // just woke: no double sample
    pct[head] = p;
    chg[head] = charging;
    head = (head + 1) % N;
    if (n < N) n++;
    if (timeOk) lastEpoch = epoch;
  }
  // Oldest first; returns the count.
  int read(uint8_t* p, bool* c) const {
    if (magic != MAGIC) return 0;
    int start = (head - n + N) % N;
    for (int i = 0; i < n; i++) { p[i] = pct[(start + i) % N]; c[i] = chg[(start + i) % N]; }
    return n;
  }
};

}  // namespace batt
