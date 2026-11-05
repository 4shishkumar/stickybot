#include "city.h"
#include <algorithm>

namespace city {

// A well-mixed 32-bit hash of (seed, index, salt) (splitmix-style).
static uint32_t mix(uint32_t seed, uint32_t index, uint32_t salt) {
  uint32_t z = seed * 0x9E3779B9u + index * 0x85EBCA6Bu + salt * 0xC2B2AE35u + 0x27D4EB2Fu;
  z ^= z >> 16;
  z *= 0x7FEB352Du;
  z ^= z >> 15;
  z *= 0x846CA68Bu;
  z ^= z >> 16;
  return z;
}

Building building(uint32_t seed, int index) {
  uint32_t r = mix(seed, (uint32_t)index, 1);
  Building b;
  b.w = 3 + r % 4;
  b.h = 3 + (r >> 4) % 10;
  b.roof = (r >> 9) % ROOF_COUNT;
  b.color = (r >> 12) % 6;
  b.windows = (uint16_t)(mix(seed, (uint32_t)index, 2) & 0xFFFF);
  b.tower = false;
  return b;
}

Building tower(uint32_t seed, int index) {
  uint32_t r = mix(seed, (uint32_t)index, 3);
  Building b;
  b.w = 4 + r % 2;
  b.h = 15 + (r >> 3) % 4;
  b.roof = ANTENNA;
  b.color = 0;
  b.windows = (uint16_t)(r >> 8);
  b.tower = true;
  return b;
}

int layout(uint32_t seed, int count, const int* towersAt, int nTowers, int width, int cell, Placed out[], int max) {
  if (max <= 0 || width <= 0 || cell <= 0 || count < 0) return 0;
  // The skyline in order, keeping the newest `max` in out[] (a ring while filling)
  int total = 0;
  auto push = [&](const Building& b) {
    out[total % max].b = b;
    total++;
  };
  int t = 0;
  for (int i = 0; i <= count; i++) {
    while (t < nTowers && (towersAt[t] <= i || i == count)) push(tower(seed, t++));  // tower t after towersAt[t] buildings
    if (i < count) push(building(seed, i));
  }
  const int n = std::min(total, max);
  if (total > max) std::rotate(out, out + total % max, out + max);  // oldest kept first
  if (!n) return 0;

  int sumW = 0;
  for (int i = 0; i < n; i++) sumW += out[i].b.w;
  const int natural = (sumW + n - 1) * cell;  // with a 1-cell gap between them
  if (natural <= width) {
    int x = (width - natural) / 2;
    for (int i = 0; i < n; i++) {
      out[i].x = (int16_t)x;
      out[i].w = (int16_t)(out[i].b.w * cell);
      x += out[i].w + cell;
    }
    return n;
  }
  // Squeeze, no gaps; from the newest back, until the room runs out (the oldest scroll off)
  const float f = (float)width / (float)(sumW * cell);
  int used = 0, first = n;
  for (int i = n - 1; i >= 0; i--) {
    int w = std::max(MIN_W * cell, (int)(out[i].b.w * cell * f));
    if (used + w > width) break;
    out[i].w = (int16_t)w;
    used += w;
    first = i;
  }
  const int m = n - first;
  for (int i = 0; i < m; i++) out[i] = out[first + i];
  int x = (width - used) / 2;
  for (int i = 0; i < m; i++) {
    out[i].x = (int16_t)x;
    x += out[i].w;
  }
  return m;
}

}  // namespace city
