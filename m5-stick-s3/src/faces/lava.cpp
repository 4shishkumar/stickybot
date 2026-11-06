// Lava: a lava lamp glowing on a dark desk. Wax blobs warm up at the bottom, rise, cool at
// the top and sink again, merging and splitting on the way (metaballs, worked out on a 2 px
// grid so it stays cheap).
//   - a blob per running Claude Code session on top of three of its own
//   - the colour says how things are: purple and pink when all is calm, amber while Claude
//     works, red when CI failed, orange (pulsing) when something waits for you
//   - a merged PR sends a big blob up from the bottom; shake it and the wax swirls; tilt it
//     and the blobs lean to the low side
// The time, battery / Bluetooth and the date are always at the top (common.h headline()),
// anything urgent as a card over the lamp's lower half.
#include "faces.h"
#include "common.h"

namespace faces {

using namespace ui;
using world::Kick;

namespace {

struct Layout {
  int cx, top, bottom;   // the glass: centre line, from the cap's bottom to the base's top
  int wTop, wBottom;     // its half-widths at the top and the bottom
};
constexpr Layout PORTRAIT{67, 94, 192, 20, 34};
constexpr Layout WIDE{200, 34, 106, 14, 25};

struct Palette {
  uint16_t hot, cool, glow;
};
static const Palette CALM{rgb(0xff, 0x5e, 0xa8), rgb(0x9b, 0x4d, 0xff), rgb(0x3a, 0x1a, 0x4a)};
static const Palette BUSY{rgb(0xff, 0xc1, 0x3a), rgb(0xff, 0x72, 0x2e), rgb(0x4a, 0x2a, 0x10)};
static const Palette FAIL{rgb(0xff, 0x52, 0x52), rgb(0xa8, 0x1e, 0x3a), rgb(0x4a, 0x10, 0x14)};
static const Palette WAIT{rgb(0xff, 0xb3, 0x3c), rgb(0xff, 0x7a, 0x1c), rgb(0x4a, 0x2a, 0x08)};

constexpr int MAX_BLOBS = 3 + world::MAX_SESSIONS + 1;

class Lava : public Face {
 public:
  const char* id() const override { return "lava"; }
  const char* title() const override { return "Lava Lamp"; }
  const char* about() const override {
    return "A lava lamp: a blob per Claude session, the colour says how things are; shake it to swirl";
  }

  void reset() override {
    burstAt_ = swirlAt_ = 0;
    last_ = 0;
    lean_ = 0;
    for (int i = 0; i < 3; i++) pal_[i] = 0;
  }

  uint32_t frameMs(bool onBattery, uint32_t idleMs) const override {
    if (!onBattery) return 50;
    return idleMs < 20000 ? 80 : 160;  // the wax is slow anyway
  }

  void kick(Kick k, uint32_t now) override {
    if (k == Kick::MERGED || k == Kick::RELEASE || k == Kick::LEVEL_UP || k == Kick::CLAUDE_DONE) burstAt_ = now | 1;
    if (k == Kick::SHAKE || k == Kick::FLICK || k == Kick::UPSIDE_DOWN) swirlAt_ = now | 1;
  }

  void draw(const Dash& d, bool wide, uint32_t now) override;

 private:
  uint32_t burstAt_ = 0, swirlAt_ = 0, last_ = 0;
  float    lean_ = 0;
  float    pal_[3] = {0, 0, 0};  // eased weights of BUSY / FAIL / WAIT over CALM

  float halfWidth(const Layout& L, int y) const {
    float t = constrain((float)(y - L.top) / (L.bottom - L.top), 0.0f, 1.0f);
    return L.wTop + (L.wBottom - L.wTop) * t;
  }
};

static uint16_t mix(const Palette& a, const Palette& b, float t, int which) {
  const uint16_t ca = which == 0 ? a.hot : which == 1 ? a.cool : a.glow;
  const uint16_t cb = which == 0 ? b.hot : which == 1 ? b.cool : b.glow;
  return blend(ca, cb, t);
}

void Lava::draw(const Dash& d, bool wide, uint32_t now) {
  const Layout& L = wide ? WIDE : PORTRAIT;
  const world::Snapshot& w = faces::world();
  const bool first = !last_;
  float dt = first ? 0 : constrain((now - last_) / 1000.0f, 0.0f, 0.25f);
  last_ = now;
  const Urgent u = urgent(d);

  // the mood, eased so the colour drifts rather than jumps
  const float want[3] = {d.agentLinked && w.nSessions ? 1.0f : 0.0f, w.ci == world::Ci::RED ? 1.0f : 0.0f,
                         u.on || (d.agentLinked && (w.asks > 0 || w.reviews > 0)) ? 1.0f : 0.0f};
  for (int i = 0; i < 3; i++) pal_[i] = first ? want[i] : pal_[i] + (want[i] - pal_[i]) * min(1.0f, dt * 1.5f);
  const float pulse = 0.75f + 0.25f * sinf(now / 500.0f);
  uint16_t hot = CALM.hot, cool = CALM.cool, glow = CALM.glow;
  const Palette* P[3] = {&BUSY, &FAIL, &WAIT};
  for (int i = 0; i < 3; i++) {
    const float t = pal_[i] * (i == 2 ? pulse : 1.0f);
    hot = blend(hot, P[i]->hot, t);
    cool = blend(cool, P[i]->cool, t);
    glow = blend(glow, P[i]->glow, t);
  }

  // the room: dark, the lamp's glow on the wall and the desk
  const uint16_t wall = rgb(0x0c, 0x0a, 0x12);
  canvas.fillScreen(wall);
  for (int r = 7; r >= 1; r--)
    canvas.fillEllipse(L.cx, (L.top + L.bottom) / 2, L.wBottom * 1.1f + r * (wide ? 9 : 10), (L.bottom - L.top) / 2 + r * 9,
                       blend(wall, glow, 0.9f - r * 0.11f));
  const int desk = L.bottom + (wide ? 16 : 18);
  canvas.fillRect(0, desk, W, H - desk, rgb(0x1a, 0x12, 0x10));
  canvas.fillEllipse(L.cx, desk + 3, L.wBottom + 18, 4, blend(rgb(0x1a, 0x12, 0x10), glow, 0.8f));

  // the wax: blobs on slow, different cycles, a session's one smaller and quicker
  float gx, gy;
  gravity(gx, gy);
  lean_ += (constrain(gx, -0.6f, 0.6f) - lean_) * min(1.0f, dt * 2);
  const float span = L.bottom - L.top;
  const float swirl = swirlAt_ && now - swirlAt_ < 2500 ? 1 - (now - swirlAt_) / 2500.0f : 0;
  struct B { float x, y, r2; } b[MAX_BLOBS];
  int nb = 0;
  const int sessions = d.agentLinked ? w.nSessions : 0;
  for (int i = 0; i < 3 + sessions && nb < MAX_BLOBS - 1; i++) {
    const bool own = i < 3;
    const float period = own ? 21000 + i * 6100 : 13000 + (w.sessions[i - 3].id % 7) * 1300;
    const float ph = fmodf(now / period + (own ? i * 0.31f : (w.sessions[i - 3].id % 100) / 100.0f), 1.0f);
    const float rise = 0.5f - 0.5f * cosf(ph * 2 * PI);  // 0 at the bottom, 1 at the top, eased at both
    const float r = (own ? (wide ? 9.0f : 12.0f) - i : (wide ? 5.5f : 7.0f)) * (1 + 0.25f * (1 - rise));  // warm = big
    const float y = L.bottom - 6 - rise * (span - 14);
    const float hw = halfWidth(L, (int)y) - r * 0.6f;
    float x = L.cx + sinf(now / (2600.0f + i * 700) + i * 1.7f) * hw * 0.55f + lean_ * hw * 0.8f;
    x += swirl * sinf(now / 120.0f + i) * hw;
    b[nb++] = {x, y, r * r};
  }
  if (burstAt_ && now - burstAt_ < 6000) {  // a big blob from the bottom
    const float p = (now - burstAt_) / 6000.0f;
    const float r = wide ? 10 : 14;
    b[nb++] = {(float)L.cx, L.bottom - 4 - smooth(min(1.0f, p * 1.4f)) * (span - 18), r * r};
  }
  // the glass: the liquid behind the wax, then the wax row by row in 2 px runs
  const uint16_t liquid = blend(rgb(0x1c, 0x10, 0x2a), glow, 0.5f);
  for (int y = L.top; y < L.bottom; y += 2) {
    const int hw = (int)halfWidth(L, y), x0 = L.cx - hw;
    canvas.fillRect(x0, y, 2 * hw, 2, blend(liquid, wall, 0.2f * (float)(y - L.top) / span));
    int runStart = -1;
    uint16_t runC = 0;
    for (int x = x0; x <= L.cx + hw; x += 2) {
      float f = 0;
      if (x < L.cx + hw)
        for (int i = 0; i < nb; i++) {
          const float dx = x - b[i].x, dy = y - b[i].y;
          f += b[i].r2 / (dx * dx + dy * dy + 1);
        }
      // the wax's colour: hotter (lighter) at the bottom and deep inside a blob
      uint16_t c = 0;
      if (f >= 1) {
        const float heat = constrain(0.25f + 0.55f * (float)(y - L.top) / span + 0.12f * min(f - 1, 2.0f), 0.0f, 1.0f);
        c = blend(cool, hot, heat);
        if (f < 1.25f) c = blend(c, liquid, 0.35f);  // a soft edge
      }
      if (c != runC || x >= L.cx + hw) {
        if (runC && runStart >= 0) canvas.fillRect(runStart, y, x - runStart, 2, runC);
        runStart = x;
        runC = c;
      }
    }
  }
  // the glass's shine, the cap and the base (metal)
  const uint16_t metal = rgb(0x6a, 0x70, 0x7a), metalDark = rgb(0x3a, 0x3e, 0x46), shine = rgb(0x9a, 0xa2, 0xae);
  for (int y = L.top + 6; y < L.bottom - 8; y += 2) {
    const int hw = (int)halfWidth(L, y);
    canvas.fillRect(L.cx - hw + 3, y, 2, 2, blend(liquid, INK, 0.28f));
  }
  // a trapezoid (half-widths at its top and bottom), lit on the left, in shade on the right
  auto trap = [&](int y0, float h0, int y1, float h1) {
    canvas.fillTriangle(L.cx - h0, y0, L.cx, y0, L.cx - h1, y1, metal);
    canvas.fillTriangle(L.cx, y0, L.cx - h1, y1, L.cx, y1, metal);
    canvas.fillTriangle(L.cx, y0, L.cx + h0, y0, L.cx, y1, metalDark);
    canvas.fillTriangle(L.cx + h0, y0, L.cx, y1, L.cx + h1, y1, metalDark);
  };
  const int capH = wide ? 12 : 16, waist = L.bottom + (desk - L.bottom) * 4 / 10;
  trap(L.top - capH, L.wTop * 0.55f, L.top, L.wTop + 2);                        // the cap
  canvas.fillRect(L.cx - 2, L.top - capH - 2, 4, 2, metalDark);
  trap(L.bottom, L.wBottom + 1, waist, L.wBottom * 0.72f);                      // the base, in at the waist
  trap(waist, L.wBottom * 0.72f, desk, L.wBottom + 6);
  canvas.drawFastHLine(L.cx - L.wBottom - 1, L.bottom, 2 * L.wBottom + 2, shine);
  canvas.drawFastHLine(L.cx - L.wTop - 2, L.top - 1, 2 * L.wTop + 4, shine);

  // what it means, in a word under (or beside) the lamp
  String mood = !d.agentLinked ? "" : w.ci == world::Ci::RED ? "CI failed"
              : w.asks ? String(w.asks) + (w.asks == 1 ? " needs you" : " need you")
              : w.reviews ? String(w.reviews) + (w.reviews == 1 ? " review" : " reviews")
              : sessions ? String(sessions) + (sessions == 1 ? " session" : " sessions") : "";
  if (u.on) {
    if (wide) signBoard(6, 74, 156, 46, u.a, u.b, u.bg, u.ink);
    else signBoard(6, 150, W - 12, 50, u.a, u.b, u.bg, u.ink);
  } else if (mood.length()) {
    if (wide) shadowTxt(mood, 6, 86, F14, blend(INK, hot, 0.35f), wall);
    else shadowTxt(mood, W / 2, desk + 9, F12, blend(INK, hot, 0.35f), wall, middle_center);
  }
  headline(d, wide, INK, wall, false);  // landscape: the date under the clock (the lamp is on the right)
}

}  // namespace

Face& lava() {
  static Lava f;
  return f;
}

}  // namespace faces
