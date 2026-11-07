// Big Time: the time and nothing else, as big as the screen allows (hours over minutes in
// portrait, one line across in landscape), the date small under it and the battery as a
// hairline at the bottom. Anything urgent comes up as a card under the time (the time
// shrinks to make room in landscape). Static: drawn when something changes.
#include "faces.h"
#include "common.h"

namespace faces {

using namespace ui;

namespace {

// The battery as a thin line along the bottom (green while charging, red when low).
static void batteryLine(const Dash& d, int y) {
  if (d.battery < 0) return;
  const int x = 24, w = W - 48;
  const uint16_t c = d.charging ? ui::OK : d.battery <= 15 ? ERR : blend(GROUND, INK, 0.45f);
  canvas.fillRoundRect(x, y, w, 2, 1, blend(GROUND, INK, 0.1f));
  canvas.fillRoundRect(x, y, max(2, w * d.battery / 100), 2, 1, c);
}

class BigTime : public Face {
 public:
  const char* id() const override { return "bigtime"; }
  const char* title() const override { return "Big Time"; }
  const char* about() const override { return "Just the time, as big as it gets, and the date"; }

  void draw(const Dash& d, bool wide, uint32_t) override {
    for (int y = 0; y < H; y += 4) canvas.fillRect(0, y, W, 4, blend(rgb(0x14, 0x1c, 0x22), GROUND, (float)y / H));
    const Urgent u = urgent(d);
    const int colon = d.clock.indexOf(':');
    String date = d.date;
    if (!d.agentLinked) date = "agent offline";
    if (wide) {
      if (!d.clock.length()) {
        txt(d.name, W / 2, u.on ? 30 : 56, fit(d.name, W - 12, {F24, F18, F12}), INK, middle_center);
      } else if (u.on) {  // the card needs the room: the time a size down
        txt(d.clock, W / 2 - 10, 30, F56, INK, middle_center);
        txt(d.ampm, W / 2 - 10 + textW(d.clock, F56) / 2 + 4, 18, F12, DIM);
      } else {
        txt(d.clock, W / 2, 54, fit(d.clock, W - 12, {F96, F56}), INK, middle_center);
        txt(d.ampm, W - 8, 10, F12, DIM, top_right);
      }
      if (u.on) signBoard(10, 62, W - 20, 50, u.a, u.b, u.bg, u.ink);
      else txt(ellipsize(date, F14, W - 16), W / 2, 106, F14, DIM, middle_center);
      batteryLine(d, H - 3);
      return;
    }
    if (d.clock.length() && colon > 0) {
      txt(d.clock.substring(0, colon), W / 2, 62, F96, INK, middle_center);
      txt(d.clock.substring(colon + 1), W / 2, 150, F96, blend(INK, AGENT, 0.55f), middle_center);
      if (!u.on) txt(ellipsize(d.ampm.length() ? d.ampm + "  " + date : date, F14, W - 12), W / 2, 206, F14, DIM, middle_center);
    } else {
      txt(d.name, W / 2, 100, fit(d.name, W - 12, {F24, F18, F12}), INK, middle_center);
      txt("time comes from the Mac agent", W / 2, 128, F9, FAINT, middle_center);
    }
    if (u.on) signBoard(6, 186, W - 12, 42, u.a, u.b, u.bg, u.ink);
    batteryLine(d, H - 2);
  }
  // frameMs 0: drawn when something changes (the minute, the battery, a card)

 private:
  static int textW(const String& s, const lgfx::IFont* f) {
    canvas.setFont(f);
    return canvas.textWidth(s);
  }
};

}  // namespace

Face& bigTimeFace() {
  static BigTime f;
  return f;
}

}  // namespace faces
