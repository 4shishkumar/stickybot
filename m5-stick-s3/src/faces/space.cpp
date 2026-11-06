// Space: StickyBot floats in a space bubble with a little jetpack over a ringed planet.
//   - stars drift past in three layers and twinkle; now and then a shooting star
//   - each running Claude Code session is a satellite orbiting the planet (coloured by
//     project, the same hues as the Aquarium's fish); it goes behind the planet and back
//   - a merged PR launches a rocket from the planet (a release: a golden one and confetti);
//     CI green or Claude done: a loop-the-loop; CI failed: the antenna light blinks red
//   - something waiting for you: a sign floats by the bot (review requests: a blue one);
//     the agent's card in a speech bubble now and then, like StickyWorld
//   - toys: shake = warp speed (the stars streak), flick = a loop, upside down = a wobble,
//     lift = it waves hi
// The time, battery / Bluetooth and the date are always at the top (common.h headline()).
#include "faces.h"
#include "common.h"

namespace faces {

using namespace ui;
using world::Kick;

namespace {

constexpr uint16_t SHELL     = rgb(0xe9, 0xec, 0xef);
constexpr uint16_t SHELL_RIM = rgb(0x4a, 0x52, 0x5c);
constexpr uint16_t SCREEN    = rgb(0x12, 0x15, 0x19);
constexpr uint16_t GOLD      = rgb(0xf2, 0xc1, 0x4e);

static const uint16_t SAT[8] = {rgb(0xff, 0x8c, 0x42), rgb(0xff, 0x6b, 0x9a), rgb(0xff, 0xd1, 0x4a),
                                rgb(0x9b, 0xe5, 0x64), rgb(0xb3, 0x8c, 0xff), rgb(0xff, 0x5e, 0x5e),
                                rgb(0x4f, 0xd8, 0xe8), rgb(0xf2, 0xa6, 0x3b)};

struct Layout {
  int   planetX, planetY, planetR;
  int   botX, botY;    // where the bot hovers
  int   asideX, asideY;  // where it moves to make room for a sign or a speech bubble
  int   signX, signY, signW, signH;
  float k;             // bot scale
};
// Portrait: the planet at the bottom, the bot above it, a sign under the clock. Landscape:
// the planet bottom right, a sign on the left with the bot beside it.
constexpr Layout PORTRAIT{67, 202, 28, 67, 128, 67, 150, 6, 78, 123, 42, 1.0f};
constexpr Layout WIDE{194, 108, 22, 104, 88, 158, 78, 6, 56, 128, 42, 0.9f};

enum class Act : uint8_t { NONE, ROCKET, GOLD_ROCKET, LOOP, WOBBLE, WARP, HI };

static uint32_t actMs(Act a) {
  switch (a) {
    case Act::ROCKET: return 3200;
    case Act::GOLD_ROCKET: return 4200;
    case Act::LOOP: return 1500;
    case Act::WOBBLE: return 1800;
    case Act::WARP: return 1800;
    case Act::HI: return 1800;
    default: return 0;
  }
}

class Space : public Face {
 public:
  const char* id() const override { return "space"; }
  const char* title() const override { return "Space"; }
  const char* about() const override {
    return "StickyBot floats over a ringed planet; Claude sessions orbit it, merged PRs launch rockets";
  }

  void reset() override {
    act_ = Act::NONE;
    pend_ = Act::NONE;
    cardA_ = "";
    cardAt_ = 0;
    homeX_ = homeY_ = -1;
    last_ = 0;
  }

  uint32_t frameMs(bool onBattery, uint32_t idleMs) const override {
    if (!onBattery) return 40;
    return idleMs < 20000 ? 66 : 150;
  }

  void kick(Kick k, uint32_t now) override {
    Act a;
    switch (k) {
      case Kick::MERGED: a = Act::ROCKET; break;
      case Kick::RELEASE:
      case Kick::LEVEL_UP: a = Act::GOLD_ROCKET; break;
      case Kick::CI_GREEN:
      case Kick::CLAUDE_DONE:
      case Kick::FLICK: a = Act::LOOP; break;
      case Kick::CI_RED:
      case Kick::UPSIDE_DOWN: a = Act::WOBBLE; break;
      case Kick::SHAKE: a = Act::WARP; break;
      case Kick::LIFT:
      case Kick::REVIEW: a = Act::HI; break;
      default: return;
    }
    pend_ = a;  // played from the next draw (the latest wins)
    pendAt_ = now;
    why_ = k;
  }

  void draw(const Dash& d, bool wide, uint32_t now) override;

 private:
  Act      act_ = Act::NONE, pend_ = Act::NONE;
  Kick     why_ = Kick::NONE;
  uint32_t actAt_ = 0, pendAt_ = 0, last_ = 0;
  float    homeX_ = -1, homeY_ = -1;  // where it hovers now (it glides aside for a sign)
  String   cardA_;
  uint32_t cardAt_ = 0;

  void stars(const Layout& L, uint32_t now, float warp);
  void planet(const Layout& L, bool front);
  void satellites(const Layout& L, const world::Snapshot& w, bool front, uint32_t now);
  void rocket(const Layout& L, float p, bool gold);
  void bot(float cx, float cy, float k, ui::Mood m, bool redLight, bool waving, uint32_t now);
};

void Space::stars(const Layout& L, uint32_t now, float warp) {
  // the sky: a deep blue fading to black at the top
  for (int y = 0; y < H; y += 4)
    canvas.fillRect(0, y, W, 4, blend(rgb(0x02, 0x03, 0x0a), rgb(0x0e, 0x12, 0x2e), (float)y / H));
  // a faint nebula, in soft steps
  for (int i = 0; i < 4; i++)
    canvas.fillEllipse(W * 0.3f, H * 0.5f, W * (0.42f - i * 0.08f), H * (0.16f - i * 0.03f),
                       blend(rgb(0x0b, 0x0e, 0x24), rgb(0x2a, 0x16, 0x4a), 0.2f + i * 0.14f));
  const int n = W > H ? 46 : 40;
  for (int i = 0; i < n; i++) {
    const int layer = i % 3;  // far, middle, near
    const float speed = (1.5f + layer * 2.5f) * (1 + warp * 30);
    const float x = fmodf(hash32(i * 13 + 1) % 1000 + W * 4 - now / 1000.0f * speed, (float)W);
    const int y = hash32(i * 13 + 2) % H;
    const float tw = 0.55f + 0.45f * sinf(now / 600.0f + i * 1.9f);
    const uint16_t c = blend(rgb(0x10, 0x14, 0x30), i % 7 == 0 ? rgb(0xff, 0xe3, 0xb0) : INK, (0.35f + 0.2f * layer) + 0.4f * tw);
    if (warp > 0.05f) canvas.drawFastHLine(x, y, 2 + (int)(warp * (6 + layer * 8)), c);
    else canvas.fillRect(x, y, layer == 2 ? 2 : 1, layer == 2 ? 2 : 1, c);
  }
  // a shooting star every 9 s
  const uint32_t ph = now % 9000;
  if (ph < 700) {
    const float p = ph / 700.0f;
    const int sx = 20 + hash32(now / 9000) % (W - 40), sy = 80 + hash32(now / 9000 + 1) % 40;
    const float x = sx + p * 60, y = sy - p * 22;
    for (int j = 0; j < 10; j++)
      canvas.fillRect(x - j * 3, y + j * 1.1f, 2, 1, blend(INK, rgb(0x0e, 0x12, 0x2e), j / 10.0f + p * 0.5f));
  }
}

// The planet in two halves of its ring: the far half behind it, the near half in front.
void Space::planet(const Layout& L, bool front) {
  const int cx = L.planetX, cy = L.planetY, r = L.planetR;
  const uint16_t ring = rgb(0xd9, 0xb8, 0x8a), ring2 = rgb(0x9c, 0x7e, 0x5a);
  const int ri = r + r / 3, ro = r + r * 3 / 4;  // the ring seen at a slant: wide, flat
  if (front) {
    canvas.fillEllipseArc(cx, cy, ri, ro, ri * 0.26f, ro * 0.3f, 0, 180, ring);
    return;
  }
  canvas.fillEllipseArc(cx, cy, ri, ro, ri * 0.26f, ro * 0.3f, 180, 360, ring2);
  canvas.fillCircle(cx, cy, r, rgb(0x5a, 0x4a, 0xb0));
  for (int y = -r + 1; y < r; y++) {  // bands, and the night side's shadow along the bottom right
    const int half = (int)sqrtf((float)(r * r - y * y));
    const int band = (y + r) * 7 / (2 * r);
    if (band % 2) canvas.drawFastHLine(cx - half, cy + y, 2 * half, rgb(0x6c, 0x5b, 0xc8));
    const int shade = (int)(half * 0.35f * (0.5f + 0.5f * y / r));
    if (shade > 0) canvas.drawFastHLine(cx + half - shade, cy + y, shade, rgb(0x34, 0x2a, 0x72));
  }
  canvas.fillCircle(cx - r * 0.38f, cy - r * 0.4f, max(2.0f, r * 0.16f), rgb(0x8a, 0x7a, 0xe0));  // a crater
  canvas.fillCircle(cx + r * 0.25f, cy - r * 0.62f, max(1.0f, r * 0.08f), rgb(0x46, 0x39, 0x8a));
}

// One satellite per Claude session on an orbit round the planet (tilted ellipse: the half
// with sin < 0 is behind it).
void Space::satellites(const Layout& L, const world::Snapshot& w, bool front, uint32_t now) {
  for (int i = 0; i < w.nSessions; i++) {
    const world::Session& s = w.sessions[i];
    const float orx = L.planetR * 1.5f + 4 + (i % 3) * 7, ory = orx * 0.3f;
    const float a = now / (5200.0f + i * 900) * 2 * PI + i * 2.1f;
    if ((sinf(a) >= 0) != front) continue;
    const float x = L.planetX + cosf(a) * orx, y = L.planetY - 3 + sinf(a) * ory;
    const uint16_t c = SAT[hash32(s.id) % 8];
    const uint16_t dim = front ? c : blend(c, rgb(0x0e, 0x12, 0x2e), 0.45f);
    canvas.fillRect(x - 2, y - 2, 4, 4, dim);                        // the body
    canvas.fillRect(x - 7, y - 1, 4, 2, blend(dim, ui::BLUE, 0.5f));      // the solar panels
    canvas.fillRect(x + 3, y - 1, 4, 2, blend(dim, ui::BLUE, 0.5f));
    if (front && (now / 500 + i) % 3 == 0) canvas.fillRect(x, y - 4, 1, 1, INK);  // a blink
  }
}

// The rocket from the planet's top, p 0..1 of its flight.
void Space::rocket(const Layout& L, float p, bool gold) {
  const float sx = L.planetX - L.planetR * 0.25f, sy = L.planetY - L.planetR - 2;
  const float y = sy - p * p * (sy + 40), x = sx + p * 18;
  const uint16_t body = gold ? GOLD : SHELL;
  // the smoke left behind
  for (int j = 1; j < 8; j++) {
    const float q = p - j * 0.03f;
    if (q < 0) break;
    const float yy = sy - q * q * (sy + 40), xx = sx + q * 18;
    canvas.fillCircle(xx, yy + 14, 2 + j * 0.6f, blend(rgb(0x8a, 0x90, 0x9c), rgb(0x0e, 0x12, 0x2e), j / 8.0f));
  }
  const float fl = 4 + 3 * frac01((uint32_t)(p * 1000));  // the flame flickers
  canvas.fillTriangle(x - 2, y + 9, x + 2, y + 9, x, y + 9 + fl + 3, ASK);
  canvas.fillTriangle(x - 1, y + 9, x + 1, y + 9, x, y + 9 + fl, SUN);
  canvas.fillTriangle(x - 5, y + 9, x - 2, y + 4, x - 2, y + 9, ERR);  // fins
  canvas.fillTriangle(x + 5, y + 9, x + 2, y + 4, x + 2, y + 9, ERR);
  canvas.fillRoundRect(x - 3, y - 4, 6, 14, 2, body);
  canvas.fillTriangle(x - 3, y - 3, x + 3, y - 3, x, y - 9, ERR);  // the nose
  canvas.fillCircle(x, y + 1, 1.5f, ui::BLUE);                         // the window
}

// The bot in its bubble helmet, a jetpack's flame under it.
void Space::bot(float cx, float cy, float k, ui::Mood m, bool redLight, bool waving, uint32_t now) {
  const float bw = 26 * k, bh = 30 * k, left = cx - bw / 2, top = cy - bh / 2;
  // the flame
  const float fl = (5 + 2.5f * sinf(now / 45.0f) + 1.5f * sinf(now / 23.0f)) * k;
  canvas.fillTriangle(cx - 5 * k, top + bh, cx + 5 * k, top + bh, cx, top + bh + fl + 3 * k, ASK);
  canvas.fillTriangle(cx - 2.5f * k, top + bh, cx + 2.5f * k, top + bh, cx, top + bh + fl, SUN);
  canvas.fillRoundRect(cx - 7 * k, top + bh - 3 * k, 14 * k, 5 * k, 2, rgb(0x5a, 0x62, 0x6c));  // the jetpack
  // the antenna and its light
  canvas.drawFastVLine(cx, top - 6 * k, 6 * k, SHELL_RIM);
  const bool blink = redLight ? (now / 300) % 2 : (now / 900) % 3 == 0;
  canvas.fillCircle(cx, top - 7 * k, 2 * k, redLight ? (blink ? ERR : rgb(0x5a, 0x10, 0x10)) : blink ? ui::OK : rgb(0x1a, 0x4a, 0x2a));
  // the shell, the screen, button A
  canvas.fillRoundRect(left, top, bw, bh, 7 * k, SHELL);
  canvas.drawRoundRect(left, top, bw, bh, 7 * k, SHELL_RIM);
  const float pw = bw - 6 * k, ph = bh * 0.58f, py = top + 3 * k;
  canvas.fillRoundRect(left + 3 * k, py, pw, ph, 4 * k, SCREEN);
  canvas.fillCircle(cx, top + bh * 0.8f, 2.4f * k, blend(SHELL, AGENT, 0.55f));
  eyes(m, cx, py + ph / 2, 0.18f * k, moodColor(m), SCREEN);
  // arms: waving, or resting on its sides
  const float wave = waving ? sinf(now / 90.0f) * 5 * k : 0;
  canvas.drawWideLine(left, top + bh * 0.55f, left - 5 * k, top + bh * 0.75f, 1.5f * k, SHELL_RIM);
  canvas.drawWideLine(left + bw, top + bh * 0.55f, left + bw + 5 * k, top + bh * (waving ? 0.1f : 0.75f) + wave, 1.5f * k,
                      SHELL_RIM);
  // the bubble helmet: a faint glass ring and a highlight
  const float hr = 24 * k;
  canvas.drawCircle(cx, cy - 2 * k, hr, blend(rgb(0x0e, 0x12, 0x2e), rgb(0x9f, 0xd8, 0xff), 0.45f));
  canvas.drawCircle(cx, cy - 2 * k, hr - 1, blend(rgb(0x0e, 0x12, 0x2e), rgb(0x9f, 0xd8, 0xff), 0.18f));
  canvas.fillArc(cx, cy - 2 * k, hr - 4, hr - 2, 200, 245, blend(rgb(0x0e, 0x12, 0x2e), INK, 0.55f));
}

void Space::draw(const Dash& d, bool wide, uint32_t now) {
  const Layout& L = wide ? WIDE : PORTRAIT;
  const world::Snapshot& w = faces::world();
  const float dt = last_ ? constrain((now - last_) / 1000.0f, 0.0f, 0.25f) : 0;
  last_ = now;
  if (homeX_ >= 0 && (homeX_ > W || homeY_ > H)) homeX_ = -1;  // turned: start where it belongs

  if (pend_ != Act::NONE) {  // a reaction from a kick (stale ones, from while the screen was off, dropped)
    if (now - pendAt_ < 5 * 60000UL) {
      act_ = pend_;
      actAt_ = now;
      if (act_ == Act::GOLD_ROCKET) confettiReset(L.botX, 40);
    }
    pend_ = Act::NONE;
  }
  if (act_ != Act::NONE && now - actAt_ >= actMs(act_)) act_ = Act::NONE;
  const float p = act_ != Act::NONE ? (float)(now - actAt_) / actMs(act_) : 0;

  const float warp = act_ == Act::WARP ? sinf(PI * p) : 0;
  stars(L, now, warp);
  const bool rocketUp = act_ == Act::ROCKET || act_ == Act::GOLD_ROCKET;
  if (rocketUp) rocket(L, p, act_ == Act::GOLD_ROCKET);  // behind the planet's ring: it leaves from the far side
  satellites(L, w, false, now);
  planet(L, false);
  satellites(L, w, true, now);
  planet(L, true);

  // what's up: a sign (urgent, review requests) or the agent's card in a speech bubble for a
  // few seconds when it changes (not over a sign); either way the bot moves aside for it
  Urgent u = urgent(d);
  const bool reviews = !u.on && d.agentLinked && w.reviews > 0;
  const bool sign = u.on || reviews;
  if (d.cardA != cardA_) {
    cardA_ = d.cardA;
    cardAt_ = now;
  }
  const bool cardKind = d.cardKind.length() && d.cardKind != "hello" && d.cardKind != "face";
  const bool bubble = !sign && act_ == Act::NONE && d.agentLinked && cardKind && cardAt_ && now - cardAt_ < 7000;

  // the bot: hovering, bobbing, gliding aside; a loop or a wobble when something happened
  const float tx = sign || bubble ? L.asideX : L.botX, ty = sign || bubble ? L.asideY : L.botY;
  if (homeX_ < 0) {
    homeX_ = tx;
    homeY_ = ty;
  }
  homeX_ += (tx - homeX_) * min(1.0f, dt * 3);
  homeY_ += (ty - homeY_) * min(1.0f, dt * 3);
  float bx = homeX_ + sinf(now / 2300.0f) * (sign || bubble ? 2 : wide ? 10 : 8), by = homeY_ + sinf(now / 900.0f) * 3;
  if (act_ == Act::LOOP) {
    const float a = p * 2 * PI;
    bx += sinf(a) * 16;
    by -= (1 - cosf(a)) * 14;
  } else if (act_ == Act::WOBBLE) {
    bx += sinf(now / 35.0f) * 2.5f;
  }
  ui::Mood m = ui::Mood::IDLE;
  if (act_ == Act::GOLD_ROCKET) m = ui::Mood::PARTY;
  else if (act_ == Act::LOOP || act_ == Act::HI || rocketUp) m = ui::Mood::DONE;
  else if (act_ == Act::WOBBLE) m = ui::Mood::ASK;
  else if (act_ == Act::WARP) m = ui::Mood::THINK;
  else if (sign || (d.agentLinked && w.asks > 0)) m = ui::Mood::ASK;
  else if (w.ci == world::Ci::RED) m = ui::Mood::ERROR;
  else if (d.agentLinked && w.nSessions) m = ui::Mood::THINK;
  else if (isNightMinute(d.minute)) m = ui::Mood::OFFLINE;  // sleepy
  if (sign || bubble) lookAt(wide ? -6.0f : 0.0f);  // at the sign
  bot(bx, by, L.k, m, w.ci == world::Ci::RED, act_ == Act::HI, now);
  lookAt(NAN);
  if (act_ == Act::GOLD_ROCKET) confettiDraw(false);

  if (sign) {
    const int bob = (int)(sinf(now / 700.0f) * 1.5f);
    if (u.on) signBoard(L.signX, L.signY + bob, L.signW, L.signH, u.a, u.b, u.bg, u.ink);
    else signBoard(L.signX, L.signY + bob, L.signW, L.signH, w.reviews == 1 ? "Review requested" : String(w.reviews) + " reviews",
                   w.reviews == 1 ? "a PR waits for you" : "PRs wait for you", ui::BLUE, rgb(0x06, 0x16, 0x2a));
  } else if (bubble) {
    speech(L.signX, L.signY, L.signW, L.signH - 4, (int)bx, d.cardA, d.cardB, rgb(0xf4, 0xf6, 0xf8), GROUND);
  }

  // what happened, in a pill over the bot
  if (act_ != Act::NONE && !sign) {
    String label;
    switch (why_) {
      case Kick::MERGED:      label = "PR merged!"; break;
      case Kick::RELEASE:     label = "Release!"; break;
      case Kick::LEVEL_UP:    label = "Level up!"; break;
      case Kick::CI_GREEN:    label = "CI green"; break;
      case Kick::CLAUDE_DONE: label = "Claude done"; break;
      case Kick::CI_RED:      label = "CI failed"; break;
      case Kick::SHAKE:       label = "warp!"; break;
      case Kick::LIFT:        label = "hi!"; break;
      case Kick::REVIEW:      label = "review?"; break;
      default: break;
    }
    if (label.length()) pill(label, constrain((int)bx, 36, W - 36), (int)(by - 36 * L.k), blend(GROUND, AGENT, 0.35f), INK);
  }

  headline(d, wide, INK, rgb(0x02, 0x03, 0x0a));
}

}  // namespace

Face& space() {
  static Space f;
  return f;
}

}  // namespace faces
