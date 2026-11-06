// Dashboard: everything at a glance in a dense grid. The time and the date, the weather with
// the day's high and low, battery / Bluetooth, four tiles for what the agent knows (approvals
// and questions waiting, review requests, Claude sessions working, CI with today's merges)
// and the agent's current card at the bottom (orange when it needs you). Static: drawn when
// something changes.
#include "faces.h"
#include "common.h"

namespace faces {

using namespace ui;

namespace {

struct Tile {
  String   value, label;
  uint16_t accent;  // the value's colour; a hot tile gets a tinted background
  bool     hot;
  String   extra;   // small, after the label ("+2 PRs" merged today)
};

class Dashboard : public Face {
 public:
  const char* id() const override { return "dashboard"; }
  const char* title() const override { return "Dashboard"; }
  const char* about() const override {
    return "Time, weather, battery, what waits for you, Claude, CI and the current card in one grid";
  }

  void draw(const Dash& d, bool wide, uint32_t) override {
    canvas.fillScreen(GROUND);
    const world::Snapshot& w = faces::world();
    const bool known = d.agentLinked && w.valid;
    auto n = [&](int v) { return known ? String(v) : String("-"); };
    const Tile tiles[4] = {
        {n(w.asks), w.asks == 1 ? "needs you" : "need you", ASK, known && w.asks > 0, ""},
        {n(w.reviews), w.reviews == 1 ? "review" : "reviews", ui::BLUE, known && w.reviews > 0, ""},
        {n(w.nSessions), "Claude", ui::VIOLET, false, ""},
        {!known || w.ci == world::Ci::NONE ? String("-") : w.ci == world::Ci::GREEN ? String("ok") : String("fail"), "CI",
         w.ci == world::Ci::RED ? ERR : ui::OK, known && w.ci == world::Ci::RED,
         known && w.mergedToday ? "+" + String(w.mergedToday) + " PR" + (w.mergedToday == 1 ? "" : "s") : String()},
    };
    statusIcons(d, W - 4, 9, INK);
    if (wide) {
      const bool ampm = clockAt(d, 6, 34, 112);
      dateLine(d, ampm, 6, 62, 112);
      weather(d, 6, 84, 112);
      const int x0 = 122, tw = (W - 4 - x0 - 4) / 2;
      for (int i = 0; i < 4; i++) tile(tiles[i], x0 + (i % 2) * (tw + 4), 20 + (i / 2) * 34, tw, 30);
      card(d, x0, 90, W - 4 - x0, 32);
      return;
    }
    const bool ampm = clockAt(d, 6, 38, W - 12);
    dateLine(d, ampm, 6, 64, W - 12);
    weather(d, 6, 88, W - 12);
    const int tw = (W - 12 - 4) / 2;
    for (int i = 0; i < 4; i++) tile(tiles[i], 6 + (i % 2) * (tw + 4), 116 + (i / 2) * 36, tw, 32);
    card(d, 6, 190, W - 12, 36);
  }
  // frameMs 0: drawn when something changes

 private:
  // The time big; AM / PM beside it if there's room (returns whether it was).
  static bool clockAt(const Dash& d, int x, int y, int room) {
    if (!d.clock.length()) {
      txt(ellipsize(d.name, F18, room), x, y, F18, INK);
      return true;
    }
    canvas.setFont(F44);
    const int cw = canvas.textWidth(d.clock);
    const lgfx::IFont* f = cw <= room ? F44 : F24;
    txt(d.clock, x, y, f, INK);
    if (f != F44 || !clockFits(d, F44, room)) return false;
    txt(d.ampm, x + cw + 3, y - 11, F12, DIM);
    return true;
  }

  // The date, after AM / PM when that had no room by the clock.
  static void dateLine(const Dash& d, bool ampmShown, int x, int y, int room) {
    String s = ampmShown || !d.ampm.length() ? d.date : d.ampm + "  " + d.date;
    if (s.length()) txt(ellipsize(s, F12, room), x, y, F12, DIM);
  }

  // The icon, the temperature, the sky in words and the day's high / low.
  static void weather(const Dash& d, int x, int y, int room) {
    if (!d.wxValid) {
      txt("no weather yet", x, y, F9, FAINT);
      return;
    }
    weatherIcon(d.wxIcon, d.wxDay, x, y - 10, 20);
    String t = String(d.wxTemp) + "°";
    txt(t, x + 24, y, F18, INK);
    canvas.setFont(F18);
    const int tx = x + 24 + canvas.textWidth(t) + 6;
    String hl = String(d.wxHi) + "° / " + String(d.wxLo) + "°";  // the day's high / low
    txt(ellipsize(hl, F9, x + room - tx), tx, y - 6, F9, INK);
    txt(ellipsize(d.wxCond, F9, x + room - tx), tx, y + 6, F9, DIM);
  }

  static void tile(const Tile& t, int x, int y, int w, int h) {
    const uint16_t bg = t.hot ? blend(PANEL, t.accent, 0.22f) : PANEL;
    canvas.fillRoundRect(x, y, w, h, 6, bg);
    const int vy = y + h / 2 - 5;
    const bool none = t.value == "-" || t.value == "0";  // nothing there: quiet
    txt(t.value, x + 6, vy, t.value.length() > 2 ? F14 : F18, none ? FAINT : t.accent);
    txt(ellipsize(t.label, F9, w - 10), x + 6, y + h - 7, F9, t.hot ? INK : DIM);
    if (t.extra.length()) txt(t.extra, x + w - 5, y + h - 7, F9, ui::OK, middle_right);
  }

  // The card: the stick's own news first (urgent()), then the agent's, then a hint.
  static void card(const Dash& d, int x, int y, int w, int h) {
    const Urgent u = urgent(d);
    String a = u.a, b = u.b;
    uint16_t bg = u.bg, ink = u.ink;
    if (!u.on) {
      bg = CHIP;
      ink = INK;
      if (!d.agentLinked) {
        a = "Agent offline";
        b = "A still dictates";
      } else if (!d.cardKind.length() || d.cardKind == "face" || d.cardKind == "hello") {
        a = "Hold B to ask";
        b = "A dictates";
      } else {
        a = d.cardA;
        b = d.cardB;
      }
    }
    canvas.fillRoundRect(x, y, w, h, 8, bg);
    txt(ellipsize(a, F12, w - 12), x + 6, y + 10, F12, ink);
    txt(ellipsize(b, F9, w - 12), x + 6, y + h - 9, F9, u.on ? ink : DIM);
  }
};

}  // namespace

Face& dashboard() {
  static Dashboard f;
  return f;
}

}  // namespace faces
