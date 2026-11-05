// Aquarium: a tank on your desk.
//   - each running Claude Code session is a fish (bigger the longer it runs, coloured by
//     project); when the session finishes its fish leaps out with a splash; two ambient
//     fish keep it company (logic/school.cpp)
//   - PR events float up as bubbles (a merge is a big one that pops with a sparkle, a
//     release a golden one); a review request brings a crab holding a tiny sign; failed CI
//     turns the water murky until it's fixed
//   - tilt the stick: the water surface stays level and the fish drift to the low side
//     (logic/spring.cpp); shake it: a burst of bubbles and everyone darts off
//   - night: dim water, the fish sleep near the bottom
// The time, battery / Bluetooth and anything urgent (a floating banner) are always on screen.
#include "faces.h"
#include "common.h"
#include "../logic/school.h"
#include "../logic/sky.h"
#include "../logic/spring.h"

namespace faces {

using namespace ui;
using world::Kick;

namespace {

struct Layout {
  int surface;  // where the water starts (level)
  int sand;     // where it ends
};
constexpr Layout PORTRAIT{32, 214};
constexpr Layout WIDE{26, 114};

static const uint16_t FISH[school::HUES] = {
    rgb(0xff, 0x8c, 0x42), rgb(0xff, 0x6b, 0x9a), rgb(0xff, 0xd1, 0x4a), rgb(0x9b, 0xe5, 0x64),
    rgb(0xb3, 0x8c, 0xff), rgb(0xff, 0x5e, 0x5e), rgb(0x4f, 0xd8, 0xe8), rgb(0xf2, 0xa6, 0x3b)};

struct Bubble {
  bool     on = false;
  float    x = 0, y = 0, r = 1, vy = 20;
  uint8_t  kind = 0;  // 0 plain, 1 a PR (big, pops with a sparkle), 2 a release (gold)
  uint32_t popAt = 0; // popped at the surface: the sparkle
};

class Aquarium : public Face {
 public:
  const char* id() const override { return "aquarium"; }
  const char* title() const override { return "Aquarium"; }
  const char* about() const override {
    return "A fish per running Claude session, PRs as bubbles; tilt the stick and the water follows";
  }

  void reset() override {
    began_ = false;
    for (auto& b : bubbles_) b = Bubble{};
    murk_ = 0;
    tilt_ = spring::Spring{};
    last_ = 0;
  }

  uint32_t frameMs(bool onBattery, uint32_t idleMs) const override {
    if (!onBattery) return 40;
    return idleMs < 20000 ? 66 : 150;
  }

  void kick(Kick k, uint32_t now) override {
    if (!began_) { pending_[npending_ < 4 ? npending_++ : 3] = k; return; }  // first draw plays them
    apply(k, now);
  }

  void draw(const Dash& d, bool wide, uint32_t now) override;

 private:
  school::School school_;
  spring::Spring tilt_;
  Bubble   bubbles_[28];
  bool     began_ = false;
  int      bw_ = 0, bh_ = 0;  // the water box the school knows
  float    murk_ = 0, crabX_ = 30;
  int      crabDir_ = 1;
  uint32_t last_ = 0, lastBubble_ = 0, wokeAt_ = 0;
  Kick     pending_[4];
  int      npending_ = 0;

  void apply(Kick k, uint32_t now);
  void bubble(float x, float y, float r, uint8_t kind);
  void fish(const school::Fish& f, float ox, float oy, bool sleepy, uint32_t now);
  void crab(float x, int y, int reviews, uint32_t now, bool night);
  void plants(const Layout& L, uint32_t now, bool night);
};

void Aquarium::bubble(float x, float y, float r, uint8_t kind) {
  for (auto& b : bubbles_) {
    if (b.on || b.popAt) continue;
    b.on = true;
    b.x = x;
    b.y = y;
    b.r = r < 2 ? 2 : r;
    b.vy = kind ? 16 : 22 + frac01((uint32_t)(x * 31 + y)) * 14;
    b.kind = kind;
    return;
  }
}

void Aquarium::apply(Kick k, uint32_t now) {
  const Layout& L = W > H ? WIDE : PORTRAIT;
  const float bottom = L.sand - 4;
  switch (k) {
    case Kick::MERGED:
      bubble(W * 0.5f, bottom, 9, 1);
      for (int i = 0; i < 4; i++) bubble(W * 0.5f + (i - 2) * 7, bottom + 6 + i * 5, 2, 0);
      break;
    case Kick::RELEASE:
    case Kick::LEVEL_UP:
      bubble(W * 0.5f, bottom, 11, 2);
      for (int i = 0; i < 10; i++) bubble(W * frac01(now + i), bottom + i * 4, 1.5f + (i % 3), 0);
      break;
    case Kick::CI_GREEN:
    case Kick::CLAUDE_DONE:
    case Kick::CLAUDE_START:
      for (int i = 0; i < 7; i++) bubble(W * (0.2f + 0.6f * frac01(now + i * 3)), bottom + i * 6, 1.5f + (i % 2), 0);
      break;
    case Kick::SHAKE:
      school_.scatter();
      tilt_.v += 3;
      for (int i = 0; i < 18; i++)
        bubble(W * frac01(now + i * 11), L.surface + 20 + (L.sand - L.surface - 30) * frac01(now + i * 13), 1.5f + (i % 3), 0);
      break;
    case Kick::FLICK:
    case Kick::UPSIDE_DOWN:
      tilt_.v += k == Kick::FLICK ? 2.5f : -4;  // a slosh
      school_.scatter();
      break;
    case Kick::LIFT:
      tilt_.v += 1.2f;
      for (int i = 0; i < 5; i++) bubble(W * (0.3f + 0.1f * i), bottom, 2, 0);
      break;
    default:
      break;
  }
}

void Aquarium::plants(const Layout& L, uint32_t now, bool night) {
  const uint16_t weed = night ? rgb(0x1d, 0x4a, 0x33) : rgb(0x3f, 0xa3, 0x5c);
  const uint16_t weed2 = blend(weed, TFT_BLACK, 0.25f);
  const int xs[] = {W / 9, W / 3 + 4, W - W / 5, W - 10};
  for (int s = 0; s < 4; s++) {  // seaweed, swaying
    int n = (W > H ? 12 : 17) - s % 2 * 4;
    for (int j = 0; j < n; j++) {
      float px = xs[s] + sinf(now / 900.0f + j * 0.45f + s) * j * 0.55f;
      canvas.fillCircle(px, L.sand - j * 3, max(2.0f, 3.0f - j * 0.08f), s % 2 ? weed2 : weed);
    }
  }
  // coral
  const uint16_t coral = night ? rgb(0x5a, 0x2a, 0x3a) : rgb(0xff, 0x7f, 0x8e);
  const uint16_t coral2 = night ? rgb(0x5a, 0x3a, 0x22) : rgb(0xff, 0xa9, 0x4d);
  int cx = W / 2 + 16;
  canvas.fillRoundRect(cx - 2, L.sand - 14, 4, 14, 2, coral);
  canvas.fillRoundRect(cx - 8, L.sand - 10, 4, 8, 2, coral);
  canvas.fillRoundRect(cx + 4, L.sand - 12, 4, 9, 2, coral);
  canvas.fillRect(cx - 6, L.sand - 4, 12, 3, coral);
  int cx2 = W / 5 + 10;
  canvas.fillCircle(cx2, L.sand - 3, 5, coral2);
  canvas.fillCircle(cx2 + 5, L.sand - 5, 3, coral2);
  // rocks
  const uint16_t rock = night ? rgb(0x2a, 0x2e, 0x36) : rgb(0x7a, 0x80, 0x8a);
  canvas.fillEllipse(W - W / 3, L.sand, 9, 5, rock);
  canvas.fillEllipse(8, L.sand + 1, 7, 4, blend(rock, TFT_BLACK, 0.2f));
}

void Aquarium::crab(float x, int y, int reviews, uint32_t now, bool night) {
  const uint16_t shell = night ? rgb(0x7a, 0x2e, 0x24) : rgb(0xe8, 0x55, 0x3c);
  const uint16_t dark = blend(shell, TFT_BLACK, 0.35f);
  float step = sinf(now / 110.0f);
  for (int i = 0; i < 3; i++) {  // legs
    canvas.drawLine(x - 4 - i * 2, y, x - 8 - i * 2, y + 4 + (i % 2 ? step : -step), dark);
    canvas.drawLine(x + 4 + i * 2, y, x + 8 + i * 2, y + 4 + (i % 2 ? -step : step), dark);
  }
  canvas.fillEllipse(x, y, 8, 5, shell);
  canvas.fillRect(x - 3, y - 8, 1, 4, dark);  // eyes on stalks
  canvas.fillRect(x + 3, y - 8, 1, 4, dark);
  canvas.fillCircle(x - 3, y - 9, 1.5f, INK);
  canvas.fillCircle(x + 3, y - 9, 1.5f, INK);
  // the claws hold up a tiny sign
  canvas.fillCircle(x - 9, y - 6, 2.5f, shell);
  canvas.fillCircle(x + 9, y - 6, 2.5f, shell);
  String s = reviews == 1 ? "review" : String(reviews) + " reviews";
  canvas.setFont(F9);
  int w = canvas.textWidth(s) + 8, top = y - 26;
  canvas.drawFastVLine(x - 9, top + 12, y - 8 - top - 12, rgb(0x8a, 0x62, 0x3c));
  canvas.drawFastVLine(x + 9, top + 12, y - 8 - top - 12, rgb(0x8a, 0x62, 0x3c));
  int sx = constrain((int)x - w / 2, 2, W - w - 2);
  canvas.fillRoundRect(sx, top, w, 13, 3, rgb(0xf4, 0xf6, 0xf8));
  txt(s, sx + w / 2, top + 6, F9, rgb(0x0b, 0x2a, 0x55), middle_center);
}

void Aquarium::fish(const school::Fish& f, float ox, float oy, bool sleepy, uint32_t now) {
  const float s = f.size, x = ox + f.x, y = oy + f.y, L = 10 * s, hh = 5.6f * s;
  const int dir = f.facingRight() ? 1 : -1;
  uint16_t col = f.ambient ? (f.hue ? rgb(0xc9, 0xc2, 0x9a) : rgb(0x9c, 0xc4, 0xd4)) : FISH[f.hue % school::HUES];
  if (sleepy) col = blend(col, rgb(0x0a, 0x1a, 0x2c), 0.45f);
  const uint16_t darker = blend(col, TFT_BLACK, 0.3f);
  const float wig = sinf(now / (sleepy ? 400.0f : 110.0f) + f.id) * 1.6f * s;
  // tail, fin, body, belly, eye
  canvas.fillTriangle(x - dir * (L - 2), y, x - dir * (L + 6 * s), y - 4 * s - wig, x - dir * (L + 6 * s), y + 4 * s + wig, darker);
  canvas.fillTriangle(x - dir * 2 * s, y - hh + 1, x + dir * 3 * s, y - hh + 1, x - dir * 4 * s, y - hh - 3 * s, darker);
  canvas.fillEllipse(x, y, L, hh, col);
  canvas.fillEllipse(x + dir * s, y + hh * 0.4f, L * 0.6f, hh * 0.35f, blend(col, TFT_WHITE, 0.3f));
  if (f.ambient) canvas.fillRect(x - 1.5f * s, y - hh + 1, 3 * s, 2 * hh - 2, darker);  // a stripe
  const float ex = x + dir * L * 0.55f, ey = y - hh * 0.25f;
  if (sleepy) {
    canvas.drawFastHLine(ex - 1.5f * s, ey, 3 * s, PUPIL);
  } else {
    canvas.fillCircle(ex, ey, max(1.5f, 1.9f * s), TFT_WHITE);
    canvas.fillCircle(ex + dir * 0.5f, ey, max(1.0f, 1.0f * s), PUPIL);
  }
}

void Aquarium::draw(const Dash& d, bool wide, uint32_t now) {
  const Layout& L = wide ? WIDE : PORTRAIT;
  const world::Snapshot& w = faces::world();
  const int minute = d.minute >= 0 ? d.minute : 12 * 60;
  const sky::Colors sc = sky::colors(minute, false);
  const bool night = isNightMinute(d.minute) || sc.light < 0.15f;
  const float light = night ? 0.0f : sc.light;
  const int waterH = L.sand - L.surface;
  if (!began_ || bw_ != W || bh_ != waterH) {
    if (!began_) {
      school_.begin(now | 1, W, waterH, now);
      wokeAt_ = now;
    } else {
      school_.resize(W, waterH);
    }
    began_ = true;
    bw_ = W;
    bh_ = waterH;
    for (int i = 0; i < npending_; i++) apply(pending_[i], now);
    npending_ = 0;
  }
  if (last_ && now - last_ > 5000) wokeAt_ = now;  // back after the screen was off: names again
  float dt = last_ ? constrain((now - last_) / 1000.0f, 0.0f, 0.25f) : 0;
  last_ = now;

  // the fish follow the sessions (kept while the agent is away: unknown isn't "finished")
  if (d.agentLinked) {
    school::Session ss[world::MAX_SESSIONS];
    for (int i = 0; i < w.nSessions; i++) ss[i] = {w.sessions[i].id, w.sessions[i].project, w.sessions[i].secs};
    school_.sync(ss, w.nSessions, now);
  }
  // tilt: the surface stays level with the world, the fish drift to the low side
  float gx, gy;
  gravity(gx, gy);
  float target = constrain(gx, -0.7f, 0.7f);
  tilt_.step(target, dt);
  school_.step(dt, tilt_.x * 1.2f, night, now);
  murk_ += ((w.ci == world::Ci::RED ? 1.0f : 0.0f) - murk_) * min(1.0f, dt * 0.8f);

  // water
  uint16_t wTop = blend(rgb(0x0e, 0x2a, 0x44), rgb(0x2a, 0x86, 0xc0), light);
  uint16_t wLow = blend(rgb(0x04, 0x10, 0x1e), rgb(0x0c, 0x3a, 0x5e), light);
  const uint16_t MURK = rgb(0x4a, 0x5a, 0x24);
  wTop = blend(wTop, MURK, 0.55f * murk_);
  wLow = blend(wLow, blend(MURK, TFT_BLACK, 0.4f), 0.55f * murk_);
  const float slope = tilt_.x * 0.16f;  // px per px: the water piles up on the low side
  const int top = L.surface - 14;
  for (int y = top; y < L.sand; y += 3) {
    float k = (float)(y - L.surface) / waterH;
    uint16_t band = blend(wTop, wLow, constrain(k, 0.0f, 1.0f));
    canvas.fillRect(0, y, W, 3, band);
    if (light > 0.3f && murk_ < 0.6f && y > L.surface) {  // light rays, slanting down
      for (int r = 0; r < 3; r++) {
        float rx = W * (0.15f + 0.32f * r) + sinf(now / 3200.0f + r * 2) * 8 + (y - L.surface) * 0.35f;
        float rw = 8 + (y - L.surface) * 0.1f;
        canvas.fillRect((int)rx, y, (int)rw, 3, blend(band, TFT_WHITE, 0.07f * light * (1 - k)));
      }
    }
  }
  // the air above the surface, and the surface itself (level with the world: it tilts on screen)
  const uint16_t air = blend(rgb(sc.top.r, sc.top.g, sc.top.b), GROUND, 0.35f);
  const uint16_t foam = blend(wTop, TFT_WHITE, 0.35f);
  for (int x = 0; x < W; x += 2) {
    int sy = L.surface + (int)(slope * (x - W / 2) + 1.6f * sinf(x / 11.0f + now / 420.0f));
    sy = constrain(sy, top + 2, L.surface + 14);
    canvas.fillRect(x, 0, 2, sy, air);
    canvas.fillRect(x, sy, 2, 1, foam);
  }
  // sand
  const uint16_t sand = night ? rgb(0x3a, 0x33, 0x24) : rgb(0xc9, 0xad, 0x74);
  canvas.fillRect(0, L.sand, W, H - L.sand, sand);
  for (int x = 0; x < W; x += 4) canvas.fillRect(x, L.sand - (hash32(x + 3) % 2), 4, 2, sand);
  for (int i = 0; i < 18; i++)
    canvas.fillRect(hash32(i + 51) % W, L.sand + 3 + hash32(i + 77) % max(3, H - L.sand - 6), 2, 1, blend(sand, TFT_BLACK, 0.2f));
  plants(L, now, night);

  // the bubbler: a shell on the sand
  const int bx = W / 2 - 22;
  canvas.fillEllipse(bx, L.sand - 1, 6, 4, night ? rgb(0x6a, 0x5a, 0x60) : rgb(0xf2, 0xd4, 0xdc));
  if (now - lastBubble_ > (night ? 2600u : 900u)) {
    lastBubble_ = now;
    bubble(bx + (hash32(now) % 5) - 2, L.sand - 5, 1.5f + (hash32(now + 1) % 2), 0);
  }

  // review requests: the crab with its sign
  if (d.agentLinked && w.reviews > 0) {
    crabX_ += crabDir_ * 6 * dt;
    if (crabX_ > W - 20) crabDir_ = -1;
    if (crabX_ < 20) crabDir_ = 1;
    crab(crabX_, L.sand + (wide ? 6 : 8), w.reviews, now, night);
  }

  // bubbles rise; the big ones pop with a sparkle
  for (auto& b : bubbles_) {
    if (b.popAt) {
      uint32_t e = now - b.popAt;
      if (e > 450) { b.popAt = 0; continue; }
      float p = e / 450.0f, r = 4 + p * 10;
      uint16_t sp = b.kind == 2 ? SUN : INK;
      for (int i = 0; i < 6; i++) {
        float a = i * PI / 3 + p;
        canvas.fillRect(b.x + cosf(a) * r, b.y + sinf(a) * r * 0.7f, 2, 2, blend(sp, wTop, p));
      }
      continue;
    }
    if (!b.on) continue;
    b.y -= b.vy * dt;
    b.x += sinf(now / 300.0f + b.r * 3) * 6 * dt + tilt_.x * -8 * dt;  // up the slope
    int sy = L.surface + (int)(slope * (b.x - W / 2));
    if (b.y - b.r <= sy) {
      b.on = false;
      if (b.kind) { b.popAt = now ? now : 1; b.y = sy; }
      continue;
    }
    uint16_t rim = b.kind == 2 ? SUN : blend(wTop, TFT_WHITE, 0.55f);
    canvas.drawCircle(b.x, b.y, b.r, rim);
    canvas.fillRect(b.x - b.r * 0.4f, b.y - b.r * 0.5f, max(1.0f, b.r * 0.35f), max(1.0f, b.r * 0.35f), rim);
    if (b.kind == 1 && b.r >= 8) txt("PR", b.x, b.y + 1, F9, blend(wTop, TFT_WHITE, 0.8f), middle_center);
  }

  // the fish (names for a few seconds after they arrive, and when the screen wakes)
  const bool names = now - wokeAt_ < 5000;
  for (int i = 0; i < school_.size(); i++) {
    const school::Fish& f = school_.at(i);
    if (!f.on) continue;
    fish(f, 0, L.surface, night && f.st == school::State::SWIM, now);
    if (f.st == school::State::JUMP) {  // splashes as it leaves and comes back
      float p = school::School::jumpPhase(f, now);
      if ((p >= 0 && p < 0.25f) || p > 0.8f) {
        float sx = p > 0.8f ? f.x : f.jumpX, k = p > 0.8f ? (p - 0.8f) / 0.2f : p / 0.25f;
        for (int j = 0; j < 5; j++) {
          float a = PI * (0.15f + 0.7f * j / 4.0f);
          canvas.fillRect(sx - cosf(a) * 10 * k, L.surface - sinf(a) * 9 * k + 6 * k * k, 2, 2, foam);
        }
      }
    }
  }
  for (int i = 0; i < school_.size(); i++) {  // the names on top of every fish
    const school::Fish& f = school_.at(i);
    if (!f.on || f.ambient || !f.label[0] || f.st != school::State::SWIM || !(names || now - f.born < 6000)) continue;
    String l(f.label);
    const float y = L.surface + f.y + 5.6f * f.size + 7;
    txt(l, f.x + 1, y + 1, F9, rgb(0x02, 0x0c, 0x18), middle_center);
    txt(l, f.x, y, F9, INK, middle_center);
  }
  if (night)  // z z z from the sleepers
    for (int i = 0; i < school_.size(); i++) {
      const school::Fish& f = school_.at(i);
      if (!f.on || f.ambient) continue;
      float ph = fmodf(now / 1600.0f + i * 0.3f, 1.0f);
      txt("z", f.x + 6, L.surface + f.y - 6 - ph * 14, F9, blend(DIM, wLow, ph), middle_center);
    }

  // anything urgent floats on the water
  Urgent u = urgent(d);
  if (u.on) {
    int bob = (int)(sinf(now / 600.0f) * 1.5f);
    if (wide) signBoard(8, L.surface + 6 + bob, W - 16, 42, u.a, u.b, u.bg, u.ink);
    else signBoard(6, L.surface + 8 + bob, W - 12, 50, u.a, u.b, u.bg, u.ink);
  }

  // always: the time and battery / Bluetooth in the air strip
  const uint16_t shade = rgb(0x02, 0x08, 0x12);
  int tw = timeText(d, 6, wide ? 12 : 15, F18, INK, shade);
  if (wide && d.wxValid) shadowTxt(String(d.wxTemp) + "°", 6 + tw + 8, 12, F12, DIM, shade);
  statusIcons(d, W - 4, wide ? 11 : 9, INK);
  String n = quietNotice(d);
  if (n.length()) shadowTxt(wide ? String("agent offline") : n, 6, L.sand + (wide ? 8 : 10), F9, INK, shade);
  else if (wide && w.nSessions && d.agentLinked)
    shadowTxt(String(w.nSessions) + (w.nSessions == 1 ? " session" : " sessions"), 6, L.sand + 8, F9, INK, shade);
}

}  // namespace

Face& aquarium() {
  static Aquarium f;
  return f;
}

}  // namespace faces
