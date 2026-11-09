#include "draw.h"
#include <math.h>
#include "../ui_draw.h"
#include "../logic/pet.h"

namespace games {
namespace draw {

using namespace ui;
using G = lgfx::LovyanGFX;

constexpr uint16_t SHELL = rgb(0xe9, 0xec, 0xef);  // StickyBot (as in StickyWorld)
constexpr uint16_t SHELL_RIM = rgb(0x4a, 0x52, 0x5c);
constexpr uint16_t SCREEN = rgb(0x12, 0x15, 0x19);
constexpr uint16_t LEG = rgb(0x3a, 0x40, 0x48);
constexpr uint16_t GOLD = rgb(0xf2, 0xc1, 0x4e);
constexpr uint16_t RIM = rgb(0x05, 0x06, 0x08);   // text outlines

uint16_t accent(Id g) {
  switch (g) {
    case Id::STACK: return AMBER;
    case Id::FLAPPY: return ui::OK;
    case Id::DINO: return SUN;
    case Id::REACTION: return ERR;
    case Id::MAZE: return AGENT;
    case Id::BALANCE: return ui::VIOLET;
    case Id::CATCH: return ui::BLUE;
    default: return INK;
  }
}

// --- The cached background --------------------------------------------------------------

static M5Canvas bg(&M5.Display);
static uint32_t bgKey = 0;
static int bgW = 0, bgH = 0;
static bool bgOk = false;

// Copies the background for `key` under the frame; paints it first if it's a new one. Without
// the memory for it, paints straight onto the canvas every frame (slower, same picture).
static void backdrop(uint32_t key, void (*paint)(G& g, uint32_t key)) {
  if (!bgOk || bgW != W || bgH != H) {
    bg.deleteSprite();
    bg.setColorDepth(16);
    bg.setPsram(true);
    bgOk = bg.createSprite(W, H) != nullptr;
    bgW = W;
    bgH = H;
    bgKey = 0;
  }
  if (!bgOk) { paint(canvas, key); return; }
  if (key != bgKey) {
    paint(bg, key);
    bgKey = key;
  }
  bg.pushSprite(&canvas, 0, 0);
}

void release() {
  bg.deleteSprite();
  bgOk = false;
  bgKey = 0;
}

void present() { canvas.pushSprite(0, 0); }

static void vgrad(G& g, int y0, int h, uint16_t top, uint16_t bottom) {
  for (int y = 0; y < h; y += 2) g.fillRect(0, y0 + y, g.width(), 2, blend(top, bottom, (float)y / h));
}

// Darkens what's on the canvas (to ~3/8): the pause and game-over panels sit on it. The
// sprite keeps its pixels byte-swapped.
static void dim() {
  uint16_t* p = (uint16_t*)canvas.getBuffer();
  if (!p) return;
  for (int i = 0, n = W * H; i < n; i++) {
    uint16_t c = (uint16_t)((p[i] >> 8) | (p[i] << 8));
    c = (uint16_t)(((c >> 2) & 0x39E7) + ((c >> 3) & 0x18E3));
    p[i] = (uint16_t)((c >> 8) | (c << 8));
  }
}

// Text with a dark rim, readable on any background
static void outline(const String& s, int x, int y, const lgfx::IFont* f, uint16_t ink, textdatum_t d = middle_center) {
  canvas.setFont(f);
  canvas.setTextDatum(d);
  canvas.setTextColor(RIM);
  static const int8_t O[8][2] = {{-1, -1}, {0, -1}, {1, -1}, {-1, 0}, {1, 0}, {-1, 1}, {0, 1}, {1, 1}};
  for (auto& o : O) canvas.drawString(s, x + o[0], y + o[1]);
  canvas.drawString(s, x + 1, y + 2);
  canvas.setTextColor(ink);
  canvas.drawString(s, x, y);
}

static void score(long v, int x, int y, textdatum_t d = middle_center) { outline(String(v), x, y, F32, INK, d); }

static void bestLabel(long best, int x, int y, textdatum_t d, bool ms = false) {
  if (best <= 0) return;
  outline(String("BEST ") + best + (ms ? " ms" : ""), x, y, F9, DIM, d);
}

// StickyBot: the stick with the agent's eyes, on two legs (StickyWorld's bot, simpler).
// k: scale (1 = 28 x 36 px). walk: the legs' phase (NAN = standing). wings: arms out at that
// angle (NAN = none).
static void bot(float cx, float footY, float k, Mood mood, float walk, uint8_t acc, float look = 0, float wings = NAN,
                float squash = 1) {
  const float bw = 28 * k / squash, bh = 32 * k * squash, legH = 4 * k;
  const float bottom = footY - legH, top = bottom - bh, left = cx - bw / 2;
  const float swing = isnan(walk) ? 0 : sinf(walk) * 1.8f * k;
  canvas.fillRoundRect(cx - 8 * k, bottom + (swing > 0 ? -swing : 0), 5 * k, legH + fabsf(swing), 2, LEG);
  canvas.fillRoundRect(cx + 3 * k, bottom + (swing < 0 ? swing : 0), 5 * k, legH + fabsf(swing), 2, LEG);
  if (!isnan(wings)) {  // little arms, flapping
    for (int side = -1; side <= 1; side += 2) {
      const float sx = cx + side * bw / 2, sy = top + bh * 0.55f;
      canvas.drawWideLine(sx, sy, sx + side * 7 * k * cosf(wings), sy - 7 * k * sinf(wings), 1.6f * k, SHELL_RIM);
    }
  }
  canvas.fillRoundRect(left, top, bw, bh, 7 * k, SHELL);
  canvas.drawRoundRect(left, top, bw, bh, 7 * k, SHELL_RIM);
  const float pw = bw - 6 * k, ph = bh * 0.58f, py = top + 3 * k;
  canvas.fillRoundRect(left + 3 * k, py, pw, ph, 4 * k, SCREEN);
  canvas.fillCircle(cx, top + bh * 0.8f, 2.6f * k, blend(SHELL, AGENT, 0.55f));
  lookAt(look);
  eyes(mood, cx, py + ph / 2, 0.19f * k, moodColor(mood), SCREEN);
  lookAt(NAN);
  if (acc & pet::CROWN) {
    const float cw = bw * 0.55f, cb = top - 1, cl = cx - cw / 2;
    canvas.fillRect(cl, cb - 3 * k, cw, 3 * k, GOLD);
    for (int i = 0; i < 3; i++) {
      const float tx = cl + cw * (i + 0.5f) / 3;
      canvas.fillTriangle(tx - 3 * k, cb - 3 * k, tx + 3 * k, cb - 3 * k, tx, cb - 8 * k, GOLD);
    }
  } else if (acc & pet::HAT) {
    const float hw = bw * 0.42f, brim = bw * 0.72f, hb = top + 1;
    canvas.fillRoundRect(cx - brim / 2, hb - 2 * k, brim, 2.5f * k, 1, rgb(0x22, 0x24, 0x2a));
    canvas.fillRect(cx - hw / 2, hb - 10 * k, hw, 8.5f * k, rgb(0x22, 0x24, 0x2a));
    canvas.fillRect(cx - hw / 2, hb - 4.5f * k, hw, 1.8f * k, WISPR);
  }
}

// --- Small icons for the picker (16 x 16 around cx, cy) ---------------------------------------

static void icon(Id g, int cx, int cy, uint16_t c) {
  switch (g) {
    case Id::STACK:
      for (int i = 0; i < 3; i++) canvas.fillRoundRect(cx - 7 + i * 2 - (i == 1 ? 3 : 0), cy + 4 - i * 5, 12, 4, 1, blend(c, INK, i * 0.2f));
      break;
    case Id::FLAPPY:
      canvas.fillRect(cx + 3, cy - 8, 5, 6, ui::OK);
      canvas.fillRect(cx + 3, cy + 4, 5, 5, ui::OK);
      canvas.fillCircle(cx - 4, cy, 3, SHELL);
      break;
    case Id::DINO:
      canvas.fillRoundRect(cx - 7, cy - 6, 7, 9, 2, SHELL);
      canvas.fillRect(cx + 3, cy, 4, 6, ERR);
      canvas.drawFastHLine(cx - 8, cy + 7, 16, DIM);
      break;
    case Id::REACTION:
      canvas.fillCircle(cx, cy, 7, ui::OK);
      canvas.fillArc(cx, cy, 7, 0, 90, 270, ERR);
      break;
    case Id::MAZE:
      canvas.drawRect(cx - 7, cy - 7, 15, 15, c);
      canvas.drawFastVLine(cx - 2, cy - 7, 10, c);
      canvas.drawFastHLine(cx - 2, cy + 2, 7, c);
      canvas.fillCircle(cx + 4, cy - 3, 2, INK);
      break;
    case Id::BALANCE:
      canvas.drawWideLine(cx - 8, cy + 2, cx + 8, cy - 1, 1.2f, c);
      canvas.fillTriangle(cx - 3, cy + 7, cx + 3, cy + 7, cx, cy + 2, DIM);
      canvas.fillCircle(cx + 2, cy - 4, 3, SHELL);
      break;
    case Id::CATCH:
      canvas.fillCircle(cx - 3, cy - 5, 3, GOLD);
      canvas.fillRoundRect(cx - 7, cy + 2, 14, 5, 1, rgb(0xb0, 0x7a, 0x44));
      break;
    default: break;
  }
}

// --- Picker ---------------------------------------------------------------------------------

void picker(int selected, const long bests[COUNT], uint32_t now) {
  canvas.fillScreen(GROUND);
  canvas.fillRect(0, 0, W, BAR_H, PANEL);
  txt("Games", PAD, BAR_H / 2, F14, INK);
  bot(W - 14, BAR_H - 2, 0.42f, Mood::IDLE, NAN, 0, 4);
  constexpr int TOP = BAR_H + 3, ROW = 26;
  for (int i = 0; i < COUNT; i++) {
    const Id g = (Id)i;
    const int y = TOP + i * ROW;
    const bool sel = i == selected;
    if (sel) {
      canvas.fillRoundRect(3, y + 1, W - 6, ROW - 2, 6, CHIP);
      canvas.fillRoundRect(3, y + 6, 3, ROW - 12, 1, accent(g));
    }
    icon(g, 18, y + ROW / 2, accent(g));
    txt(info(g).title, 32, y + ROW / 2 - 1, F12, sel ? INK : DIM);
    if (info(g).tilt) {  // a little "tilt" mark
      canvas.setFont(F12);
      const int x = 32 + canvas.textWidth(info(g).title) + 5;
      canvas.drawWideLine(x, y + ROW / 2 + 2, x + 7, y + ROW / 2 - 2, 0.8f, FAINT);
    }
    const String b = bests[i] > 0 ? String(bests[i]) + (info(g).lowerBetter ? "ms" : "") : String("-");
    txt(b, W - 9, y + ROW / 2 - 1, F12, bests[i] > 0 ? (sel ? accent(g) : DIM) : FAINT, middle_right);
  }
  const Id g = (Id)selected;
  txt(ellipsize(info(g).how, F9, W - 8), W / 2, H - 25, F9, DIM, middle_center);
  txt("A next  B play  hold B back", W / 2, H - 9, F9, FAINT, middle_center);
}

// --- Backdrops ------------------------------------------------------------------------------

static void paintStack(G& g, uint32_t) { vgrad(g, 0, g.height(), rgb(0x10, 0x12, 0x2a), rgb(0x3a, 0x22, 0x48)); }

static void paintFlappy(G& g, uint32_t) {
  vgrad(g, 0, Flappy::GROUND, rgb(0x4a, 0xa8, 0xd8), rgb(0xb8, 0xe6, 0xf0));
  // a far city, light against the sky
  Rng r(7);
  for (int x = 0; x < g.width();) {
    const int w = r.range(10, 20), h = r.range(18, 46);
    g.fillRect(x, Flappy::GROUND - h, w - 1, h, rgb(0x9c, 0xcf, 0xe0));
    x += w;
  }
  g.fillRect(0, Flappy::GROUND, g.width(), g.height() - Flappy::GROUND, rgb(0xd9, 0xc0, 0x7a));
  g.fillRect(0, Flappy::GROUND, g.width(), 3, rgb(0x5d, 0xb0, 0x48));
}

static void paintDino(G& g, uint32_t) {
  vgrad(g, 0, Dino::GROUND, rgb(0x1a, 0x1e, 0x3a), rgb(0xe0, 0x8a, 0x5a));
  g.fillCircle(186, 62, 15, rgb(0xff, 0xd8, 0x7a));  // the sun, setting
  Rng r(3);
  for (int x = -10; x < g.width(); x += 26) {  // hills
    const int h = r.range(14, 30);
    g.fillTriangle(x, Dino::GROUND, x + 30, Dino::GROUND - h, x + 60, Dino::GROUND, rgb(0x5a, 0x34, 0x4a));
  }
  g.fillRect(0, Dino::GROUND, g.width(), g.height() - Dino::GROUND, rgb(0x2a, 0x1c, 0x24));
  g.drawFastHLine(0, Dino::GROUND, g.width(), rgb(0xf0, 0xb0, 0x80));
}

static void paintBalance(G& g, uint32_t) {
  vgrad(g, 0, g.height(), rgb(0x12, 0x10, 0x26), rgb(0x2c, 0x1e, 0x46));
  Rng r(11);
  for (int i = 0; i < 26; i++) g.drawPixel(r.range(0, g.width() - 1), r.range(0, 70), blend(INK, GROUND, r.unit() * 0.6f));
  const int cx = Balance::CX, cy = Balance::CY;
  g.fillTriangle(cx - 14, g.height() - 8, cx + 14, g.height() - 8, cx, cy + 6, rgb(0x4a, 0x40, 0x66));
  g.fillRect(0, g.height() - 8, g.width(), 8, rgb(0x20, 0x1a, 0x30));
}

static void paintCatch(G& g, uint32_t) {
  vgrad(g, 0, g.height(), rgb(0x08, 0x10, 0x24), rgb(0x1e, 0x2a, 0x4a));
  Rng r(5);
  for (int i = 0; i < 40; i++) g.drawPixel(r.range(0, g.width() - 1), r.range(20, 200), blend(INK, GROUND, r.unit() * 0.7f));
  g.fillRect(0, 226, g.width(), g.height() - 226, rgb(0x1a, 0x30, 0x22));
}

static const Maze* paintingMaze = nullptr;
static void paintMaze(G& g, uint32_t) {
  g.fillScreen(GROUND);
  const Maze& m = *paintingMaze;
  const MazeGrid& gr = m.grid();
  const float cs = m.cellPx(), ox = m.originX(), oy = m.originY();
  g.fillRect(ox, oy, gr.cols * cs, gr.rows * cs, rgb(0x16, 0x1d, 0x22));
  const uint16_t wc = blend(AGENT, INK, 0.35f);
  for (int r = 0; r < gr.rows; r++)
    for (int c = 0; c < gr.cols; c++) {
      const int x = ox + c * cs, y = oy + r * cs;
      if (gr.wall(c, r, MazeGrid::N)) g.fillRect(x - 1, y - 1, cs + 2, 2, wc);
      if (gr.wall(c, r, MazeGrid::W)) g.fillRect(x - 1, y - 1, 2, cs + 2, wc);
      if (c == gr.cols - 1 && gr.wall(c, r, MazeGrid::E)) g.fillRect(x + cs - 1, y - 1, 2, cs + 2, wc);
      if (r == gr.rows - 1 && gr.wall(c, r, MazeGrid::S)) g.fillRect(x - 1, y + cs - 1, cs + 2, 2, wc);
    }
  // the start
  g.fillRoundRect(ox + 3, oy + 3, cs - 6, cs - 6, 3, rgb(0x22, 0x2c, 0x34));
}

// --- Title card, calibration ------------------------------------------------------------------

static void gameBackdrop(Id g) {
  switch (g) {
    case Id::STACK: backdrop(1, paintStack); break;
    case Id::FLAPPY: backdrop(2, paintFlappy); break;
    case Id::DINO: backdrop(3, paintDino); break;
    case Id::BALANCE: backdrop(6, paintBalance); break;
    case Id::CATCH: backdrop(7, paintCatch); break;
    default: canvas.fillScreen(GROUND); break;
  }
}

static void card(Id g, int& y0, int& h) {
  const bool wide = W > H;
  const int w = W - 16;
  h = wide ? H - 12 : 176;
  y0 = wide ? 6 : (H - h) / 2 - 6;
  canvas.fillRoundRect(8, y0, w, h, 10, PANEL);
  canvas.drawRoundRect(8, y0, w, h, 10, blend(PANEL, accent(g), 0.6f));
}

void ready(Id g, const Ctx& c) {
  gameBackdrop(g);
  dim();
  int y0, h;
  card(g, y0, h);
  const bool wide = W > H;
  icon(g, W / 2, y0 + (wide ? 13 : 18), accent(g));
  txt(info(g).title, W / 2, y0 + (wide ? 34 : 42), F24, INK, middle_center);
  String lines[3];
  const int n = wrap(info(g).how, F12, lines, wide ? 2 : 3, W - 36);
  for (int i = 0; i < n; i++) txt(lines[i], W / 2, y0 + (wide ? 56 : 66) + i * 16, F12, DIM, middle_center);
  const int by = y0 + h - (wide ? 30 : 20);
  if (c.best > 0)
    txt(String("Best ") + c.best + (info(g).lowerBetter ? " ms" : ""), W / 2, by - (wide ? 18 : 22), F12, accent(g),
        middle_center);
  const bool pulse = (c.now / 500) % 2;
  txt(info(g).tilt ? "Lay it flat, then A" : "A to start", W / 2, by, F12, pulse ? INK : DIM, middle_center);
  txt("hold B: back", W / 2, wide ? y0 + h - 11 : y0 + h + 12, F9, FAINT, middle_center);
}

void calibrate(Id g, uint32_t msLeft, float tx, float ty, const Ctx& c) {
  canvas.fillScreen(GROUND);
  const bool wide = W > H;
  const int cx = W / 2, cy = wide ? H / 2 + 8 : H / 2 - 10;
  // a bubble level: the bubble sits in the middle when it's still
  const int R = wide ? 40 : 46;
  canvas.drawCircle(cx, cy, R, LINE);
  canvas.drawCircle(cx, cy, R / 4, FAINT);
  canvas.drawFastHLine(cx - R, cy, 2 * R, LINE);
  canvas.drawFastVLine(cx, cy - R, 2 * R, LINE);
  const float bx = constrain(tx * R * 2.5f, -R + 7.0f, R - 7.0f), by = constrain(ty * R * 2.5f, -R + 7.0f, R - 7.0f);
  canvas.fillCircle(cx + bx, cy + by, 7, accent(g));
  const int n = (int)(msLeft + 999) / 1000;
  outline(String(n < 1 ? 1 : n), wide ? 30 : cx, wide ? cy : cy - R - 26, F44, INK);
  txt("Hold it still", cx, wide ? 12 : cy + R + 18, F12, DIM, middle_center);
  txt(info(g).title, cx, wide ? H - 10 : cy + R + 36, F9, FAINT, middle_center);
  (void)c;
}

// --- Stack ----------------------------------------------------------------------------------

static uint16_t hue(int level) {
  static const uint16_t P[] = {rgb(0xff, 0x9f, 0x1c), rgb(0xff, 0xc9, 0x4d), rgb(0x3d, 0xdc, 0x84), rgb(0x2e, 0xc4, 0xb6),
                               rgb(0x5a, 0xa9, 0xff), rgb(0x9b, 0x7b, 0xff), rgb(0xff, 0x6b, 0x9a), rgb(0xff, 0x52, 0x52)};
  const int n = sizeof P / sizeof P[0], i = level / 3;
  return blend(P[i % n], P[(i + 1) % n], (level % 3) / 3.0f);
}

static void block(float x, float y, float w, uint16_t c) {
  canvas.fillRect(x, y, w, Stack::BLOCK_H - 1, c);
  canvas.drawFastHLine(x, y, w, blend(c, TFT_WHITE, 0.45f));
  canvas.drawFastHLine(x, y + Stack::BLOCK_H - 2, w, blend(c, TFT_BLACK, 0.35f));
}

void stack(const Stack& s, const Ctx& c) {
  backdrop(1, paintStack);
  constexpr int BH = Stack::BLOCK_H, BASE_Y = 214;  // the base block's top, before the tower scrolls
  const int h = s.height();
  const int lift = h > 9 ? (h - 9) * BH : 0;        // keeps the top of the tower around y 106
  auto yOf = [&](int level) { return BASE_Y - level * BH + lift; };
  for (int lv = h; lv >= 0 && lv > h - Stack::KEEP + 1; lv--) {
    const int y = yOf(lv);
    if (y > H) break;
    const Stack::Block& b = s.block(lv);
    block(b.x, y, b.w, hue(lv));
  }
  if (yOf(0) + BH < H) canvas.fillRect(0, yOf(0) + BH, W, H - yOf(0) - BH, rgb(0x14, 0x10, 0x20));
  // a perfect drop: the top block flashes
  const Stack::Block& top = s.block(h);
  if (s.lastDrop() == Stack::Drop::PERFECT && s.lastDropMs() < 450 && h > 0) {
    const float k = 1 - s.lastDropMs() / 450.0f;
    canvas.drawRect(top.x - 2, yOf(h) - 2, top.w + 4, BH + 3, blend(GROUND, TFT_WHITE, k));
    outline(s.combo() > 1 ? String("Perfect x") + s.combo() : String("Perfect"), W / 2, yOf(h) - 44 - (int)(12 * (1 - k)),
            F12, blend(SUN, INK, 1 - k));
  }
  // StickyBot rides the top of the tower (hops when a block lands)
  const float hop = s.lastDropMs() < 300 ? sinf(PI * s.lastDropMs() / 300.0f) * 6 : 0;
  const Mood mood = s.over ? Mood::ERROR : s.lastDrop() == Stack::Drop::PERFECT && s.lastDropMs() < 900 ? Mood::DONE : Mood::IDLE;
  if (!s.over) bot(top.x + top.w / 2, yOf(h) - hop, 0.42f, mood, NAN, c.acc, (s.x() + s.w() / 2 - top.x - top.w / 2) / 8);
  // the sliding one, and what was cut off
  if (!s.over) block(s.x(), yOf(h + 1) - 16, s.w(), hue(h + 1));
  if (s.debris()) {
    const uint16_t dc = blend(hue(s.debLevel()), GROUND, constrain(s.debFall() / 160.0f, 0.0f, 0.8f));
    block(s.debX(), yOf(s.debLevel()) + s.debFall() - (s.over ? 16 : 0), s.debW(), dc);
  }
  if (s.over) bot(top.x + top.w / 2, yOf(h), 0.42f, mood, NAN, c.acc);
  score(s.score, W / 2, 22);
  bestLabel(c.best, W - 4, 6, top_right);
}

// --- Flappy ---------------------------------------------------------------------------------

void flappy(const Flappy& f, const Ctx& c) {
  backdrop(2, paintFlappy);
  const float sc = f.scroll();
  // two slow clouds
  for (int i = 0; i < 2; i++) {
    const float x = fmodf(200 - sc * 0.25f + i * 90, W + 60) - 30, y = 40 + i * 36;
    canvas.fillRoundRect(x, y, 30, 9, 4, rgb(0xee, 0xf8, 0xfb));
    canvas.fillCircle(x + 11, y + 1, 6, rgb(0xee, 0xf8, 0xfb));
  }
  const uint16_t pipe = rgb(0x48, 0xb0, 0x3c), pipeDk = rgb(0x2c, 0x76, 0x26), pipeLt = rgb(0x8c, 0xe0, 0x6a);
  for (int i = 0; i < Flappy::PIPES; i++) {
    const Flappy::Pipe& p = f.pipe(i);
    if (p.x > W || p.x + Flappy::PIPE_W < 0) continue;
    const float top = p.gapY - p.gapH / 2, bot_ = p.gapY + p.gapH / 2;
    const int pw = Flappy::PIPE_W;
    canvas.fillRect(p.x + 2, 0, pw - 4, top, pipe);
    canvas.fillRect(p.x + 2, bot_, pw - 4, Flappy::GROUND - bot_, pipe);
    canvas.fillRect(p.x + 4, 0, 3, top, pipeLt);
    canvas.fillRect(p.x + 4, bot_, 3, Flappy::GROUND - bot_, pipeLt);
    canvas.fillRect(p.x + pw - 6, 0, 3, top, pipeDk);
    canvas.fillRect(p.x + pw - 6, bot_, 3, Flappy::GROUND - bot_, pipeDk);
    canvas.fillRect(p.x, top - 8, pw, 8, pipe);  // the caps
    canvas.drawRect(p.x, top - 8, pw, 8, pipeDk);
    canvas.fillRect(p.x, bot_, pw, 8, pipe);
    canvas.drawRect(p.x, bot_, pw, 8, pipeDk);
  }
  // the ground's stripes move with the pipes
  for (int x = -(int)fmodf(sc, 12); x < W; x += 12)
    canvas.fillTriangle(x, Flappy::GROUND + 3, x + 6, Flappy::GROUND + 3, x + 2, Flappy::GROUND + 9, rgb(0xc4, 0xa8, 0x62));
  // StickyBot, flapping
  const float wing = f.over ? -0.6f : f.vy() < 0 ? 0.9f * sinf(f.t / 40.0f) : -0.3f;
  const float squash = f.over ? 1 : constrain(1 - f.vy() / 2500.0f, 0.88f, 1.12f);
  const Mood m = f.over ? Mood::ERROR : f.vy() < -120 ? Mood::DONE : Mood::IDLE;
  bot(Flappy::BIRD_X, f.y() + 9, 0.44f, m, NAN, c.acc, 5, wing, squash);
  if (!f.started()) outline("Tap A", W / 2, f.y() - 30, F14, INK);
  score(f.score, W / 2, 24);
  bestLabel(c.best, W - 4, 6, top_right);
}

// --- Dino -----------------------------------------------------------------------------------

static void bug(float x, float y, float w, float h, uint32_t now) {  // y: the bottom
  const uint16_t body = rgb(0x8c, 0x3c, 0xd0), dark = rgb(0x4c, 0x1c, 0x78);
  const float legs = (now / 90) % 2 ? 1 : -1;
  for (float lx = x + 2; lx < x + w - 1; lx += 4) canvas.drawFastVLine(lx + legs, y - 3, 3, dark);
  canvas.fillRoundRect(x, y - h, w, h - 2, w > 8 ? 4 : 2, body);
  canvas.drawRoundRect(x, y - h, w, h - 2, w > 8 ? 4 : 2, dark);
  canvas.fillRect(x + 2, y - h + 3, 2, 2, TFT_WHITE);
  canvas.fillRect(x + w - 5, y - h + 3, 2, 2, TFT_WHITE);
}

static void drone(float x, float y, float w, float h, uint32_t now) {  // y: the bottom
  canvas.fillRoundRect(x, y - h + 3, w, h - 3, 3, rgb(0x6c, 0x76, 0x84));
  canvas.fillCircle(x + w / 2, y - h / 2 + 1, 2, ERR);
  const float p = ((now / 40) % 2) ? 6 : 3;
  canvas.drawFastHLine(x + 1 - p / 2, y - h + 1, 6 + p, DIM);
  canvas.drawFastHLine(x + w - 7 - p / 2, y - h + 1, 6 + p, DIM);
}

void dino(const Dino& d, const Ctx& c) {
  backdrop(3, paintDino);
  const float dist = d.distance();
  for (int x = -(int)fmodf(dist, 18); x < W; x += 18) canvas.drawFastHLine(x, Dino::GROUND + 6, 6, rgb(0x6a, 0x40, 0x48));
  for (int x = -(int)fmodf(dist * 0.6f, 31); x < W; x += 31) canvas.drawPixel(x, Dino::GROUND + 13, rgb(0x6a, 0x40, 0x48));
  for (int i = 0; i < Dino::OBST; i++) {
    const Dino::Obstacle& o = d.obstacle(i);
    if (!o.active) continue;
    if (o.fly) drone(o.x, Dino::GROUND - o.lift, o.w, o.h, d.t);
    else bug(o.x, Dino::GROUND, o.w, o.h, d.t);
  }
  const bool air = !d.onGround();
  const Mood m = d.over ? Mood::ERROR : air ? Mood::DONE : Mood::IDLE;
  bot(Dino::BOT_X, Dino::GROUND - d.height() + 1, 0.62f, m, d.over || air ? NAN : dist / 7.0f, c.acc, 7);
  char buf[12];
  snprintf(buf, sizeof buf, "%05ld", d.score);
  outline(buf, W - 6, 13, F18, INK, middle_right);
  if (c.best > 0) {
    snprintf(buf, sizeof buf, "HI %05ld", c.best);
    outline(buf, W - 6, 31, F9, DIM, middle_right);
  }
}

// --- Reaction -------------------------------------------------------------------------------

void reaction(const Reaction& r, const Ctx& c) {
  using P = Reaction::Phase;
  const P p = r.phase();
  const bool go = p == P::GO;
  const uint16_t bgc = go ? rgb(0x1e, 0xa8, 0x56) : p == P::FALSE_START ? rgb(0xc8, 0x6a, 0x10)
                       : p == P::WAIT ? rgb(0x8a, 0x16, 0x22) : rgb(0x16, 0x26, 0x2e);
  canvas.fillScreen(bgc);
  const int cx = W / 2;
  if (p == P::WAIT) {
    eyes(Mood::ASK, cx, 92, 0.6f, INK, bgc);
    txt("Wait...", cx, 160, F24, INK, middle_center);
    txt("for green", cx, 184, F12, blend(INK, bgc, 0.35f), middle_center);
  } else if (go) {
    eyes(Mood::LISTEN, cx, 92, 0.7f, INK, bgc);
    txt("NOW!", cx, 168, F32, INK, middle_center);
  } else if (p == P::FALSE_START) {
    eyes(Mood::ERROR, cx, 92, 0.6f, INK, bgc);
    txt("Too soon!", cx, 166, F24, INK, middle_center);
  } else {
    const int ms = r.time(r.round());
    eyes(ms < 250 ? Mood::PARTY : Mood::DONE, cx, 80, 0.5f, ms < 250 ? SUN : AGENT, bgc);
    outline(String(ms), cx, 146, F44, INK);
    txt("ms", cx, 178, F14, DIM, middle_center);
  }
  for (int i = 0; i < Reaction::ROUNDS; i++) {  // the rounds, done ones filled
    const int x = cx + (i - 1) * 18, y = 214;
    if (r.time(i) > 0) canvas.fillCircle(x, y, 5, INK);
    else canvas.drawCircle(x, y, 5, blend(INK, bgc, i == r.round() ? 0.2f : 0.6f));
  }
  if (c.best > 0) txt(String("best ") + c.best + " ms", cx, 14, F9, blend(INK, bgc, 0.35f), middle_center);
}

// --- Maze -----------------------------------------------------------------------------------

void maze(const Maze& m, const Ctx& c) {
  paintingMaze = &m;
  backdrop(0x40000000u | (m.seed() & 0x3fffffffu), paintMaze);
  const MazeGrid& g = m.grid();
  const float cs = m.cellPx();
  // the exit glows
  const float ex = m.originX() + (g.cols - 0.5f) * cs, ey = m.originY() + (g.rows - 0.5f) * cs;
  const float pulse = 0.5f + 0.5f * sinf(c.now / 220.0f);
  canvas.fillCircle(ex, ey, cs * 0.36f, blend(rgb(0x16, 0x1d, 0x22), ui::OK, 0.35f + 0.4f * pulse));
  canvas.fillCircle(ex, ey, cs * 0.18f, ui::OK);
  // the ball: a shadow, the ball, a shine
  const float bx = m.ballX(), by = m.ballY(), r = m.ballR();
  canvas.fillCircle(bx + 1, by + 2, r, rgb(0x08, 0x0a, 0x0c));
  canvas.fillCircle(bx, by, r, SHELL);
  canvas.fillCircle(bx - r * 0.3f, by - r * 0.3f, r * 0.35f, TFT_WHITE);
  // HUD
  txt(String("Level ") + m.level(), 6, 11, F12, INK);
  const uint32_t s = (m.timeLeft() + 999) / 1000;
  char buf[8];
  snprintf(buf, sizeof buf, "%u:%02u", (unsigned)(s / 60), (unsigned)(s % 60));
  txt(buf, W - 6, 11, F14, s <= 10 && (c.now / 300) % 2 ? ERR : s <= 10 ? ASK : INK, middle_right);
  if (m.levelMs() < 900 && m.level() > 1) outline(String("Level ") + m.level() + "!", W / 2, H / 2, F24, ui::OK);
}

// --- Balance --------------------------------------------------------------------------------

void balance(const Balance& b, const Ctx& c) {
  backdrop(6, paintBalance);
  const float a = b.angle(), ca = cosf(a), sa = sinf(a);
  const float cx = Balance::CX, cy = Balance::CY, L = Balance::HALF + 8;
  // the beam, its middle zone, the ends
  canvas.drawWideLine(cx - ca * L, cy - sa * L, cx + ca * L, cy + sa * L, 3.0f, rgb(0xa0, 0x8c, 0xd8));
  const float z = Balance::ZONE + Balance::R * 0.4f;
  canvas.drawWideLine(cx - ca * z, cy - sa * z, cx + ca * z, cy + sa * z, 3.0f, b.inZone() ? ui::OK : blend(ui::OK, GROUND, 0.4f));
  canvas.fillCircle(cx, cy, 4, rgb(0x4a, 0x40, 0x66));
  // the wind: streaks blowing the way the beam leans
  for (int i = 0; i < 4; i++) {
    const float ph = fmodf(c.now / 600.0f + i * 0.25f, 1.0f), y = 30 + i * 13;
    const float x = a > 0 ? ph * W : W - ph * W, len = fabsf(a) * 120;
    if (len > 3) canvas.drawFastHLine(a > 0 ? x - len : x, y, len, blend(GROUND, DIM, 0.5f));
  }
  // the ball, a StickyBot head, on the beam (or falling off it)
  const float s = b.pos(), r = Balance::R;
  const float bx = cx + ca * s + sa * (r + 2), by = cy + sa * s - ca * (r + 2) + b.fall();  // on top of it
  canvas.fillCircle(bx, by, r, SHELL);
  canvas.fillRoundRect(bx - r * 0.65f, by - r * 0.45f, r * 1.3f, r * 0.9f, 3, SCREEN);
  const float roll = s / r;  // the eyes roll with it
  canvas.fillRect(bx - 3 + sinf(roll) * 1.5f, by - 2, 2, 4, b.over ? ERR : AGENT);
  canvas.fillRect(bx + 2 + sinf(roll) * 1.5f, by - 2, 2, 4, b.over ? ERR : AGENT);
  score(b.score, W / 2, 18);
  bestLabel(c.best, W - 4, 6, top_right);
  const uint32_t secs = b.t / 1000;
  txt(String(secs) + "s", 6, 10, F9, FAINT);
}

// --- Catch ----------------------------------------------------------------------------------

static void star5(float cx, float cy, float ro, float ri, float rot, uint16_t c) {
  for (int i = 0; i < 5; i++) {
    const float a0 = rot + i * 1.2566f, a1 = a0 + 0.6283f, a2 = a0 + 1.2566f;
    canvas.fillTriangle(cx, cy, cx + cosf(a0) * ro, cy + sinf(a0) * ro, cx + cosf(a1) * ri, cy + sinf(a1) * ri, c);
    canvas.fillTriangle(cx, cy, cx + cosf(a1) * ri, cy + sinf(a1) * ri, cx + cosf(a2) * ro, cy + sinf(a2) * ro, c);
  }
}

void catcher(const Catch& k, const Ctx& c) {
  backdrop(7, paintCatch);
  if (k.sinceHit() < 250) canvas.fillRect(0, 0, W, H, blend(GROUND, ERR, 0.25f * (1 - k.sinceHit() / 250.0f)));
  for (int i = 0; i < Catch::ITEMS; i++) {
    const Catch::Item& it = k.item(i);
    if (!it.active) continue;
    const float r = Catch::ITEM_R;
    switch (it.kind) {
      case Catch::Kind::COIN:
        canvas.fillCircle(it.x, it.y, r, GOLD);
        canvas.drawCircle(it.x, it.y, r - 2, rgb(0xc0, 0x8a, 0x20));
        canvas.drawFastVLine(it.x, it.y - 2, 4, rgb(0xc0, 0x8a, 0x20));
        break;
      case Catch::Kind::STAR:
        star5(it.x, it.y, r + 2, r * 0.45f, it.y / 20.0f, rgb(0xff, 0xf0, 0x90));
        break;
      case Catch::Kind::BOMB:
        canvas.fillCircle(it.x, it.y + 1, r, rgb(0x2a, 0x2e, 0x36));
        canvas.drawCircle(it.x, it.y + 1, r, rgb(0x60, 0x68, 0x74));
        canvas.drawLine(it.x + 2, it.y - r + 1, it.x + 4, it.y - r - 3, DIM);
        canvas.fillCircle(it.x + 4, it.y - r - 4, (k.t / 80) % 2 ? 2 : 1, (k.t / 80) % 2 ? ASK : ERR);
        break;
    }
  }
  // StickyBot holds the basket up
  const float x = k.basketX(), bw = Catch::BASKET_W, by = Catch::BASKET_Y;
  const Mood m = k.over ? Mood::ERROR : k.sinceHit() < 600 ? Mood::ERROR : k.sinceCatch() < 400 ? Mood::DONE : Mood::IDLE;
  bot(x, 240, 0.5f, m, fabsf(k.basketV()) > 30 ? k.t / 60.0f : NAN, 0, k.basketV() / 40);
  canvas.drawWideLine(x - 8, 222, x - bw / 2 + 4, by + 9, 1.2f, SHELL_RIM);
  canvas.drawWideLine(x + 8, 222, x + bw / 2 - 4, by + 9, 1.2f, SHELL_RIM);
  const uint16_t wood = rgb(0xb0, 0x7a, 0x44), woodDk = rgb(0x7a, 0x50, 0x2a);
  canvas.fillTriangle(x - bw / 2, by, x + bw / 2, by, x + bw / 2 - 5, by + 10, wood);
  canvas.fillTriangle(x - bw / 2, by, x + bw / 2 - 5, by + 10, x - bw / 2 + 5, by + 10, wood);
  for (int i = 1; i < 4; i++) canvas.drawFastVLine(x - bw / 2 + i * bw / 4, by + 1, 8, woodDk);
  canvas.drawFastHLine(x - bw / 2, by, bw, rgb(0xe0, 0xa8, 0x6a));
  if (k.sinceCatch() < 400) outline("+", x + bw / 2 + 4, by - 8 - k.sinceCatch() / 30, F14, SUN);
  // hearts
  for (int i = 0; i < Catch::LIVES; i++) {
    const int hx = 8 + i * 13, hy = 10;
    const uint16_t hc = i < k.lives() ? ERR : LINE;
    canvas.fillCircle(hx - 2, hy - 1, 3, hc);
    canvas.fillCircle(hx + 2, hy - 1, 3, hc);
    canvas.fillTriangle(hx - 5, hy, hx + 5, hy, hx, hy + 6, hc);
  }
  score(k.score, W / 2, 22);
  bestLabel(c.best, W - 4, 6, top_right);
  (void)c;
}

// --- Overlays -------------------------------------------------------------------------------

void pause(int selected) {
  dim();
  const int w = W > H ? 150 : 115, h = 112, x = (W - w) / 2, y = (H - h) / 2;
  canvas.fillRoundRect(x, y, w, h, 10, PANEL);
  canvas.drawRoundRect(x, y, w, h, 10, LINE);
  txt("Paused", W / 2, y + 15, F14, INK, middle_center);
  static const char* ITEMS[PAUSE_ITEMS] = {"Resume", "Restart", "Quit"};
  for (int i = 0; i < PAUSE_ITEMS; i++) {
    const int ry = y + 30 + i * 22;
    if (i == selected) {
      canvas.fillRoundRect(x + 6, ry, w - 12, 20, 6, CHIP);
      canvas.fillRoundRect(x + 6, ry + 5, 3, 10, 1, WISPR);
    }
    txt(ITEMS[i], W / 2, ry + 10, F12, i == selected ? INK : DIM, middle_center);
  }
  txt("A next  B pick", W / 2, y + h - 10, F9, FAINT, middle_center);
}

void overConfetti() { confettiReset(W / 2, H / 3); }

void over(Id g, long sc, long best, bool newBest, bool failed, uint32_t sinceMs) {
  dim();
  const bool wide = W > H;
  const int w = wide ? 180 : 119, h = wide ? 116 : 150, x = (W - w) / 2, y = (H - h) / 2;
  canvas.fillRoundRect(x, y, w, h, 10, PANEL);
  canvas.drawRoundRect(x, y, w, h, 10, newBest ? SUN : LINE);
  const bool ms = info(g).lowerBetter;
  txt(failed ? "Too soon!" : newBest ? "New best!" : ms ? "Your time" : "Game over", W / 2, y + 16, F14, newBest ? SUN : failed ? ASK : INK,
      middle_center);
  if (failed) {
    txt("Wait for green", W / 2, y + 50, F12, DIM, middle_center);
  } else {
    outline(String(sc), W / 2, y + (wide ? 50 : 56), F44, newBest ? SUN : INK);
    if (ms) txt("ms average", W / 2, y + (wide ? 76 : 84), F9, DIM, middle_center);
  }
  if (!newBest && best > 0)
    txt(String("best ") + best + (ms ? " ms" : ""), W / 2, y + h - (wide ? 30 : 38), F12, DIM, middle_center);
  const bool ready = sinceMs > 700;  // the presses that ended it don't start the next one
  txt("A again  B games", W / 2, y + h - 12, F9, ready ? INK : FAINT, middle_center);
  if (newBest) confettiDraw(false);
}

}  // namespace draw
}  // namespace games
