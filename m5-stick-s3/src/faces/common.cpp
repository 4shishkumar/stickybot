#include "common.h"

namespace faces {

using namespace ui;

Urgent urgent(const Dash& d) {
  Urgent u;
  if (d.bt != 0) {
    u.on = true;
    u.a = d.bt == 1 ? "Waiting for the Mac" : "Bluetooth off";
    u.b = d.bt == 1 ? "pair or wake it" : "menu > Reconnect BT";
    u.bg = rgb(0x5b, 0x63, 0x6d);
    u.ink = INK;
  } else if (d.battery >= 0 && d.battery <= 15 && !d.charging) {
    u.on = true;
    u.a = "Battery low";
    u.b = String(d.battery) + "% - plug me in";
    u.bg = ERR;
    u.ink = rgb(0x2a, 0x05, 0x05);
  } else if (d.agentLinked && d.cardUrgent) {
    u.on = true;
    u.a = d.cardA;
    u.b = d.cardB;
    u.bg = ASK;
    u.ink = ON_ASK;
  }
  return u;
}

String quietNotice(const Dash& d) { return d.agentLinked ? String() : String("agent offline - A still dictates"); }

void shadowTxt(const String& s, int x, int y, const lgfx::IFont* f, uint16_t ink, uint16_t shade, textdatum_t datum) {
  txt(s, x + 1, y + 1, f, shade, datum);
  txt(s, x, y, f, ink, datum);
}

int timeText(const Dash& d, int x, int y, const lgfx::IFont* f, uint16_t ink, uint16_t shade) {
  if (!d.clock.length()) {
    shadowTxt(d.name, x, y, F12, ink, shade);
    canvas.setFont(F12);
    return canvas.textWidth(d.name);
  }
  canvas.setFont(f);
  int cw = canvas.textWidth(d.clock);
  shadowTxt(d.clock, x, y, f, ink, shade);
  shadowTxt(d.ampm, x + cw + 3, y + (f == F24 ? 6 : 4), F9, blend(ink, shade, 0.25f), shade);
  canvas.setFont(F9);
  return cw + 3 + canvas.textWidth(d.ampm);
}

void statusIcons(const Dash& d, int right, int cy, uint16_t ink) {
  int r = right - 18;
  batteryIcon(r, cy - 4, d.battery, d.charging);
  if (d.battery >= 0) {
    String pct = String(d.battery) + "%";
    canvas.setFont(F9);
    r -= canvas.textWidth(pct) + 3;
    txt(pct, r, cy, F9, d.charging ? ui::OK : ink);
  }
  r -= 8 + 7;
  btIcon(r, cy - 5, d.bt == 0 ? ink : d.bt == 1 ? ASK : FAINT);
  canvas.fillCircle(r - 7, cy, 2, d.agentLinked ? AGENT : FAINT);
}

void signBoard(int x, int y, int w, int h, const String& a, const String& b, uint16_t bg, uint16_t ink) {
  canvas.fillRoundRect(x, y, w, h, 6, bg);
  canvas.drawRoundRect(x, y, w, h, 6, blend(bg, TFT_BLACK, 0.35f));
  const int room = w - 12;
  const lgfx::IFont* fa = fit(a, room, {F14, F12, F9});
  int la = b.length() ? (h >= 50 ? 2 : 1) : 0;
  if (!b.length()) {
    String l[3];
    int n = wrap(a, fa, l, h >= 44 ? 2 : 1, room);
    for (int i = 0; i < n; i++) txt(l[i], x + w / 2, y + h / 2 - (n - 1) * 8 + i * 16, fa, ink, middle_center);
    return;
  }
  txt(ellipsize(a, fa, room), x + 6, y + 11, fa, ink);
  String l[2];
  const lgfx::IFont* fb = fit(b, room * la, {F12, F9});
  int n = wrap(b, fb, l, la, room);
  for (int i = 0; i < n; i++) txt(l[i], x + 6, y + 27 + i * 14, fb, blend(ink, bg, 0.15f));
}

void speech(int x, int y, int w, int h, int tailX, const String& a, const String& b, uint16_t bg, uint16_t ink) {
  tailX = constrain(tailX, x + 10, x + w - 10);
  canvas.fillRoundRect(x, y, w, h, 8, bg);
  canvas.fillTriangle(tailX - 5, y + h - 1, tailX + 5, y + h - 1, tailX, y + h + 6, bg);
  const int room = w - 12;
  if (b.length()) {
    txt(ellipsize(a, F12, room), x + 6, y + 10, F12, ink);
    String l[2];
    int n = wrap(b, F9, l, (h - 20) / 12, room);
    for (int i = 0; i < n; i++) txt(l[i], x + 6, y + 24 + i * 12, F9, blend(ink, bg, 0.25f));
  } else {
    String l[2];
    int n = wrap(a, F12, l, 2, room);
    for (int i = 0; i < n; i++) txt(l[i], x + w / 2, y + h / 2 - (n - 1) * 7 + i * 14, F12, ink, middle_center);
  }
}

void pill(const String& s, int cx, int cy, uint16_t bg, uint16_t ink) {
  canvas.setFont(F12);
  int w = canvas.textWidth(s) + 12;
  canvas.fillRoundRect(cx - w / 2, cy - 8, w, 16, 8, bg);
  txt(s, cx, cy, F12, ink, middle_center);
}

uint32_t hash32(uint32_t x) {
  x ^= x >> 16;
  x *= 0x7feb352d;
  x ^= x >> 15;
  x *= 0x846ca68b;
  x ^= x >> 16;
  return x;
}

float frac01(uint32_t x) { return (hash32(x) & 0xFFFF) / 65535.0f; }

uint16_t c565(uint8_t r, uint8_t g, uint8_t b) { return rgb(r, g, b); }

bool isNightMinute(int minute) { return minute >= 0 && (minute >= 22 * 60 || minute < 6 * 60); }

}  // namespace faces
