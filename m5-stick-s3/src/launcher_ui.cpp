#include "launcher_ui.h"
#include <math.h>
#include "ui_draw.h"
#include "faces/faces.h"

namespace launcherui {

using namespace ui;
using launcher::Quick;
using launcher::Tile;

constexpr uint16_t SHELL = rgb(0xe9, 0xec, 0xef);  // StickyBot (as in StickyWorld and the games)
constexpr uint16_t SCREEN = rgb(0x12, 0x15, 0x19);
constexpr uint16_t GEAR = rgb(0xb8, 0xc0, 0xc8);

static uint16_t accent(Tile t) {
  switch (t) {
    case Tile::GAMES: return AMBER;
    case Tile::REMOTE: return ERR;
    case Tile::AIR: return ui::BLUE;
    case Tile::PRESENTER: return ui::VIOLET;
    case Tile::MEDIA: return ui::OK;
    case Tile::FACES: return AGENT;
    case Tile::QUICK: return SUN;
    default: return GEAR;
  }
}

// --- Icons: drawn in a 24 x 24 box around cx, cy, scaled by k; bg = the tile under them ------

static void ring(float cx, float cy, float r0, float r1, float a0, float a1, uint16_t c) {
  canvas.fillArc(cx, cy, r0, r1, a0, a1, c);
}

static void gear(float cx, float cy, float k, uint16_t c, uint16_t bg) {
  for (int i = 0; i < 8; i++) {
    const float a = i * (float)M_PI / 4;
    canvas.drawWideLine(cx + cosf(a) * 6 * k, cy + sinf(a) * 6 * k, cx + cosf(a) * 10.5f * k, cy + sinf(a) * 10.5f * k,
                        2.1f * k, c);
  }
  canvas.fillCircle(cx, cy, 8 * k, c);
  canvas.fillCircle(cx, cy, 3.4f * k, bg);
}

static void icon(Tile t, int cx, int cy, float k, uint16_t c, uint16_t bg) {
  auto X = [&](float x) { return cx + x * k; };
  auto Y = [&](float y) { return cy + y * k; };
  switch (t) {
    case Tile::GAMES: {  // a game pad
      canvas.fillRoundRect(X(-12), Y(-7), 24 * k, 15 * k, 7 * k, c);
      canvas.fillRect(X(-9), Y(-1.2f), 7 * k, 2.6f * k, bg);
      canvas.fillRect(X(-6.8f), Y(-3.5f), 2.6f * k, 7 * k, bg);
      canvas.fillCircle(X(4.5f), Y(1.5f), 1.9f * k, bg);
      canvas.fillCircle(X(8), Y(-2), 1.9f * k, bg);
      break;
    }
    case Tile::REMOTE: {  // a remote with its IR waves
      canvas.fillRoundRect(X(-5), Y(-4), 10 * k, 17 * k, 3 * k, c);
      canvas.fillCircle(X(0), Y(0), 1.8f * k, bg);
      for (int r = 0; r < 2; r++)
        for (int q = 0; q < 2; q++) canvas.fillRect(X(-3 + q * 4), Y(4 + r * 4), 2 * k, 2 * k, bg);
      ring(X(0), Y(-5), 3.5f * k, 5 * k, 235, 305, c);
      ring(X(0), Y(-5), 7 * k, 8.5f * k, 235, 305, c);
      break;
    }
    case Tile::AIR: {  // a mouse, moving
      canvas.fillRoundRect(X(-6), Y(-10), 12 * k, 20 * k, 6 * k, c);
      canvas.drawWideLine(X(-6), Y(-2.5f), X(6), Y(-2.5f), 0.7f * k, bg);
      canvas.drawWideLine(X(0), Y(-10), X(0), Y(-2.5f), 0.7f * k, bg);
      ring(X(0), Y(0), 9.5f * k, 11 * k, 150, 210, c);
      ring(X(0), Y(0), 9.5f * k, 11 * k, 330, 390, c);
      break;
    }
    case Tile::PRESENTER: {  // a screen on a stand, a chart, the laser's dot
      canvas.fillRoundRect(X(-12), Y(-10), 24 * k, 16 * k, 2 * k, c);
      canvas.fillRect(X(-10), Y(-8), 20 * k, 12 * k, bg);
      canvas.fillRect(X(-7), Y(-1), 3 * k, 4 * k, c);
      canvas.fillRect(X(-2.5f), Y(-4), 3 * k, 7 * k, c);
      canvas.fillCircle(X(5.5f), Y(-3.5f), 1.8f * k, ERR);
      canvas.drawWideLine(X(0), Y(6), X(-5), Y(12), 0.9f * k, c);
      canvas.drawWideLine(X(0), Y(6), X(5), Y(12), 0.9f * k, c);
      break;
    }
    case Tile::MEDIA: {  // play, and the sound
      canvas.fillTriangle(X(-10), Y(-8), X(-10), Y(8), X(3), Y(0), c);
      ring(X(1), Y(0), 5.5f * k, 7.2f * k, 315, 405, c);
      ring(X(1), Y(0), 9.5f * k, 11.2f * k, 315, 405, c);
      break;
    }
    case Tile::FACES: {  // StickyBot's face
      canvas.fillRoundRect(X(-11), Y(-10), 22 * k, 21 * k, 6 * k, SHELL);
      canvas.fillRoundRect(X(-8), Y(-7), 16 * k, 11 * k, 3.5f * k, SCREEN);
      canvas.fillEllipse(X(-3.4f), Y(-1.5f), 1.7f * k, 2.6f * k, c);
      canvas.fillEllipse(X(3.4f), Y(-1.5f), 1.7f * k, 2.6f * k, c);
      canvas.fillCircle(X(0), Y(7.5f), 1.6f * k, blend(SHELL, c, 0.6f));
      break;
    }
    case Tile::QUICK: {  // two switches, one on
      canvas.fillRoundRect(X(-11), Y(-10), 22 * k, 9 * k, 4.5f * k, c);
      canvas.fillCircle(X(6.5f), Y(-5.5f), 3 * k, bg);
      canvas.drawRoundRect(X(-11), Y(1), 22 * k, 9 * k, 4.5f * k, c);
      canvas.drawRoundRect(X(-10), Y(2), 20 * k, 7 * k, 3.5f * k, c);
      canvas.fillCircle(X(-6.5f), Y(5.5f), 2.4f * k, c);
      break;
    }
    default:
      gear(cx, cy, k, c, bg);
      break;
  }
}

// --- Bits ------------------------------------------------------------------------------------

static void bar(const String& title, const String& right) {
  canvas.fillRect(0, 0, W, BAR_H, PANEL);
  txt(title, PAD, BAR_H / 2, F14, INK);
  if (right.length()) txt(right, W - PAD, BAR_H / 2, F12, DIM, middle_right);
}

static void dots(int n, int at, int cx, int cy) {
  if (n <= 1) return;
  const int gap = 9, x = cx - (n - 1) * gap / 2;
  for (int i = 0; i < n; i++) canvas.fillCircle(x + i * gap, cy, i == at ? 3 : 2, i == at ? INK : LINE);
}

static void hint(const char* s) { txt(s, W / 2, H - 8, F9, FAINT, middle_center); }

static void outlineSel(int x, int y, int w, int h, int r, uint16_t c) {
  canvas.drawRoundRect(x, y, w, h, r, c);
  canvas.drawRoundRect(x + 1, y + 1, w - 2, h - 2, r - 1, c);
}

// --- The grid ----------------------------------------------------------------------------------

void drawGrid(const GridView& v) {
  const bool wide = turnToHeldSide();
  const launcher::Grid g = launcher::gridFor(wide);
  const int n = v.layout.n, per = g.perPage(), page = v.sel / per, np = launcher::pages(n, per);
  canvas.fillScreen(GROUND);
  bar("Apps", v.clock);
  canvas.setFont(F14);
  dots(np, page, wide ? W / 2 : PAD + canvas.textWidth("Apps") + 10 + (np - 1) * 9 / 2, BAR_H / 2);

  const int m = 4, gap = 5, top = BAR_H + 4;
  const int bottom = wide ? H - 15 : H - 30;
  const int tw = (W - 2 * m - (g.cols - 1) * gap) / g.cols, th = (bottom - top - (g.rows - 1) * gap) / g.rows;
  const float k = wide ? 0.9f : 1.1f;
  for (int i = page * per; i < n && i < (page + 1) * per; i++) {
    const launcher::Cell c = launcher::cellOf(i, g);
    const Tile t = v.layout.tiles[i];
    const bool sel = i == v.sel;
    const int x = m + c.col * (tw + gap), y = top + c.row * (th + gap);
    const uint16_t ac = accent(t), bg = sel ? blend(PANEL, ac, 0.16f) : PANEL;
    canvas.fillRoundRect(x, y, tw, th, 10, bg);
    if (sel) outlineSel(x, y, tw, th, 10, ac);
    const int iy = y + (wide ? 17 : 23);
    icon(t, x + tw / 2, iy, k, sel ? ac : blend(ac, DIM, 0.25f), bg);
    const char* label = launcher::info(t).label;
    const lgfx::IFont* f = fit(label, tw - 6, {F12, F9});
    txt(label, x + tw / 2, y + th - (wide ? 9 : 11), f, sel ? INK : DIM, middle_center);
  }
  if (!wide) {
    const String about = launcher::info(v.layout.tiles[v.sel < n ? v.sel : 0]).about;
    txt(ellipsize(about, F9, W - 8), W / 2, H - 22, F9, DIM, middle_center);
  }
  hint("A next  B open  hold B home");
  canvas.pushSprite(0, 0);
}

// --- Quick settings ------------------------------------------------------------------------------

static void qicon(Quick q, int cx, int cy, uint16_t c, uint16_t bg) {
  switch (q) {
    case Quick::BRIGHTNESS:
      for (int i = 0; i < 8; i++) {
        const float a = i * (float)M_PI / 4;
        canvas.drawWideLine(cx + cosf(a) * 5.5f, cy + sinf(a) * 5.5f, cx + cosf(a) * 7.5f, cy + sinf(a) * 7.5f, 0.8f, c);
      }
      canvas.fillCircle(cx, cy, 3.6f, c);
      break;
    case Quick::VOLUME:
    case Quick::SOUND:
      canvas.fillRect(cx - 7, cy - 2, 4, 5, c);
      canvas.fillTriangle(cx - 5, cy + 0.5f, cx + 1, cy - 6, cx + 1, cy + 7, c);
      if (q == Quick::VOLUME || c != FAINT) {
        ring(cx + 1, cy + 0.5f, 3.5f, 4.8f, 320, 400, c);
        ring(cx + 1, cy + 0.5f, 6.5f, 7.8f, 320, 400, c);
      } else {  // muted: a cross
        canvas.drawWideLine(cx + 3, cy - 2.5f, cx + 8, cy + 3.5f, 0.8f, c);
        canvas.drawWideLine(cx + 8, cy - 2.5f, cx + 3, cy + 3.5f, 0.8f, c);
      }
      break;
    case Quick::WMIC:
      canvas.fillRoundRect(cx - 3, cy - 8, 6, 10, 3, c);
      ring(cx, cy - 1.5f, 5, 6.3f, 0, 180, c);
      canvas.drawWideLine(cx, cy + 5, cx, cy + 7.5f, 0.7f, c);
      canvas.drawWideLine(cx - 3, cy + 7.5f, cx + 3, cy + 7.5f, 0.7f, c);
      break;
    case Quick::CLOCK24:
      canvas.fillCircle(cx, cy, 7.5f, c);
      canvas.fillCircle(cx, cy, 5.8f, bg);
      canvas.drawWideLine(cx, cy, cx, cy - 4, 0.8f, c);
      canvas.drawWideLine(cx, cy, cx + 3, cy + 1, 0.8f, c);
      break;
    case Quick::ROTATE:  // a phone turning
      ring(cx, cy, 6, 7.5f, 200, 470, c);
      canvas.fillTriangle(cx + 1, cy - 10, cx + 1, cy - 3.5f, cx + 5.5f, cy - 6.8f, c);
      canvas.fillRoundRect(cx - 2, cy - 3, 4, 6, 1, c);
      break;
    case Quick::GESTURES:  // a stick, shaken
      canvas.fillRoundRect(cx - 3, cy - 7, 6, 14, 2, c);
      ring(cx, cy, 8, 9.4f, 150, 210, c);
      ring(cx, cy, 8, 9.4f, 330, 390, c);
      break;
    default:
      gear(cx, cy, 0.62f, c, bg);
      break;
  }
}

static const char* qlabel(Quick q) {
  switch (q) {
    case Quick::BRIGHTNESS: return "Brightness";
    case Quick::VOLUME: return "Volume";
    case Quick::SOUND: return "Sounds";
    case Quick::WMIC: return "Wispr mic";
    case Quick::CLOCK24: return "24-hour";
    case Quick::ROTATE: return "Rotate";
    case Quick::GESTURES: return "Gestures";
    default: return "Settings";
  }
}

static bool qon(const QuickView& v, Quick q) {
  switch (q) {
    case Quick::SOUND: return v.sound;
    case Quick::WMIC: return v.wmic;
    case Quick::CLOCK24: return v.clock24;
    case Quick::ROTATE: return v.rotate;
    case Quick::GESTURES: return v.gestures;
    default: return false;
  }
}

static void slider(const QuickView& v, Quick q, int x, int y, int w, int h) {
  const bool sel = v.sel == (int)q, bright = q == Quick::BRIGHTNESS;
  const uint16_t ac = bright ? SUN : AGENT;
  const int pct = bright ? v.brightness : (v.sound ? launcher::volumePct(v.volume) : 0);
  canvas.fillRoundRect(x, y, w, h, 9, sel ? blend(PANEL, ac, 0.14f) : PANEL);
  if (sel) outlineSel(x, y, w, h, 9, ac);
  qicon(q, x + 13, y + h / 2, !bright && !v.sound ? FAINT : ac, PANEL);
  const int lx = x + 27, rx = x + w - 8;
  const String value = bright || v.sound ? String(pct) + "%" : String("Muted");
  txt(value, rx, y + 10, F12, sel ? INK : DIM, middle_right);
  canvas.setFont(F12);
  const int room = rx - lx - canvas.textWidth(value) - 2;  // the label in what's left, smaller if need be
  const lgfx::IFont* lf = fit(qlabel(q), room, {F12, F9});
  canvas.setFont(lf);
  if (canvas.textWidth(qlabel(q)) <= room) txt(qlabel(q), lx, y + 10, lf, sel ? INK : DIM);
  const int ty = y + h - 9, tw = rx - lx;
  canvas.fillRoundRect(lx, ty - 2, tw, 4, 2, LINE);
  const int fw = tw * pct / 100;
  if (fw > 0) canvas.fillRoundRect(lx, ty - 2, fw, 4, 2, ac);
  if (fw > 0 || bright) canvas.fillCircle(lx + fw, ty, 4, sel ? INK : DIM);
}

static void toggleTile(const QuickView& v, Quick q, int x, int y, int w, int h, bool wide) {
  const bool sel = v.sel == (int)q, all = q == Quick::ALL, on = qon(v, q);
  const uint16_t ac = all ? GEAR : AGENT;
  const uint16_t bg = on ? blend(PANEL, ac, 0.55f) : sel ? CHIP : PANEL;
  const uint16_t ink = on ? rgb(0x06, 0x1a, 0x18) : sel ? INK : DIM;
  canvas.fillRoundRect(x, y, w, h, 9, bg);
  if (sel) outlineSel(x, y, w, h, 9, on ? INK : ac);
  const uint16_t ic = on ? ink : all ? (sel ? GEAR : DIM) : (sel ? INK : FAINT);
  if (wide) {
    qicon(q, x + 12, y + h / 2, ic, bg);
    const lgfx::IFont* f = fit(qlabel(q), w - 31, {F12, F9});
    txt(qlabel(q), x + 25, y + h / 2, f, ink);
    return;
  }
  qicon(q, x + 13, y + 13, ic, bg);
  if (!all) txt(on ? "On" : "Off", x + w - 7, y + 12, F9, on ? ink : FAINT, middle_right);
  const lgfx::IFont* f = fit(qlabel(q), w - 10, {F12, F9});
  txt(qlabel(q), x + 7, y + h - 11, f, ink);
}

void drawQuick(const QuickView& v) {
  const bool wide = turnToHeldSide();
  canvas.fillScreen(GROUND);
  bar("Quick settings", "");
  const int m = 4, gap = 5;
  const int toggles = (int)Quick::COUNT - (int)Quick::SOUND;  // six tiles
  if (wide) {
    const int sw = (W - 2 * m - gap) / 2, sh = 30, sy = BAR_H + 4;
    slider(v, Quick::BRIGHTNESS, m, sy, sw, sh);
    slider(v, Quick::VOLUME, m + sw + gap, sy, sw, sh);
    const int cols = 3, top = sy + sh + gap, bottom = H - 15;
    const int tw = (W - 2 * m - (cols - 1) * gap) / cols, th = (bottom - top - gap) / 2;
    for (int i = 0; i < toggles; i++)
      toggleTile(v, (Quick)((int)Quick::SOUND + i), m + (i % cols) * (tw + gap), top + (i / cols) * (th + gap), tw, th, true);
  } else {
    const int sh = 34, sy = BAR_H + 4;
    slider(v, Quick::BRIGHTNESS, m, sy, W - 2 * m, sh);
    slider(v, Quick::VOLUME, m, sy + sh + gap, W - 2 * m, sh);
    const int cols = 2, top = sy + 2 * (sh + gap), bottom = H - 17;
    const int tw = (W - 2 * m - gap) / cols, th = (bottom - top - 2 * gap) / 3;
    for (int i = 0; i < toggles; i++)
      toggleTile(v, (Quick)((int)Quick::SOUND + i), m + (i % cols) * (tw + gap), top + (i / cols) * (th + gap), tw, th, false);
  }
  hint(v.sel == (int)Quick::ALL ? "A next  B open  hold B back" : "A next  B set  hold B back");
  canvas.pushSprite(0, 0);
}

// --- Faces ----------------------------------------------------------------------------------------

void drawFaces(const FacesView& v) {
  const bool wide = turnToHeldSide();
  const int n = faces::count();
  canvas.fillScreen(GROUND);
  bar("Faces", String(v.sel + 1) + "/" + n);
  const int row = wide ? 19 : 22, top = BAR_H + 3, bottom = wide ? H - 29 : H - 44;
  const int visible = (bottom - top) / row;
  int first = v.sel - visible / 2;
  if (first > n - visible) first = n - visible;
  if (first < 0) first = 0;
  for (int i = first; i < n && i < first + visible; i++) {
    const int y = top + (i - first) * row;
    const bool sel = i == v.sel, cur = i == v.current;
    if (sel) {
      canvas.fillRoundRect(3, y, W - 6, row - 2, 6, CHIP);
      canvas.fillRoundRect(3, y + 4, 3, row - 10, 1, AGENT);
    }
    const int cy = y + (row - 2) / 2;
    int right = W - 10;
    if (cur) {  // in use: a teal check
      canvas.drawWideLine(right - 9, cy, right - 6, cy + 3, 1.1f, AGENT);
      canvas.drawWideLine(right - 6, cy + 3, right, cy - 4, 1.1f, AGENT);
      right -= 16;
    }
    if (i >= faces::builtinCount()) {  // an added face (a package)
      txt("added", right, cy, F9, FAINT, middle_right);
      right -= 34;
    }
    const String name = faces::at(i).title();
    txt(ellipsize(name, F12, right - 14), 13, cy, F12, sel ? INK : cur ? AGENT : DIM);
  }
  if (first > 0) canvas.fillTriangle(W / 2 - 4, top - 1, W / 2 + 4, top - 1, W / 2, top - 4, FAINT);
  if (first + visible < n) {
    const int ty = top + visible * row - 1;
    canvas.fillTriangle(W / 2 - 4, ty, W / 2 + 4, ty, W / 2, ty + 3, FAINT);
  }
  // what the face is: two lines in portrait, one in landscape (cut with ...)
  String lines[2];
  const int nl = wrap(faces::at(v.sel).about(), F9, lines, wide ? 1 : 2, W - 10);
  for (int i = 0; i < nl; i++)
    txt(i == nl - 1 ? ellipsize(lines[i], F9, W - 10) : lines[i], W / 2, H - 22 - (nl - 1 - i) * 12, F9, DIM, middle_center);
  hint("A next  B use  hold B back");
  canvas.pushSprite(0, 0);
}

}  // namespace launcherui
