#include "sky.h"
#include <string.h>

namespace sky {

static int wrap(int minute) { return ((minute % 1440) + 1440) % 1440; }

Phase phase(int minute) {
  int m = wrap(minute);
  if (m < 330) return Phase::NIGHT;
  if (m < 450) return Phase::DAWN;
  if (m < 1050) return Phase::DAY;
  if (m < 1170) return Phase::DUSK;
  return Phase::NIGHT;
}

Rgb mix(Rgb a, Rgb b, float t) {
  if (t < 0) t = 0;
  if (t > 1) t = 1;
  return {(uint8_t)(a.r + (b.r - a.r) * t), (uint8_t)(a.g + (b.g - a.g) * t), (uint8_t)(a.b + (b.b - a.b) * t)};
}

struct Key {
  int   minute;
  Rgb   top, bottom;
  float light;
};
// The day's colours, blended between these
static const Key KEYS[] = {
    {0,    {0x07, 0x0b, 0x1f}, {0x16, 0x1d, 0x45}, 0.0f},   // night
    {330,  {0x0b, 0x10, 0x2c}, {0x22, 0x26, 0x55}, 0.05f},  // before dawn
    {390,  {0x2d, 0x3a, 0x78}, {0xf0, 0x92, 0x6c}, 0.45f},  // dawn: pink horizon
    {450,  {0x3d, 0x7c, 0xc4}, {0xf5, 0xc8, 0x8f}, 0.8f},   // sunrise gold
    {540,  {0x2f, 0x7f, 0xcf}, {0x9c, 0xcf, 0xef}, 1.0f},   // day
    {1020, {0x2f, 0x7f, 0xcf}, {0x9c, 0xcf, 0xef}, 1.0f},
    {1080, {0x3b, 0x5f, 0xa8}, {0xf4, 0xa2, 0x63}, 0.75f},  // golden hour
    {1140, {0x2b, 0x2a, 0x63}, {0xe0, 0x6f, 0x52}, 0.4f},   // dusk
    {1200, {0x0b, 0x10, 0x2c}, {0x1c, 0x22, 0x4e}, 0.05f},  // night again
    {1440, {0x07, 0x0b, 0x1f}, {0x16, 0x1d, 0x45}, 0.0f},
};

Colors colors(int minute, bool cloudy) {
  int m = wrap(minute);
  const int n = sizeof KEYS / sizeof KEYS[0];
  int i = 0;
  while (i < n - 2 && m >= KEYS[i + 1].minute) i++;
  const Key &a = KEYS[i], &b = KEYS[i + 1];
  float t = (float)(m - a.minute) / (float)(b.minute - a.minute);
  Colors c{mix(a.top, b.top, t), mix(a.bottom, b.bottom, t), a.light + (b.light - a.light) * t};
  if (cloudy) {  // grey it out, more by day
    const Rgb GREY_TOP{0x5a, 0x66, 0x75}, GREY_LOW{0x93, 0x9d, 0xa8};
    float k = 0.25f + 0.45f * c.light;
    c.top = mix(c.top, mix(c.top, GREY_TOP, c.light), k);
    c.bottom = mix(c.bottom, mix(c.bottom, GREY_LOW, c.light), k);
    c.light *= 0.85f;
  }
  return c;
}

bool sunAt(int minute, float& t) {
  int m = wrap(minute);
  if (m < 360 || m > 1140) return false;
  t = (m - 360) / 780.0f;
  return true;
}

bool moonAt(int minute, float& t) {
  int m = wrap(minute);
  if (m >= 1110) t = (m - 1110) / 720.0f;       // 18:30 -> 6:30 the next morning
  else if (m <= 390) t = (m + 330) / 720.0f;
  else return false;
  return true;
}

bool overcast(const char* icon) {
  if (!icon) return false;
  return !strcmp(icon, "cloudy") || !strcmp(icon, "fog") || !strcmp(icon, "rain") || !strcmp(icon, "snow") ||
         !strcmp(icon, "storm");
}

int clouds(const char* icon) {
  if (!icon || !strcmp(icon, "clear")) return 0;
  if (!strcmp(icon, "partly")) return 2;
  if (!strcmp(icon, "fog")) return 1;
  if (!strcmp(icon, "storm")) return 5;
  return 4;  // cloudy, rain, snow
}

}  // namespace sky
