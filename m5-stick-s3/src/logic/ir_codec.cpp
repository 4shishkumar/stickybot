#include "ir_codec.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

namespace ircodec {

// --- Basics ----------------------------------------------------------------------------------

bool near(uint32_t m, uint32_t e) {
  const uint32_t d = m > e ? m - e : e - m;
  return d <= e * 30 / 100 + 120;
}

static bool nearRaw(uint32_t a, uint32_t b) {  // two captures of one button
  const uint32_t big = a > b ? a : b, d = a > b ? a - b : b - a;
  if (big > FRAME_GAP_US) return d <= big / 2;  // gaps between frames: timed less exactly
  return d <= big * 25 / 100 + 150;
}

const char* protoName(const Code& c) {
  switch (c.proto) {
    case Proto::NEC:      return "nec";
    case Proto::SAMSUNG:  return "samsung";
    case Proto::LG:       return "lg";
    case Proto::SONY:     return c.bits == 15 ? "sony15" : c.bits == 20 ? "sony20" : "sony12";
    case Proto::RC5:      return "rc5";
    case Proto::RC6:      return "rc6";
    case Proto::KASEIKYO: return c.vendor == PANASONIC_VENDOR ? "panasonic" : "kaseikyo";
    default:              return "raw";
  }
}

uint32_t carrierHz(Proto p) {
  switch (p) {
    case Proto::SONY:     return 40000;
    case Proto::RC5:
    case Proto::RC6:      return 36000;
    case Proto::KASEIKYO: return 37000;
    default:              return 38000;
  }
}

uint32_t periodUs(Proto p) {
  switch (p) {
    case Proto::SONY:     return 45000;
    case Proto::RC5:      return 113778;
    case Proto::RC6:      return 106667;
    case Proto::KASEIKYO: return 130000;
    case Proto::RAW:      return 0;  // the sender adds a gap after the capture
    default:              return 108000;  // nec, samsung, lg
  }
}

bool hasRepeatFrame(Proto p) { return p == Proto::NEC || p == Proto::LG; }
uint8_t minFrames(Proto p) { return p == Proto::SONY ? 3 : 1; }

// --- Pulse-distance protocols (nec, samsung, lg, kaseikyo) -------------------------------------

struct PD {
  uint16_t hm, hs, bm, one, zero;
};
static const PD NEC_T = {9000, 4500, 560, 1690, 560};
static const PD SAMSUNG_T = {4500, 4500, 560, 1690, 560};
static const PD LG_T = {8500, 4250, 550, 1600, 550};
static const PD KASEIKYO_T = {3456, 1728, 432, 1296, 432};

static size_t pdEncode(const PD& p, uint64_t v, int nbits, bool msb, uint16_t* out, size_t max) {
  const size_t need = 2 + 2 * nbits + 1;
  if (need > max) return 0;
  size_t k = 0;
  out[k++] = p.hm;
  out[k++] = p.hs;
  for (int i = 0; i < nbits; i++) {
    const int bit = msb ? nbits - 1 - i : i;
    out[k++] = p.bm;
    out[k++] = (v >> bit) & 1 ? p.one : p.zero;
  }
  out[k++] = p.bm;
  return k;
}

static bool pdDecode(const PD& p, const uint16_t* t, size_t n, int nbits, bool msb, uint64_t& v) {
  if (n != (size_t)(2 + 2 * nbits + 1)) return false;
  if (!near(t[0], p.hm) || !near(t[1], p.hs)) return false;
  v = 0;
  for (int i = 0; i < nbits; i++) {
    const uint16_t m = t[2 + 2 * i], s = t[3 + 2 * i];
    if (!near(m, p.bm)) return false;
    uint64_t b;
    if (near(s, p.one)) b = 1;
    else if (near(s, p.zero)) b = 0;
    else return false;
    const int bit = msb ? nbits - 1 - i : i;
    v |= b << bit;
  }
  return near(t[n - 1], p.bm);
}

static uint32_t necWord(uint32_t addr, uint32_t cmd, bool samsung) {
  const uint32_t a0 = addr & 0xFF, c0 = cmd & 0xFF;
  const uint32_t a1 = addr > 0xFF ? (addr >> 8) & 0xFF : samsung ? a0 : (~a0 & 0xFF);
  const uint32_t c1 = cmd > 0xFF ? (cmd >> 8) & 0xFF : (~c0 & 0xFF);
  return a0 | a1 << 8 | c0 << 16 | c1 << 24;
}

static void necFields(uint32_t w, bool samsung, Code& c) {
  const uint32_t a0 = w & 0xFF, a1 = (w >> 8) & 0xFF, c0 = (w >> 16) & 0xFF, c1 = w >> 24;
  const bool shortAddr = samsung ? a1 == a0 : a1 == (~a0 & 0xFF);
  c.addr = shortAddr ? a0 : (a0 | a1 << 8);
  c.cmd = c1 == (~c0 & 0xFF) ? c0 : (c0 | c1 << 8);
}

static uint8_t lgChecksum(uint32_t cmd) {
  return ((cmd & 0xF) + ((cmd >> 4) & 0xF) + ((cmd >> 8) & 0xF) + ((cmd >> 12) & 0xF)) & 0xF;
}

static uint8_t vendorParity(uint16_t vendor) {
  uint8_t p = (vendor ^ (vendor >> 8)) & 0xFF;
  return (p ^ (p >> 4)) & 0xF;
}

static uint64_t kaseikyoWord(const Code& c) {
  const uint64_t b0 = c.vendor & 0xFF, b1 = c.vendor >> 8;
  const uint64_t b2 = vendorParity(c.vendor) | (c.addr & 0xF) << 4, b3 = (c.addr >> 4) & 0xFF, b4 = c.cmd & 0xFF;
  const uint64_t b5 = b2 ^ b3 ^ b4;
  return b0 | b1 << 8 | b2 << 16 | b3 << 24 | b4 << 32 | b5 << 40;
}

// --- Sony SIRC -------------------------------------------------------------------------------

static size_t sonyEncode(const Code& c, uint16_t* out, size_t max) {
  const int nbits = c.bits;
  const size_t need = 2 + 2 * nbits - 1;
  if (need > max) return 0;
  const uint32_t v = (c.cmd & 0x7F) | c.addr << 7;
  size_t k = 0;
  out[k++] = 2400;
  out[k++] = 600;
  for (int i = 0; i < nbits; i++) {
    out[k++] = (v >> i) & 1 ? 1200 : 600;
    if (i < nbits - 1) out[k++] = 600;
  }
  return k;
}

static bool sonyDecode(const uint16_t* t, size_t n, Code& c) {
  int nbits = n == 25 ? 12 : n == 31 ? 15 : n == 41 ? 20 : 0;
  if (!nbits || !near(t[0], 2400) || !near(t[1], 600)) return false;
  uint32_t v = 0;
  for (int i = 0; i < nbits; i++) {
    const uint16_t m = t[2 + 2 * i];
    if (!near(m, 600) && !near(m, 1200)) return false;
    if (m > 900) v |= 1u << i;
    if (i < nbits - 1 && !near(t[3 + 2 * i], 600)) return false;
  }
  c = Code{};
  c.proto = Proto::SONY;
  c.bits = nbits;
  c.cmd = v & 0x7F;
  c.addr = v >> 7;
  return true;
}

// --- Bi-phase (rc5, rc6): half-bit levels <-> durations ----------------------------------------

// levels (1 mark, 0 space), each one half-bit unit long -> durations from the first mark to the
// last one
static size_t runs(const uint8_t* lv, size_t nl, uint16_t unit, uint16_t* out, size_t max) {
  size_t i = 0, k = 0;
  while (i < nl && !lv[i]) i++;
  size_t end = nl;
  while (end > i && !lv[end - 1]) end--;
  while (i < end) {
    size_t j = i;
    while (j < end && lv[j] == lv[i]) j++;
    if (k >= max) return 0;
    out[k++] = (uint16_t)((j - i) * unit);
    i = j;
  }
  return k;
}

// durations -> levels (starting with `first` units of level 0 before the first mark); false if a
// duration isn't 1..maxUnits whole units or it runs past nl
static bool levels(const uint16_t* t, size_t n, uint16_t unit, int maxUnits, int first, uint8_t* lv, size_t nl,
                   size_t& used) {
  size_t k = 0;
  for (int i = 0; i < first && k < nl; i++) lv[k++] = 0;
  for (size_t i = 0; i < n; i++) {
    const int u = (t[i] + unit / 2) / unit;
    if (u < 1 || u > maxUnits) return false;
    for (int j = 0; j < u; j++) {
      if (k >= nl) return false;
      lv[k++] = (i % 2 == 0) ? 1 : 0;
    }
  }
  used = k;
  while (k < nl) lv[k++] = 0;  // the trailing space isn't seen
  return true;
}

constexpr uint16_t RC5_T = 889, RC6_T = 444;

static size_t rc5Encode(const Code& c, uint16_t* out, size_t max, bool toggle) {
  uint8_t bits[14], lv[28];
  bits[0] = 1;
  bits[1] = (c.cmd & 0x40) ? 0 : 1;
  bits[2] = toggle;
  for (int i = 0; i < 5; i++) bits[3 + i] = (c.addr >> (4 - i)) & 1;
  for (int i = 0; i < 6; i++) bits[8 + i] = (c.cmd >> (5 - i)) & 1;
  for (int i = 0; i < 14; i++) {  // rc5: 1 = space then mark, 0 = mark then space
    lv[2 * i] = bits[i] ? 0 : 1;
    lv[2 * i + 1] = bits[i] ? 1 : 0;
  }
  return runs(lv, 28, RC5_T, out, max);
}

static bool rc5Decode(const uint16_t* t, size_t n, Code& c) {
  if (n < 3 || n > 27) return false;
  uint8_t lv[28];
  size_t used;
  if (!levels(t, n, RC5_T, 2, 1, lv, 28, used) || used < 20) return false;
  uint32_t bits = 0;
  for (int i = 0; i < 14; i++) {
    const uint8_t a = lv[2 * i], b = lv[2 * i + 1];
    if (a == b) return false;
    bits = bits << 1 | (b ? 1 : 0);
  }
  if (!(bits >> 13)) return false;  // the start bit is 1
  c = Code{};
  c.proto = Proto::RC5;
  c.addr = (bits >> 6) & 0x1F;
  c.cmd = (bits & 0x3F) | (((bits >> 12) & 1) ? 0 : 0x40);
  return true;
}

static size_t rc6Encode(const Code& c, uint16_t* out, size_t max, bool toggle) {
  uint8_t lv[8 + 44];
  size_t k = 0;
  for (int i = 0; i < 6; i++) lv[k++] = 1;
  lv[k++] = 0;
  lv[k++] = 0;
  auto bit = [&](int b) {  // rc6: 1 = mark then space, 0 = space then mark
    lv[k++] = b ? 1 : 0;
    lv[k++] = b ? 0 : 1;
  };
  bit(1);
  for (int i = 0; i < 3; i++) bit(0);  // mode 0
  lv[k++] = toggle;
  lv[k++] = toggle;
  lv[k++] = !toggle;
  lv[k++] = !toggle;
  for (int i = 7; i >= 0; i--) bit((c.addr >> i) & 1);
  for (int i = 7; i >= 0; i--) bit((c.cmd >> i) & 1);
  return runs(lv, k, RC6_T, out, max);
}

static bool rc6Decode(const uint16_t* t, size_t n, Code& c) {
  if (n < 5 || n > 47) return false;
  const int lead = (t[0] + RC6_T / 2) / RC6_T, gap = (t[1] + RC6_T / 2) / RC6_T;
  if (lead != 6 || gap != 2) return false;
  uint8_t lv[44];
  size_t used;
  if (!levels(t + 2, n - 2, RC6_T, 3, 0, lv, 44, used) || used < 30) return false;
  int k = 0;
  auto bit = [&](int& b) {
    const uint8_t x = lv[k], y = lv[k + 1];
    k += 2;
    if (x == y) return false;
    b = x;
    return true;
  };
  int b;
  if (!bit(b) || b != 1) return false;  // start bit
  for (int i = 0; i < 3; i++)
    if (!bit(b) || b != 0) return false;  // mode 0 only
  if (lv[k] != lv[k + 1] || lv[k + 2] != lv[k + 3] || lv[k] == lv[k + 2]) return false;  // toggle, double width
  k += 4;
  uint32_t addr = 0, cmd = 0;
  for (int i = 0; i < 8; i++) {
    if (!bit(b)) return false;
    addr = addr << 1 | b;
  }
  for (int i = 0; i < 8; i++) {
    if (!bit(b)) return false;
    cmd = cmd << 1 | b;
  }
  c = Code{};
  c.proto = Proto::RC6;
  c.addr = addr;
  c.cmd = cmd;
  return true;
}

// --- Encode / decode -----------------------------------------------------------------------------

static bool valid(const Code& c) {
  switch (c.proto) {
    case Proto::NEC:
    case Proto::SAMSUNG:  return c.addr <= 0xFFFF && c.cmd <= 0xFFFF;
    case Proto::LG:       return c.addr <= 0xFF && c.cmd <= 0xFFFF;
    case Proto::SONY:
      return c.cmd <= 0x7F && ((c.bits == 12 && c.addr <= 0x1F) || (c.bits == 15 && c.addr <= 0xFF) ||
                               (c.bits == 20 && c.addr <= 0x1FFF));
    case Proto::RC5:      return c.addr <= 0x1F && c.cmd <= 0x7F;
    case Proto::RC6:      return c.addr <= 0xFF && c.cmd <= 0xFF;
    case Proto::KASEIKYO: return c.addr <= 0xFFF && c.cmd <= 0xFF;
    default:              return false;
  }
}

size_t encode(const Code& c, uint16_t* out, size_t max, bool toggle) {
  if (!valid(c)) return 0;
  switch (c.proto) {
    case Proto::NEC:      return pdEncode(NEC_T, necWord(c.addr, c.cmd, false), 32, false, out, max);
    case Proto::SAMSUNG:  return pdEncode(SAMSUNG_T, necWord(c.addr, c.cmd, true), 32, false, out, max);
    case Proto::LG:
      return pdEncode(LG_T, (uint64_t)c.addr << 20 | (uint64_t)c.cmd << 4 | lgChecksum(c.cmd), 28, true, out, max);
    case Proto::SONY:     return sonyEncode(c, out, max);
    case Proto::RC5:      return rc5Encode(c, out, max, toggle);
    case Proto::RC6:      return rc6Encode(c, out, max, toggle);
    case Proto::KASEIKYO: return pdEncode(KASEIKYO_T, kaseikyoWord(c), 48, false, out, max);
    default:              return 0;
  }
}

size_t encodeRepeat(const Code& c, uint16_t* out, size_t max, bool toggle) {
  if (!valid(c)) return 0;
  if (hasRepeatFrame(c.proto)) {
    if (max < 3) return 0;
    const bool lg = c.proto == Proto::LG;
    out[0] = lg ? 8500 : 9000;
    out[1] = 2250;
    out[2] = lg ? 550 : 560;
    return 3;
  }
  return encode(c, out, max, toggle);  // the others repeat the whole frame (rc5 / rc6: same toggle)
}

bool isNecRepeat(const uint16_t* t, size_t n) {
  return n == 3 && t[0] > 7000 && t[0] < 11000 && near(t[1], 2250) && near(t[2], 560);
}

bool decode(const uint16_t* t, size_t n, Code& out) {
  if (!t || n < 3 || !(n % 2)) return false;
  uint64_t v;
  Code c;
  if (pdDecode(NEC_T, t, n, 32, false, v)) {
    c.proto = Proto::NEC;
    necFields((uint32_t)v, false, c);
  } else if (pdDecode(SAMSUNG_T, t, n, 32, false, v)) {
    c.proto = Proto::SAMSUNG;
    necFields((uint32_t)v, true, c);
  } else if (pdDecode(LG_T, t, n, 28, true, v)) {
    if (lgChecksum((v >> 4) & 0xFFFF) != (v & 0xF)) return false;
    c.proto = Proto::LG;
    c.addr = (v >> 20) & 0xFF;
    c.cmd = (v >> 4) & 0xFFFF;
  } else if (pdDecode(KASEIKYO_T, t, n, 48, false, v)) {
    const uint8_t b2 = v >> 16, b3 = v >> 24, b4 = v >> 32, b5 = v >> 40;
    c.vendor = v & 0xFFFF;
    if ((b2 & 0xF) != vendorParity(c.vendor) || b5 != (uint8_t)(b2 ^ b3 ^ b4)) return false;
    c.proto = Proto::KASEIKYO;
    c.addr = (b2 >> 4) | (uint32_t)b3 << 4;
    c.cmd = b4;
  } else if (rc6Decode(t, n, c) || sonyDecode(t, n, c) || rc5Decode(t, n, c)) {  // rc6 first: its frames pass for sony20
  } else {
    return false;
  }
  out = c;
  return true;
}

size_t firstFrame(const uint16_t* t, size_t n) {
  for (size_t i = 1; i < n; i += 2)
    if (t[i] > FRAME_GAP_US) return i;
  return n;
}

bool decodeCapture(const uint16_t* t, size_t n, Code& out) { return decode(t, firstFrame(t, n), out); }

// --- Raw captures ------------------------------------------------------------------------------

size_t cleanRaw(uint16_t* t, size_t n) {
  if (n % 2 == 0 && n) n--;  // ends with a space: drop it
  size_t w = 0, i = 0;
  while (i < n) {
    const size_t len = firstFrame(t + i, n - i);
    const bool keep = !isNecRepeat(t + i, len);
    if (keep) {
      if (w) t[w++] = i ? t[i - 1] : 40000;  // the gap before it
      memmove(t + w, t + i, len * sizeof(uint16_t));
      w += len;
    }
    i += len + 1;  // past the gap
  }
  return w;
}

bool sameRaw(const uint16_t* a, size_t na, const uint16_t* b, size_t nb) {
  if (na != nb || !na) return false;
  for (size_t i = 0; i < na; i++)
    if (!nearRaw(a[i], b[i])) return false;
  return true;
}

// --- Code strings --------------------------------------------------------------------------------

static const char DIGITS[] = "0123456789abcdefghijklmnopqrstuvwxyz";

static bool hexField(const char* s, size_t len, uint32_t& v) {
  if (!len || len > 8) return false;
  v = 0;
  for (size_t i = 0; i < len; i++) {
    char ch = s[i];
    int d = ch >= '0' && ch <= '9' ? ch - '0' : ch >= 'a' && ch <= 'f' ? ch - 'a' + 10
          : ch >= 'A' && ch <= 'F' ? ch - 'A' + 10 : -1;
    if (d < 0) return false;
    v = v << 4 | d;
  }
  return true;
}

static bool decField(const char* s, size_t len, uint32_t& v, uint32_t maxV) {
  if (!len || len > 6) return false;
  v = 0;
  for (size_t i = 0; i < len; i++) {
    if (s[i] < '0' || s[i] > '9') return false;
    v = v * 10 + (s[i] - '0');
  }
  return v <= maxV;
}

// "a:b:c" -> up to max parts (pointers + lengths)
static int split(const char* s, char sep, const char* parts[], size_t lens[], int max) {
  int k = 0;
  const char* p = s;
  while (k < max) {
    const char* e = strchr(p, sep);
    parts[k] = p;
    lens[k] = e ? (size_t)(e - p) : strlen(p);
    k++;
    if (!e) return k;
    p = e + 1;
  }
  return -1;  // too many
}

static bool parseRaw(const char* rest, Parsed& p, uint16_t* raw) {
  const char* parts[3];
  size_t lens[3];
  const int np = split(rest, ':', parts, lens, 3);
  if (np < 2 || np > 3 || !raw) return false;
  uint32_t khz;
  if (!decField(parts[0], lens[0], khz, 60) || khz < 20) return false;
  uint16_t table[36];
  int nt = 0;
  size_t n = 0;
  // the comma list (durations, or the compact form's values)
  const char* s = parts[1];
  const char* end = s + lens[1];
  uint16_t* dst = np == 3 ? table : raw;
  const size_t cap = np == 3 ? 36 : MAX_RAW;
  size_t k = 0;
  while (s < end) {
    const char* comma = (const char*)memchr(s, ',', end - s);
    const char* e = comma ? comma : end;
    uint32_t v;
    if (k >= cap || !decField(s, e - s, v, 65535) || !v) return false;
    dst[k++] = (uint16_t)v;
    s = comma ? comma + 1 : end;
    if (comma && s == end) return false;  // trailing comma
  }
  if (np == 3) {
    nt = (int)k;
    if (!nt || lens[2] > MAX_RAW) return false;
    for (size_t i = 0; i < lens[2]; i++) {
      const char* d = strchr(DIGITS, parts[2][i]);
      if (!d || !*d || d - DIGITS >= nt) return false;
      raw[n++] = table[d - DIGITS];
    }
  } else {
    n = k;
  }
  if (n % 2 == 0 && n) n--;  // a trailing space isn't sent
  if (!n) return false;
  p.code = Code{};
  p.khz = (uint16_t)khz;
  p.n = (uint16_t)n;
  return true;
}

bool parse(const char* s, Parsed& p, uint16_t* raw) {
  if (!s) return false;
  const char* colon = strchr(s, ':');
  if (!colon) return false;
  const size_t nl = colon - s;
  char name[12];
  if (nl >= sizeof name) return false;
  for (size_t i = 0; i < nl; i++) name[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
  name[nl] = 0;
  const char* rest = colon + 1;
  if (!strcmp(name, "raw")) return parseRaw(rest, p, raw);

  const char* parts[3];
  size_t lens[3];
  const int np = split(rest, ':', parts, lens, 3);
  Code c;
  uint32_t a = 0, b = 0, v = 0;
  if (!strcmp(name, "kaseikyo")) {
    if (np != 3 || !hexField(parts[0], lens[0], v) || !hexField(parts[1], lens[1], a) ||
        !hexField(parts[2], lens[2], b) || v > 0xFFFF)
      return false;
    c.proto = Proto::KASEIKYO;
    c.vendor = (uint16_t)v;
  } else {
    if (np != 2 || !hexField(parts[0], lens[0], a) || !hexField(parts[1], lens[1], b)) return false;
    if (!strcmp(name, "nec")) c.proto = Proto::NEC;
    else if (!strcmp(name, "samsung")) c.proto = Proto::SAMSUNG;
    else if (!strcmp(name, "lg")) c.proto = Proto::LG;
    else if (!strcmp(name, "sony12") || !strcmp(name, "sony")) { c.proto = Proto::SONY; c.bits = 12; }
    else if (!strcmp(name, "sony15")) { c.proto = Proto::SONY; c.bits = 15; }
    else if (!strcmp(name, "sony20")) { c.proto = Proto::SONY; c.bits = 20; }
    else if (!strcmp(name, "rc5")) c.proto = Proto::RC5;
    else if (!strcmp(name, "rc6")) c.proto = Proto::RC6;
    else if (!strcmp(name, "panasonic")) { c.proto = Proto::KASEIKYO; c.vendor = PANASONIC_VENDOR; }
    else return false;
  }
  c.addr = a;
  c.cmd = b;
  if (!valid(c)) return false;
  p = Parsed{};
  p.code = c;
  return true;
}

size_t format(const Code& c, char* out, size_t max) {
  if (!valid(c) || !out || !max) return 0;
  int n;
  if (c.proto == Proto::KASEIKYO && c.vendor != PANASONIC_VENDOR)
    n = snprintf(out, max, "kaseikyo:%x:%x:%x", (unsigned)c.vendor, (unsigned)c.addr, (unsigned)c.cmd);
  else
    n = snprintf(out, max, "%s:%x:%x", protoName(c), (unsigned)c.addr, (unsigned)c.cmd);
  if (n < 0 || (size_t)n >= max) {
    out[0] = 0;
    return 0;
  }
  return n;
}

static int digits(uint32_t v) {
  int d = 1;
  while (v >= 10) {
    v /= 10;
    d++;
  }
  return d;
}

size_t formatRaw(const uint16_t* t, size_t n, uint16_t khz, char* out, size_t max) {
  if (!t || !n || n > MAX_RAW || !out || !max) return 0;
  if (n % 2 == 0) n--;
  // Cluster the durations (first come, first served: a value joins the first cluster whose first
  // value is within 12 % + 40 us), at most 36 of them
  uint32_t ref[36], sum[36], cnt[36];
  int nc = 0;
  bool compact = true;
  size_t plain = 0;
  for (size_t i = 0; i < n; i++) {
    plain += digits(t[i]) + 1;
    if (!compact) continue;
    int c = 0;
    for (; c < nc; c++) {
      const uint32_t d = t[i] > ref[c] ? t[i] - ref[c] : ref[c] - t[i];
      if (d <= ref[c] * 12 / 100 + 40) break;
    }
    if (c == nc) {
      if (nc == 36) { compact = false; continue; }
      ref[nc] = t[i];
      sum[nc] = cnt[nc] = 0;
      nc++;
    }
    sum[c] += t[i];
    cnt[c]++;
  }
  size_t k = 0;
  auto put = [&](const char* s) {
    const size_t l = strlen(s);
    if (k + l >= max) return false;
    memcpy(out + k, s, l);
    k += l;
    out[k] = 0;
    return true;
  };
  char num[12];
  snprintf(num, sizeof num, "raw:%u:", (unsigned)khz);
  if (!put(num)) return 0;
  size_t compactLen = 0;
  if (compact) {
    for (int c = 0; c < nc; c++) compactLen += digits((sum[c] + cnt[c] / 2) / cnt[c]) + 1;
    compactLen += n;
  }
  if (compact && compactLen < plain) {
    for (int c = 0; c < nc; c++) {
      snprintf(num, sizeof num, c ? ",%u" : "%u", (unsigned)((sum[c] + cnt[c] / 2) / cnt[c]));
      if (!put(num)) return 0;
    }
    if (!put(":")) return 0;
    for (size_t i = 0; i < n; i++) {
      int c = 0;
      for (; c < nc; c++) {
        const uint32_t d = t[i] > ref[c] ? t[i] - ref[c] : ref[c] - t[i];
        if (d <= ref[c] * 12 / 100 + 40) break;
      }
      const char ch[2] = {DIGITS[c], 0};
      if (!put(ch)) return 0;
    }
  } else {
    for (size_t i = 0; i < n; i++) {
      snprintf(num, sizeof num, i ? ",%u" : "%u", (unsigned)t[i]);
      if (!put(num)) return 0;
    }
  }
  return k;
}

}  // namespace ircodec
