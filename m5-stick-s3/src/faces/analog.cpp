// Analog: a clean dial with anti-aliased hands (logic/clockface.cpp), a ticking second hand
// in orange, the date and the weather beside it. Battery / Bluetooth at the top, anything
// urgent as a card under the dial (portrait) or in the right column (landscape).
// Redraws for the second hand: 5 times a second on USB, once a second on battery.
#include "faces.h"
#include "common.h"
#include "../logic/clockface.h"

namespace faces {

using namespace ui;

namespace {

constexpr uint16_t DIAL = rgb(0x1a, 0x1e, 0x23);
constexpr uint16_t RIM  = rgb(0x34, 0x3b, 0x43);
constexpr uint16_t HAND = rgb(0xf2, 0xf4, 0xf6);

class Analog : public Face {
 public:
  const char* id() const override { return "analog"; }
  const char* title() const override { return "Analog"; }
  const char* about() const override { return "A clean dial with a ticking second hand, the date and the weather"; }

  uint32_t frameMs(bool onBattery, uint32_t) const override { return onBattery ? 1000 : 200; }

  void draw(const Dash& d, bool wide, uint32_t now) override {
    canvas.fillScreen(GROUND);
    const Urgent u = urgent(d);
    if (wide) {
      dial(d, 68, 68, 60);
      const int x = 136, cx = 186;  // the right column
      statusIcons(d, W - 4, 9, INK);
      if (u.on) {
        signBoard(x, 24, W - x - 4, 98, u.a, u.b, u.bg, u.ink);
        return;
      }
      info(d, cx, 34, W - x - 8);
      return;
    }
    statusIcons(d, W - 4, 9, INK);
    dial(d, W / 2, 88, 61);
    if (u.on) signBoard(6, 160, W - 12, 62, u.a, u.b, u.bg, u.ink);
    else info(d, W / 2, 166, W - 12);
  }

 private:
  // The date and the weather, centred on cx from y down.
  void info(const Dash& d, int cx, int y, int room) {
    if (!d.agentLinked) {
      txt("agent offline", cx, y + 8, F12, DIM, middle_center);
      return;
    }
    const int sp = d.longDate.indexOf(' ');  // "Friday" over "6 November"
    if (d.longDate.length() && sp > 0 && W > H) {
      txt(d.longDate.substring(0, sp), cx, y, F18, INK, middle_center);
      txt(ellipsize(d.longDate.substring(sp + 1), F12, room), cx, y + 20, F12, DIM, middle_center);
    } else {
      txt(ellipsize(d.date.length() ? d.date : d.name, F18, room), cx, y, F18, INK, middle_center);
    }
    if (!d.wxValid) return;
    const int wy = y + (W > H ? 50 : 28);
    String t = String(d.wxTemp) + "°";
    canvas.setFont(F18);
    const int tw = canvas.textWidth(t), w = 20 + 4 + tw, x = cx - w / 2;
    weatherIcon(d.wxIcon, d.wxDay, x, wy - 10, 20);
    txt(t, x + 24, wy, F18, INK);
    String hl = "H " + String(d.wxHi) + "°  L " + String(d.wxLo) + "°";
    txt(ellipsize(hl, F12, room), cx, wy + 19, F12, DIM, middle_center);
  }

  void dial(const Dash& d, int cx, int cy, int r) {
    canvas.fillSmoothCircle(cx, cy, r, RIM);
    canvas.fillSmoothCircle(cx, cy, r - 2, DIAL);
    for (int i = 0; i < 60; i++) {  // the ticks: a dot a minute, a bar an hour, the 12 in teal
      const float a = i * PI / 30, s = sinf(a), c = -cosf(a);
      if (i % 5) {
        canvas.fillRect(cx + s * (r - 7), cy + c * (r - 7), 1, 1, FAINT);
        continue;
      }
      const float len = i % 15 ? 6 : 9;
      canvas.drawWideLine(cx + s * (r - 6), cy + c * (r - 6), cx + s * (r - 6 - len), cy + c * (r - 6 - len),
                          i % 15 ? 0.9f : 1.4f, i == 0 ? AGENT : i % 15 ? DIM : INK);
    }
    if (d.minute < 0) {  // no time yet: the hands rest at 10:10, the name under them
      txt(d.name, cx, cy + r / 2, fit(d.name, r, {F12, F9}), DIM, middle_center);
    }
    const clockface::Hands h = clockface::hands(d.minute >= 0 ? d.minute : 10 * 60 + 10, d.second >= 0 ? d.second : 0);
    // the shadow under the hands, then hour, minute, second
    hand(cx + 1, cy + 2, h.hour, r * 0.50f, 3.4f, 1.8f, rgb(0x08, 0x09, 0x0b));
    hand(cx + 1, cy + 2, h.minute, r * 0.80f, 2.6f, 1.2f, rgb(0x08, 0x09, 0x0b));
    hand(cx, cy, h.hour, r * 0.50f, 3.4f, 1.8f, HAND);
    hand(cx, cy, h.minute, r * 0.80f, 2.6f, 1.2f, HAND);
    if (d.minute >= 0 && d.second >= 0) {
      const float a = h.second * PI / 180;
      canvas.drawWideLine(cx - sinf(a) * r * 0.18f, cy + cosf(a) * r * 0.18f, cx + sinf(a) * (r - 8), cy - cosf(a) * (r - 8),
                          0.7f, ASK);
      canvas.fillSmoothCircle(cx, cy, 4, ASK);
    } else {
      canvas.fillSmoothCircle(cx, cy, 4, HAND);
    }
    canvas.fillSmoothCircle(cx, cy, 1, DIAL);
  }

  static void hand(int cx, int cy, float deg, float len, float r0, float r1, uint16_t c) {
    const float a = deg * PI / 180;
    canvas.drawWedgeLine(cx, cy, cx + sinf(a) * len, cy - cosf(a) * len, r0, r1, c);
  }
};

}  // namespace

Face& analog() {
  static Analog f;
  return f;
}

}  // namespace faces
