#include "battery_math.h"
#include <algorithm>

namespace batt {

static const struct { int mv, pct; } CURVE[] = {
  {4150, 100}, {4050, 90}, {3970, 80}, {3900, 70}, {3840, 60}, {3800, 50},
  {3770, 40},  {3740, 30}, {3700, 20}, {3650, 10}, {3500, 5},  {3300, 0},
};
constexpr size_t CURVE_N = sizeof(CURVE) / sizeof(CURVE[0]);

float percentFromMvF(float mv) {
  if (mv >= CURVE[0].mv) return 100;
  for (size_t i = 1; i < CURVE_N; i++)
    if (mv >= CURVE[i].mv) {
      const auto &hi = CURVE[i - 1], &lo = CURVE[i];
      return lo.pct + (mv - lo.mv) * (hi.pct - lo.pct) / (float)(hi.mv - lo.mv);
    }
  return 0;
}

int percentFromMv(float mv) {
  if (mv >= CURVE[0].mv) return 100;
  for (size_t i = 1; i < CURVE_N; i++)
    if (mv >= CURVE[i].mv) {
      const auto &hi = CURVE[i - 1], &lo = CURVE[i];
      return lo.pct + (int)((mv - lo.mv) * (hi.pct - lo.pct) / (hi.mv - lo.mv));
    }
  return 0;
}

int median(const int* v, int n) {
  int sorted[32];
  if (n > 32) n = 32;
  std::copy(v, v + n, sorted);
  std::sort(sorted, sorted + n);
  return sorted[n / 2];
}

bool slope(const float* x, const float* y, int n, int minPoints, float& out) {
  if (n < minPoints || n < 2) return false;
  float mx = 0, my = 0;
  for (int i = 0; i < n; i++) { mx += x[i]; my += y[i]; }
  mx /= n; my /= n;
  float num = 0, den = 0;
  for (int i = 0; i < n; i++) {
    num += (x[i] - mx) * (y[i] - my);
    den += (x[i] - mx) * (x[i] - mx);
  }
  if (den <= 0) return false;
  out = num / den;
  return true;
}

int etaMinutes(int level, bool charging, bool haveSlope, float s) {
  if (level < 0 || !haveSlope) return -1;
  if (charging) {
    if (level >= 99) return 0;
    if (s < 0.02f) return -2;  // under ~1.2 %/h: too slow to give a time
    return (int)((100 - level) / s);
  }
  if (s > -0.005f) return 24 * 60;  // barely dropping: "a day or more"
  return (int)(level / -s);
}

}  // namespace batt
