// StickyWorld: StickyBot (a little stick with the agent's eyes) lives in a pixel landscape.
//   - the sky follows the time (dawn, day, dusk, night with stars and the moon) and the
//     weather (sun, clouds, rain, snow, fog, a storm's flashes)
//   - a city grows along the horizon: a building per merged PR, a tower per release
//     (logic/city.cpp, the same skyline every time)
//   - the bot's mood: alert (something waits for you: it holds up a sign), sad (CI failed),
//     working (Claude sessions running), sleepy (night), happy (CI green); reactions: eats
//     a snack when a PR merges, celebrates a release / level up, hops when CI goes green or
//     Claude finishes; toys: shake = dizzy, flick = backflip, upside down = panic, lift = hi
//   - level, XP, streak and the city's size along the ground; accessories as it earns them
// The time, the battery / Bluetooth and anything urgent are always on screen.
#include "faces.h"
#include "common.h"
#include "../logic/city.h"
#include "../logic/pet.h"
#include "../logic/sky.h"
#include "../settings.h"
#include "../sound.h"

namespace faces {

using namespace ui;
using pet::Mood;
using world::Kick;

namespace {

constexpr uint16_t SHELL     = rgb(0xe9, 0xec, 0xef);  // the bot's body (the stick)
constexpr uint16_t SHELL_RIM = rgb(0x4a, 0x52, 0x5c);
constexpr uint16_t SCREEN    = rgb(0x12, 0x15, 0x19);
constexpr uint16_t GOLD      = rgb(0xf2, 0xc1, 0x4e);
constexpr uint16_t WINDOW    = rgb(0xff, 0xd1, 0x66);  // lit at night

static uint16_t c(const sky::Rgb& x) { return rgb(x.r, x.g, x.b); }

struct Layout {
  int  groundY, statsY;
  int  signX, signY, signW, signH;
  int  floorH;  // px per building floor
  float k;      // bot scale
};
constexpr Layout PORTRAIT{180, 200, 8, 46, 119, 58, 3, 1.2f};
constexpr Layout WIDE{100, 114, 94, 22, 140, 44, 2, 1.0f};

struct Reaction {
  Mood     m = Mood::IDLE;
  uint32_t at = 0;     // when it was kicked (it starts when it's first drawn)
  Kick     why = Kick::NONE;
};

class StickyWorld : public Face {
 public:
  const char* id() const override { return "stickyworld"; }
  const char* title() const override { return "StickyWorld"; }
  const char* about() const override {
    return "StickyBot in a pixel world: the sky follows the time and weather, a city grows with your merged PRs";
  }

  void reset() override {
    x_ = tx_ = -1;
    nq_ = 0;
    cur_ = Reaction{};
    curStart_ = 0;
    cityKey_ = 0;
    cardA_ = "";
    munched_ = false;
  }

  uint32_t frameMs(bool onBattery, uint32_t idleMs) const override {
    if (!onBattery) return 40;
    return idleMs < 20000 ? 66 : 150;
  }

  void kick(Kick k, uint32_t now) override {
    Mood m;
    switch (k) {
      case Kick::MERGED:      m = Mood::EATING; break;
      case Kick::RELEASE:
      case Kick::LEVEL_UP:    m = Mood::CELEBRATING; break;
      case Kick::CI_GREEN:
      case Kick::CLAUDE_DONE: m = Mood::HOP; break;
      case Kick::CI_RED:
      case Kick::UPSIDE_DOWN: m = Mood::PANIC; break;
      case Kick::SHAKE:       m = Mood::DIZZY; break;
      case Kick::FLICK:       m = Mood::FLIP; break;
      case Kick::REVIEW:
      case Kick::LIFT:        m = Mood::LOOK; break;
      default: return;
    }
    if (nq_ == QN) {  // full: toys make room for news
      for (int i = 0; i < nq_; i++) q_[i] = q_[i + 1 < QN ? i + 1 : i];
      nq_--;
    }
    q_[nq_++] = {m, now, k};
  }

  void draw(const Dash& d, bool wide, uint32_t now) override;

 private:
  static constexpr int QN = 4;
  Reaction q_[QN];
  int      nq_ = 0;
  Reaction cur_;
  uint32_t curStart_ = 0;
  bool     munched_ = false;
  float    x_ = -1, tx_ = -1, walkPh_ = 0;
  int      facing_ = 1;
  uint32_t idleUntil_ = 0, last_ = 0;
  String   cardA_;
  uint32_t cardAt_ = 0;
  // the skyline, laid out again only when it changes
  city::Placed city_[72];
  int          nCity_ = 0;
  uint32_t     cityKey_ = 0;
  uint16_t     haze_ = 0;  // the sky at the horizon: the city fades into it by day

  Mood reaction(uint32_t now, uint32_t& into);
  void sky(const Dash& d, const Layout& L, uint32_t now, bool& night, float& light);
  void skyline(const world::Snapshot& w, const Layout& L, bool night, float light, uint32_t now);
  void ground(const Dash& d, const Layout& L, bool night);
  void weather(const Dash& d, const Layout& L, uint32_t now, bool night);
  void bot(float cx, int footY, float k, Mood m, uint32_t into, uint32_t now, uint8_t acc, bool night, bool walking,
           int poleX, int poleTop);
  void food(int kind, float x, float y, float s, float eaten);
  void stats(const Dash& d, const world::Snapshot& w, const Layout& L, bool wide, uint16_t ink, uint16_t shade);
};

// --- Reactions ---------------------------------------------------------------------------

// The reaction playing now (it starts at its first draw, so one kicked while an event screen
// was up still plays when Home comes back), or the steady mood (IDLE as a placeholder).
Mood StickyWorld::reaction(uint32_t now, uint32_t& into) {
  if (cur_.m != Mood::IDLE && now - curStart_ >= pet::reactionMs(cur_.m)) cur_ = Reaction{};
  while (cur_.m == Mood::IDLE && nq_) {
    cur_ = q_[0];
    for (int i = 1; i < nq_; i++) q_[i - 1] = q_[i];
    nq_--;
    if (now - cur_.at > 5 * 60000UL) { cur_ = Reaction{}; continue; }  // stale (the screen was off)
    curStart_ = now;
    munched_ = false;
    if (cur_.m == Mood::CELEBRATING) confettiReset(x_, 60);
  }
  into = now - curStart_;
  return cur_.m;
}

// --- The scene ---------------------------------------------------------------------------

void StickyWorld::sky(const Dash& d, const Layout& L, uint32_t now, bool& night, float& light) {
  const int minute = d.minute >= 0 ? d.minute : 12 * 60;
  const char* icon = d.wxValid ? d.wxIcon.c_str() : "clear";
  sky::Colors sc = sky::colors(minute, sky::overcast(icon));
  light = sc.light;
  night = light < 0.3f;
  uint16_t top = c(sc.top), bot = c(sc.bottom);
  haze_ = bot;
  const bool storm = d.wxValid && d.wxIcon == "storm";
  const uint32_t ph = now % 7000;
  const bool flash = storm && (ph < 90 || (ph > 170 && ph < 230));
  if (flash) { top = blend(top, TFT_WHITE, 0.5f); bot = blend(bot, TFT_WHITE, 0.5f); }
  for (int y = 0; y < L.groundY; y += 4) canvas.fillRect(0, y, W, 4, blend(top, bot, (float)y / L.groundY));

  // stars, twinkling
  if (light < 0.35f) {
    const int n = W > H ? 34 : 26;
    for (int i = 0; i < n; i++) {
      int sx = hash32(i * 7 + 1) % W, sy = 4 + hash32(i * 7 + 2) % (L.groundY - 36);
      float tw = 0.5f + 0.5f * sinf(now / 520.0f + i * 1.7f);
      uint16_t col = blend(blend(top, bot, (float)sy / L.groundY), INK, (0.35f + 0.65f * tw) * (1 - light / 0.35f));
      int sz = i % 5 == 0 ? 2 : 1;
      canvas.fillRect(sx, sy, sz, sz, col);
    }
  }
  // the sun or the moon on its arc
  float t;
  auto arc = [&](float t, int& x, int& y) {
    x = 16 + (int)(t * (W - 32));
    y = L.groundY - 14 - (int)(sinf(PI * t) * (L.groundY - (W > H ? 52 : 74)));
  };
  if (sky::sunAt(minute, t) && !sky::overcast(icon)) {
    int x, y;
    arc(t, x, y);
    uint16_t sunC = blend(SUN, rgb(0xff, 0x7a, 0x45), constrain(1.2f - light, 0.0f, 1.0f));
    for (int r = 12; r > 7; r -= 2) canvas.fillCircle(x, y, r, blend(blend(top, bot, (float)y / L.groundY), sunC, 0.18f * (13 - r) / 2));
    canvas.fillCircle(x, y, 7, sunC);
    canvas.fillRect(x - 3, y - 4, 3, 2, blend(sunC, TFT_WHITE, 0.5f));  // a pixel glint
  } else if (sky::moonAt(minute, t)) {
    int x, y;
    arc(t, x, y);
    canvas.fillCircle(x, y, 6, SNOW);
    canvas.fillCircle(x + 3, y - 2, 5, blend(top, bot, (float)y / L.groundY));
    canvas.fillRect(x - 3, y + 1, 2, 2, blend(SNOW, FAINT, 0.4f));  // a crater
  }
  // clouds, drifting
  int nc = d.wxValid ? sky::clouds(icon) : 0;
  uint16_t cloud = night ? rgb(0x3a, 0x40, 0x58) : sky::overcast(icon) ? rgb(0xc0, 0xc7, 0xcf) : rgb(0xf4, 0xf6, 0xf8);
  if (storm) cloud = blend(cloud, rgb(0x40, 0x46, 0x50), 0.5f);
  if (flash) cloud = blend(cloud, TFT_WHITE, 0.5f);
  for (int i = 0; i < nc; i++) {
    float speed = 2.5f + i * 1.3f;
    int span = W + 60;
    int cx = (int)fmodf(hash32(i + 40) % span + now / 1000.0f * speed, (float)span) - 30;
    int cy = (W > H ? 26 : 50) + hash32(i + 50) % (W > H ? 30 : 46);
    uint16_t cc = i & 1 ? blend(cloud, top, 0.12f) : cloud;
    canvas.fillRoundRect(cx, cy + 5, 28, 9, 4, cc);
    canvas.fillRoundRect(cx + 5, cy, 13, 10, 5, cc);
    canvas.fillRoundRect(cx + 13, cy + 2, 11, 8, 4, cc);
  }
  if (flash && ph < 90) {  // the bolt
    int bx = 20 + hash32(now / 7000) % (W - 40), y0 = 40, y1 = L.groundY - 30;
    int mid = (y0 + y1) / 2;
    canvas.drawWideLine(bx, y0, bx - 6, mid, 2, TFT_WHITE);
    canvas.drawWideLine(bx - 6, mid, bx + 3, mid, 2, TFT_WHITE);
    canvas.drawWideLine(bx + 3, mid, bx - 4, y1, 2, TFT_WHITE);
  }
}

void StickyWorld::skyline(const world::Snapshot& w, const Layout& L, bool night, float light, uint32_t now) {
  static const uint16_t WALLS[] = {rgb(0x8e, 0x9a, 0xaf), rgb(0xc9, 0xad, 0xa7), rgb(0x9a, 0x8c, 0x98),
                                   rgb(0xa3, 0xb1, 0x8a), rgb(0xd4, 0xa3, 0x73), rgb(0x6d, 0x82, 0x99)};
  const int cell = 2;
  uint32_t key = hash32(w.seed ^ (w.merged * 977u) ^ (w.nTowers * 131u) ^ (W * 7u)) | 1;
  if (key != cityKey_) {
    cityKey_ = key;
    nCity_ = city::layout(w.seed, w.merged, w.towers, w.nTowers, W - 4, cell, city_, 72);
  }
  const uint16_t dusk = rgb(0x0b, 0x0f, 0x24);
  const float dark = constrain((0.6f - light) / 0.5f, 0.0f, 0.78f);  // walls fade into the night
  for (int i = 0; i < nCity_; i++) {
    const city::Placed& p = city_[i];
    const city::Building& b = p.b;
    const int x = p.x + 2, bw = p.w, bh = b.h * L.floorH, top = L.groundY - bh;
    uint16_t wall = b.tower ? GOLD : WALLS[b.color % 6];
    wall = blend(blend(wall, haze_, 0.3f * light), dusk, dark);  // far away: a little hazy by day
    const uint16_t trim = blend(wall, TFT_BLACK, 0.25f);
    // roof
    if (b.tower) {
      canvas.fillTriangle(x, top, x + bw - 1, top, x + bw / 2, top - 7, trim);
      canvas.drawFastVLine(x + bw / 2, top - 12, 5, trim);
      canvas.fillRect(x + bw / 2 + 1, top - 12, 3, 2, night ? WINDOW : ERR);  // a flag
    } else if (b.roof == city::PEAK && bw >= 5) {
      canvas.fillTriangle(x, top, x + bw - 1, top, x + bw / 2, top - bw / 2, trim);
    } else if (b.roof == city::DOME && bw >= 5) {
      canvas.fillCircle(x + bw / 2, top, bw / 2 - 1, trim);
    } else if (b.roof == city::ANTENNA) {
      canvas.drawFastVLine(x + bw / 2, top - 5, 5, trim);
      if (night && (now / 700 + i) % 2) canvas.fillRect(x + bw / 2, top - 6, 1, 1, ERR);
    }
    canvas.fillRect(x, top, bw, bh, wall);
    canvas.drawFastHLine(x, top, bw, blend(wall, TFT_WHITE, 0.2f));
    if (b.tower) canvas.fillRect(x, top + 4, bw, 2, trim);
    // windows: 2 x 2 px, one per floor and column; lit at night by the building's bits
    if (bw >= 5) {
      const int cols = (bw - 1) / 3;
      int bit = 0;
      for (int f = 1; f * L.floorH + 2 <= bh - 1; f++) {
        int wy = top + bh - f * L.floorH - (L.floorH > 2 ? 1 : 0);
        for (int cI = 0; cI < cols; cI++, bit++) {
          bool lit = (b.windows >> (bit % 16)) & 1;
          uint16_t wc = night ? (lit ? WINDOW : blend(wall, TFT_BLACK, 0.3f))
                              : blend(wall, rgb(0x2b, 0x3a, 0x55), 0.45f);
          canvas.fillRect(x + 1 + cI * 3, wy, 2, L.floorH > 2 ? 2 : 1, wc);
        }
      }
    }
  }
}

void StickyWorld::ground(const Dash& d, const Layout& L, bool night) {
  const bool snow = d.wxValid && d.wxIcon == "snow";
  uint16_t grass = night ? rgb(0x1f, 0x3b, 0x2a) : rgb(0x5a, 0x9e, 0x4b);
  uint16_t dirt = night ? rgb(0x22, 0x1b, 0x16) : rgb(0x6e, 0x52, 0x37);
  if (snow) grass = night ? rgb(0x8a, 0x93, 0xa6) : rgb(0xee, 0xf2, 0xf6);
  canvas.fillRect(0, L.groundY, W, H - L.groundY, dirt);
  canvas.fillRect(0, L.groundY, W, 4, grass);
  for (int x = 0; x < W; x += 6)  // tufts
    if (hash32(x + 7) % 3 == 0) canvas.fillRect(x + 2, L.groundY - 1, 2, 1, grass);
  const uint16_t speck = blend(dirt, TFT_BLACK, 0.3f);
  for (int i = 0; i < 22; i++)
    canvas.fillRect(hash32(i + 300) % W, L.groundY + 7 + hash32(i + 400) % max(4, H - L.groundY - 10), 2, 2, speck);
}

void StickyWorld::weather(const Dash& d, const Layout& L, uint32_t now, bool night) {
  if (!d.wxValid) return;
  const String& w = d.wxIcon;
  if (w == "rain" || w == "storm") {
    const int n = W > H ? 30 : 24;
    const uint16_t rc = night ? blend(RAIN, GROUND, 0.4f) : RAIN;
    for (int i = 0; i < n; i++) {
      float y = fmodf(hash32(i + 600) % 300 + now * 0.17f, (float)L.groundY + 8);
      float x = fmodf(hash32(i + 700) % W - y * 0.25f + W, (float)W);
      canvas.drawFastVLine((int)x, (int)y - 4, 4, rc);
    }
  } else if (w == "snow") {
    const int n = W > H ? 34 : 28;
    for (int i = 0; i < n; i++) {
      float y = fmodf(hash32(i + 800) % 300 + now * 0.024f, (float)L.groundY + 4);
      float x = hash32(i + 900) % W + sinf(now / 900.0f + i) * 5;
      canvas.fillRect((int)x, (int)y, 2, 2, SNOW);
    }
  } else if (w == "fog") {
    const uint16_t f = night ? rgb(0x3a, 0x40, 0x4a) : rgb(0xc8, 0xcd, 0xd3);
    for (int i = 0; i < 3; i++) {
      int y = L.groundY - 30 + i * 9, off = (int)(now / 120 + i * 40) % 60;
      for (int x = -off; x < W; x += 60) canvas.fillRoundRect(x, y, 44, 4, 2, f);
    }
  }
}

// A snack (by how many PRs merged): an apple, a cookie, a donut, a slice of pizza.
void StickyWorld::food(int kind, float x, float y, float s, float eaten) {
  const float r = 5 * s * (1 - 0.8f * eaten);
  if (r < 1) return;
  switch (kind % 4) {
    case 0:  // apple
      canvas.fillCircle(x, y, r, rgb(0xe6, 0x39, 0x46));
      canvas.fillRect(x, y - r - 2, 1, 3, rgb(0x6b, 0x47, 0x2a));
      canvas.fillRect(x + 1, y - r - 2, 3, 2, ui::OK);
      break;
    case 1:  // cookie
      canvas.fillCircle(x, y, r, rgb(0xd4, 0xa3, 0x73));
      canvas.fillRect(x - r / 2, y - r / 3, 2, 2, rgb(0x4a, 0x2c, 0x16));
      canvas.fillRect(x + r / 3, y + r / 4, 2, 2, rgb(0x4a, 0x2c, 0x16));
      break;
    case 2:  // donut
      canvas.fillCircle(x, y, r, rgb(0xf2, 0x8d, 0xb2));
      canvas.fillCircle(x, y, max(1.0f, r / 2.6f), SCREEN);
      canvas.fillRect(x - r / 2, y - r / 2, 2, 1, SNOW);
      break;
    default:  // pizza
      canvas.fillTriangle(x - r, y - r, x + r, y - r, x, y + r, rgb(0xf4, 0xc4, 0x5f));
      canvas.fillRect(x - r, y - r - 1, 2 * r, 2, rgb(0xc0, 0x7a, 0x3a));
      canvas.fillCircle(x, y - r / 3, max(1.0f, r / 4), ERR);
      break;
  }
}

// The bot: a little stick (light shell, dark screen with the eyes, button A), on two legs.
// poleX >= 0: it holds a sign's pole there (up to poleTop).
void StickyWorld::bot(float cx, int footY, float k, Mood m, uint32_t into, uint32_t now, uint8_t acc, bool night,
                      bool walking, int poleX, int poleTop) {
  float yOff = 0, xOff = 0, sx = 1, sy = 1;
  bool flip = false;
  const float p = pet::reactionMs(m) ? (float)into / pet::reactionMs(m) : 0;
  switch (m) {
    case Mood::HOP: yOff = -sinf(PI * p) * 18 * k; if (p < 0.1f || p > 0.92f) { sy = 0.86f; sx = 1.1f; } break;
    case Mood::FLIP: yOff = -sinf(PI * p) * 26 * k; flip = p > 0.3f && p < 0.7f; break;
    case Mood::CELEBRATING: yOff = -sinf(PI * ((into % 650) / 650.0f)) * 12 * k; break;
    case Mood::PANIC: xOff = sinf(now / 22.0f) * 2.2f; break;
    case Mood::DIZZY: xOff = sinf(now / 170.0f) * 3.5f; break;
    case Mood::SLEEPY: sy = 0.93f + 0.025f * sinf(now / 800.0f); break;
    case Mood::LOOK: k *= 1.12f; break;
    case Mood::EATING: if (p > 0.3f && p < 0.8f) sy = 1 + 0.04f * sinf(into / 45.0f); break;
    default:
      yOff = walking ? -fabsf(sinf(walkPh_)) * 1.6f : sinf(now / 520.0f) * 0.7f;
      break;
  }
  const float bw = 28 * k * sx, bh = 32 * k * sy, legH = 4 * k;
  const float bottom = footY - legH + yOff, top = bottom - bh, left = cx + xOff - bw / 2;
  const uint16_t shell = night ? blend(SHELL, rgb(0x20, 0x26, 0x40), 0.45f) : SHELL;
  const uint16_t rim = night ? blend(SHELL_RIM, rgb(0x10, 0x14, 0x28), 0.4f) : SHELL_RIM;
  const uint16_t legC = rgb(0x3a, 0x40, 0x48);
  // its shadow on the ground (smaller while it's in the air)
  const float air = constrain(-yOff / (26 * k), 0.0f, 1.0f);
  canvas.fillEllipse(cx + xOff, footY - 1, 12 * k * (1 - 0.4f * air), 2.2f * k, blend(rgb(0x20, 0x30, 0x18), TFT_BLACK, night ? 0.6f : 0.2f));
  // legs (on top when it's upside down mid-flip)
  float swing = walking ? sinf(walkPh_) * 1.6f * k : 0;
  float ly = flip ? top - legH : bottom;
  canvas.fillRoundRect(cx + xOff - 8 * k, ly + (flip ? 0 : swing > 0 ? -swing : 0), 5 * k, legH + fabsf(swing), 2, legC);
  canvas.fillRoundRect(cx + xOff + 3 * k, ly + (flip ? 0 : swing < 0 ? swing : 0), 5 * k, legH + fabsf(swing), 2, legC);
  // the shell
  canvas.fillRoundRect(left, top, bw, bh, 7 * k, shell);
  canvas.drawRoundRect(left, top, bw, bh, 7 * k, rim);
  // the screen with the eyes (at the bottom when upside down)
  const float pw = bw - 6 * k, ph = bh * 0.58f;
  const float py = flip ? bottom - 3 * k - ph : top + 3 * k;
  canvas.fillRoundRect(left + 3 * k, py, pw, ph, 4 * k, SCREEN);
  const float ecx = cx + xOff, ecy = py + ph / 2 + (flip ? 1 : 0);
  // button A under the screen
  canvas.fillCircle(ecx, flip ? top + bh * 0.2f : top + bh * 0.8f, 2.6f * k, blend(shell, AGENT, 0.55f));

  ui::Mood em = ui::Mood::IDLE;
  switch (m) {
    case Mood::HAPPY: case Mood::HOP: em = ui::Mood::DONE; break;
    case Mood::WORKING: case Mood::DIZZY: em = ui::Mood::THINK; break;
    case Mood::ALERT: case Mood::PANIC: em = ui::Mood::ASK; break;
    case Mood::SAD: em = ui::Mood::ERROR; break;
    case Mood::SLEEPY: em = ui::Mood::OFFLINE; break;
    case Mood::CELEBRATING: em = ui::Mood::PARTY; break;
    case Mood::EATING: em = p < 0.3f ? ui::Mood::ASK : ui::Mood::DONE; break;
    default: break;
  }
  if (m == Mood::LOOK) lookAt(0);
  else if (em == ui::Mood::IDLE && walking) lookAt(facing_ * 6.0f);
  const float es = 0.19f * k;
  eyes(em, ecx, ecy, es, moodColor(em), SCREEN);
  lookAt(NAN);
  // munching
  if (m == Mood::EATING && p >= 0.3f && p < 0.8f) {
    float open = 0.5f + 0.5f * sinf(into / 70.0f);
    float my = py + ph - 4 * k;
    canvas.fillEllipse(ecx, my, 3.2f * k, 0.8f + open * 1.8f * k, moodColor(ui::Mood::DONE));
    if (open > 0.4f) canvas.fillEllipse(ecx, my, 2.0f * k, open * 1.1f * k, SCREEN);
  }
  // accessories (not mid-flip)
  if (!flip) {
    const bool shadesOn = (acc & pet::SHADES) &&
                          (m == Mood::IDLE || m == Mood::HAPPY || m == Mood::WORKING || m == Mood::HOP || m == Mood::LOOK);
    if (acc & pet::SHADES) {
      const float gap = 30 * es, lw = 9 * k, lh = 5.5f * k;
      const float gy = shadesOn ? ecy - 2.5f * k : top - 1;  // off duty: pushed up on its head
      for (int side = -1; side <= 1; side += 2) {
        canvas.fillRoundRect(ecx + side * gap - lw / 2, gy, lw, lh, 2, rgb(0x08, 0x09, 0x0b));
        canvas.drawFastHLine(ecx + side * gap - lw / 2 + 2, gy + 1, 3, rgb(0x6c, 0x76, 0x80));
      }
      canvas.drawFastHLine(ecx - gap + lw / 2, gy + 1, 2 * gap - lw, rgb(0x08, 0x09, 0x0b));
    }
    if (acc & pet::CROWN) {
      const float cw = bw * 0.55f, cb = top - 1, cl = ecx - cw / 2;
      canvas.fillRect(cl, cb - 3 * k, cw, 3 * k, GOLD);
      for (int i = 0; i < 3; i++) {
        float tx = cl + cw * (i + 0.5f) / 3;
        canvas.fillTriangle(tx - 3 * k, cb - 3 * k, tx + 3 * k, cb - 3 * k, tx, cb - 8 * k, GOLD);
      }
      canvas.fillRect(ecx - 1, cb - 2.5f * k, 2, 2, ERR);
    } else if (acc & pet::HAT) {
      const float hw = bw * 0.42f, brim = bw * 0.72f, hb = top + 1;
      canvas.fillRoundRect(ecx - brim / 2, hb - 2 * k, brim, 2.5f * k, 1, rgb(0x22, 0x24, 0x2a));
      canvas.fillRect(ecx - hw / 2, hb - 10 * k, hw, 8.5f * k, rgb(0x22, 0x24, 0x2a));
      canvas.fillRect(ecx - hw / 2, hb - 4.5f * k, hw, 1.8f * k, WISPR);
    }
  }
  // the sign's pole, held in its right hand
  if (poleX >= 0) {
    float hy = top + bh * 0.55f;
    canvas.fillRect(poleX - 1, poleTop, 3, hy - poleTop + 3, rgb(0x8a, 0x62, 0x3c));
    canvas.drawWideLine(left + bw - 1, hy + 2, poleX - 1, hy, 1.6f * k, rim);
    canvas.fillCircle(poleX, hy, 2.2f * k, shell);
  }
  // extras
  if (m == Mood::CELEBRATING) {  // arms up
    canvas.drawWideLine(left + 1, top + bh * 0.5f, left - 5 * k, top + bh * 0.1f, 1.6f * k, rim);
    canvas.drawWideLine(left + bw - 1, top + bh * 0.5f, left + bw + 5 * k, top + bh * 0.1f, 1.6f * k, rim);
  } else if (m == Mood::SAD) {  // a little rain cloud of its own
    float cy = top - 9 * k;
    uint16_t cc = rgb(0x8a, 0x93, 0xa0);
    canvas.fillRoundRect(ecx - 9 * k, cy, 18 * k, 5 * k, 2, cc);
    canvas.fillCircle(ecx - 2 * k, cy, 3.5f * k, cc);
    for (int i = 0; i < 3; i++) {
      float dy = fmodf(now / 9.0f + i * 7, 9 * k);
      canvas.drawFastVLine(ecx - 6 * k + i * 6 * k, cy + 6 * k + dy, 2, RAIN);
    }
  } else if (m == Mood::PANIC) {  // "!" and a sweat drop
    canvas.fillCircle(left + bw + 2, top + 6 * k + fmodf(now / 30.0f, 6), 1.6f * k, RAIN);
  } else if (m == Mood::DIZZY) {  // stars round its head
    for (int i = 0; i < 3; i++) {
      float a = now / 260.0f + i * 2.09f;
      canvas.fillRect(ecx + cosf(a) * 11 * k - 1, top - 5 * k + sinf(a) * 3 * k - 1, 3, 3, SUN);
    }
  }
}

void StickyWorld::stats(const Dash& d, const world::Snapshot& w, const Layout& L, bool wide, uint16_t ink,
                        uint16_t shade) {
  const pet::Level lv = pet::levelFor(w.xp);
  const int y = L.statsY;
  String lvS = "Lv " + String(lv.level);
  canvas.setFont(F12);
  int x = 6;
  shadowTxt(lvS, x, y, F12, ink, shade);
  x += canvas.textWidth(lvS) + 5;
  const int bw = wide ? 46 : 28;
  canvas.fillRoundRect(x, y - 3, bw, 6, 3, blend(shade, ink, 0.25f));
  canvas.fillRoundRect(x, y - 3, max(4, bw * lv.into / max(1, lv.need)), 6, 3, AGENT);
  x += bw + 7;
  // the streak: a flame
  const uint16_t fl = w.streak ? ASK : FAINT;
  canvas.fillTriangle(x, y + 4, x + 7, y + 4, x + 3, y - 6, fl);
  canvas.fillCircle(x + 3, y + 2, 3, fl);
  canvas.fillCircle(x + 3, y + 2, 1, w.streak ? SUN : DIM);
  shadowTxt(String(w.streak), x + 10, y, F12, ink, shade);
  // the city: a little building + how many
  String n = String(w.merged);
  canvas.setFont(F12);
  int nx = W - 6 - canvas.textWidth(n);
  shadowTxt(n, nx, y, F12, ink, shade);
  int bx = nx - 11;
  canvas.fillRect(bx, y - 5, 7, 10, blend(shade, ink, 0.6f));
  canvas.fillRect(bx + 2, y - 3, 1, 1, shade);
  canvas.fillRect(bx + 4, y - 3, 1, 1, shade);
  canvas.fillRect(bx + 2, y, 1, 1, shade);
  canvas.fillRect(bx + 4, y, 1, 1, shade);
  if (wide && w.mergedToday) shadowTxt("+" + String(w.mergedToday) + " today", bx - 6, y, F9, ink, shade, middle_right);
}

void StickyWorld::draw(const Dash& d, bool wide, uint32_t now) {
  const Layout& L = wide ? WIDE : PORTRAIT;
  const world::Snapshot& w = faces::world();
  float dt = last_ ? constrain((now - last_) / 1000.0f, 0.0f, 0.25f) : 0;
  last_ = now;

  bool night;
  float light;
  sky(d, L, now, night, light);
  skyline(w, L, night, light, now);
  ground(d, L, night);
  weather(d, L, now, night);

  // what to show and how the bot feels
  Urgent u = urgent(d);
  const bool reviews = !u.on && d.agentLinked && w.reviews > 0;
  pet::MoodInputs in{u.on || reviews || (d.agentLinked && w.asks > 0), w.ci == world::Ci::RED,
                     w.ci == world::Ci::GREEN, d.agentLinked ? w.nSessions : 0,
                     isNightMinute(d.minute) || light < 0.15f};
  uint32_t into = 0;
  Mood m = reaction(now, into);
  const bool reacting = m != Mood::IDLE;
  if (!reacting) m = pet::steadyMood(in);
  const bool holdSign = u.on || reviews;

  // where it goes
  const float margin = 18 * L.k;
  if (x_ < 0) x_ = tx_ = W * 0.4f;
  if (holdSign) {
    tx_ = L.signX + 12 - 14 * L.k;  // its hand under the pole
  } else if (!reacting && m != Mood::SLEEPY && fabsf(tx_ - x_) < 1 && now > idleUntil_) {
    tx_ = margin + frac01(now) * (W - 2 * margin);
    idleUntil_ = now + 2500 + (uint32_t)(frac01(now + 1) * 4000);
  }
  bool walking = false;
  if (!reacting && m != Mood::SLEEPY && fabsf(tx_ - x_) >= 1) {
    float speed = holdSign ? 34 : m == Mood::WORKING ? 26 : m == Mood::SAD ? 9 : 16;
    float step = min(fabsf(tx_ - x_), speed * dt);
    facing_ = tx_ > x_ ? 1 : -1;
    x_ += facing_ * step;
    walkPh_ += step * 0.45f;
    walking = step > 0;
    if (fabsf(tx_ - x_) < 1) idleUntil_ = max(idleUntil_, now + 1500);
  }
  x_ = constrain(x_, margin, W - margin);

  // the sign it holds up (urgent things, review requests)
  int poleX = -1, poleTop = 0;
  if (holdSign && fabsf(tx_ - x_) < 3) {
    poleX = L.signX + 12;
    poleTop = L.signY + L.signH - 2;
  }
  bot(x_, L.groundY + 1, L.k, m, into, now, w.acc, night, walking, poleX, poleTop);
  // the snack falls, then gets eaten
  if (m == Mood::EATING) {
    const float p = (float)into / pet::reactionMs(m);
    const float mouthY = L.groundY - 4 * L.k - 32 * L.k * 0.42f;
    if (p < 0.3f) food(w.merged, x_, -8 + (mouthY + 8) * smooth(p / 0.3f), L.k, 0);
    else if (p < 0.8f) {
      if (!munched_) {
        munched_ = true;
        if (settings::soundOn()) sound::play(String("pop"));
      }
      food(w.merged, x_ + 7 * L.k, mouthY + 4, L.k, (p - 0.3f) / 0.5f);
      for (int i = 0; i < 4; i++) {  // crumbs
        float cp = fmodf(into / 300.0f + i * 0.25f, 1.0f);
        canvas.fillRect(x_ + 8 + i * 3 - 6, mouthY + 6 + cp * 14, 2, 2, rgb(0xd4, 0xa3, 0x73));
      }
    }
  }

  if (holdSign) {
    // a little bounce while it's lifted up, steady once held
    if (u.on) signBoard(L.signX, L.signY, L.signW, L.signH, u.a, u.b, u.bg, u.ink);
    else signBoard(L.signX, L.signY, L.signW, L.signH, w.reviews == 1 ? "Review requested" : String(w.reviews) + " reviews",
                   w.reviews == 1 ? "a PR waits for you" : "PRs wait for you", ui::BLUE, rgb(0x06, 0x16, 0x2a));
  }

  if (m == Mood::CELEBRATING) confettiDraw(false);

  // reactions say what happened
  if (reacting) {
    String label;
    switch (cur_.why) {
      case Kick::MERGED:      label = "+" + String(pet::XP_MERGE) + " XP"; break;
      case Kick::RELEASE:     label = "Release!"; break;
      case Kick::LEVEL_UP:    label = "Level " + String(pet::levelFor(w.xp).level) + "!"; break;
      case Kick::CI_GREEN:    label = "CI green"; break;
      case Kick::CLAUDE_DONE: label = "Claude done"; break;
      case Kick::CI_RED:      label = "CI failed"; break;
      case Kick::UPSIDE_DOWN: label = "whoa!"; break;
      case Kick::LIFT:        label = "hi!"; break;
      case Kick::REVIEW:      label = "review?"; break;
      case Kick::SHAKE:       label = "@#%!"; break;
      default: break;
    }
    bool late = m != Mood::EATING || (float)into / pet::reactionMs(m) > 0.75f;  // XP after the munch
    if (label.length() && late && !holdSign) {
      float rise = min(1.0f, into / 600.0f) * 6;
      pill(label, constrain((int)x_, 34, W - 34), L.groundY - (int)(52 * L.k) - (int)rise,
           blend(GROUND, AGENT, 0.35f), INK);
    }
  }

  // a speech bubble with the agent's card now and then (not over a sign)
  if (d.cardA != cardA_) {
    cardA_ = d.cardA;
    cardAt_ = now;
  }
  const bool cardKind = d.cardKind.length() && d.cardKind != "hello" && d.cardKind != "face";
  if (!holdSign && !reacting && d.agentLinked && cardKind && now - cardAt_ < 7000 && cardAt_) {
    if (wide) speech(96, 22, 138, 40, (int)x_, d.cardA, d.cardB, rgb(0xf4, 0xf6, 0xf8), GROUND);
    else speech(6, 48, W - 12, 42, (int)x_, d.cardA, d.cardB, rgb(0xf4, 0xf6, 0xf8), GROUND);
  }

  // always: the time, battery / Bluetooth, the temperature, the stats
  const uint16_t ink = INK, shade = rgb(0x05, 0x07, 0x10);
  const int tw = timeText(d, 6, wide ? 15 : 17, F24, ink, shade);
  String sub = quietNotice(d);
  if (!sub.length()) sub = d.date;
  if (sub.length()) shadowTxt(ellipsize(sub, F9, wide ? 120 : W - 12), 6, wide ? 34 : 37, F9, blend(ink, shade, 0.15f), shade);
  statusIcons(d, W - 4, 9, ink);
  if (d.wxValid && wide) shadowTxt(String(d.wxTemp) + "°", 6 + tw + 8, 15, F14, ink, shade);  // the right is the sign's
  else if (d.wxValid) shadowTxt(String(d.wxTemp) + "°", W - 6, 25, F12, ink, shade, middle_right);
  stats(d, w, L, wide, ink, shade);
}

}  // namespace

Face& stickyWorld() {
  static StickyWorld f;
  return f;
}

}  // namespace faces
