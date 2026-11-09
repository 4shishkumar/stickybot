#include "ui.h"
#include "ui_draw.h"
#include "faces/faces.h"
#include <M5Unified.h>
#include "battery.h"
#include "fonts_vlw.h"
#include "logic/text.h"
#include "logic/prompt.h"
#include "logic/orientation.h"

namespace ui {

// Portrait, 135 x 240. "Flip screen" in the menu switches between the two
// portrait rotations (0 and 2); with auto-rotate the motion sensor picks them.
// Landscape, 240 x 135 (rotations 1 and 3): the screens with text to read (prompts,
// questions, events, the agent, home / status, messages) have a wide layout too, used
// while the stick is held sideways (setOrientation); the rest stay portrait.
//
// Look: a calm dark base with anti-aliased Inter fonts (tools/make_fonts.py). The
// agent screens have big eyes with a mood (colour + expression) that also tints
// the background; an approval prompt floods the screen orange; while charging
// and idle the stick becomes a desk clock (landscape when lying on its side).


// Anti-aliased fonts, loaded from flash in begin(). F44 / F56 / F96 have digits only (clocks).
struct Vlw {
  lgfx::PointerWrapper data;
  lgfx::VLWfont font;
  const lgfx::IFont* load(const uint8_t* p, size_t n) {
    data.set(p, n);
    return font.loadFont(&data) ? &font : (const lgfx::IFont*)&fonts::Font2;
  }
};
static Vlw vlw10, vlw12, vlw14, vlw18, vlw24, vlw32, vlw44, vlw56, vlw96;
const lgfx::IFont *F9, *F12, *F14, *F18, *F24, *F32, *F44, *F56, *F96;  // F44/F56/F96: digits only (clocks)

M5Canvas canvas(&M5.Display);
static uint8_t  brightness = 128;
static bool     awake_ = true, flipped_ = false;
static uint32_t lastActivity = 0, awakeFor = 60000;
int             W, H;
static float    level_ = 0;  // smoothed mic level 0..1

void begin() {
  M5.Display.setRotation(0);
  M5.Display.setBrightness(brightness);
  W = M5.Display.width();
  H = M5.Display.height();
  canvas.setColorDepth(16);
  canvas.createSprite(W, H);
  F9 = vlw10.load(INTER_10, INTER_10_LEN);
  F12 = vlw12.load(INTER_12, INTER_12_LEN);
  F14 = vlw14.load(INTER_14, INTER_14_LEN);
  F18 = vlw18.load(INTER_18, INTER_18_LEN);
  F24 = vlw24.load(INTER_24, INTER_24_LEN);
  F32 = vlw32.load(INTER_32, INTER_32_LEN);
  F44 = vlw44.load(INTER_44, INTER_44_LEN);
  F56 = vlw56.load(INTER_56, INTER_56_LEN);
  F96 = vlw96.load(INTER_96, INTER_96_LEN);
  lastActivity = millis();
}

static int  orient_ = 0;  // shown now: 0 portrait; +1 / -1 landscape (rotation 1 / 3)
static int  want_ = 0;    // the reading screens' rotation, from setOrientation()
static bool dimmed_ = false;

void setFlipped(bool flipped) {
  flipped_ = flipped;
  if (orient_ == 0) M5.Display.setRotation(flipped ? 2 : 0);
}

void setOrientation(bool flipped, int side) {
  want_ = side;
  setFlipped(flipped);
}

// Turns the display (and the canvas, sized to match) to portrait (0) or a landscape side.
static void applyOrientation(int o) {
  if (o == orient_) return;
  orient_ = o;
  canvas.deleteSprite();
  M5.Display.setRotation(o == 0 ? (flipped_ ? 2 : 0) : o > 0 ? 1 : 3);
  W = M5.Display.width();
  H = M5.Display.height();
  canvas.createSprite(W, H);
}

static void undim() {
  if (!dimmed_) return;
  dimmed_ = false;
  if (awake_) M5.Display.setBrightness(brightness);
}

void useSide(int side) {
  want_ = side;
  applyOrientation(side);
  undim();
}

void dockEnd() {  // portrait-only screens
  applyOrientation(0);
  undim();
}

// Screens with a landscape layout: turn to the held side; true = draw the wide layout.
static bool wide() {
  applyOrientation(want_);
  undim();
  return orient_ != 0;
}

// Landscape: the end of the screen that button A is at (+1 right, -1 left). The
// stick turned with its right edge up (rotation 1) has A on the right.
static int aEnd() { return orientation::buttonAOnRight(orient_) ? 1 : -1; }

void setBrightness(uint8_t pct) {
  brightness = (uint16_t)pct * 255 / 100;
  if (awake_) M5.Display.setBrightness(brightness);
}

bool wake(uint32_t forMs) {
  lastActivity = millis();
  awakeFor = forMs;
  if (awake_) return false;
  M5.Display.wakeup();
  M5.Display.setBrightness(brightness);
  awake_ = true;
  return true;
}

void keepAwake() { lastActivity = millis(); }

void off() {
  if (!awake_) return;
  M5.Display.setBrightness(0);
  M5.Display.sleep();
  awake_ = false;
}

void tick(bool allowSleep) {
  if (!allowSleep) lastActivity = millis();
  if (awake_ && millis() - lastActivity > awakeFor) off();
}

bool awake() { return awake_; }

void setLevel(int peak) {
  float v = constrain(peak / 14000.0f, 0.0f, 1.0f);
  level_ = v > level_ ? v : level_ * 0.82f;  // fast attack, slow release
}

// --- Text ------------------------------------------------------------------------

void txt(const String& s, int x, int y, const lgfx::IFont* f, uint16_t color, textdatum_t datum) {
  canvas.setFont(f);
  canvas.setTextColor(color);
  canvas.setTextDatum(datum);
  canvas.drawString(s, x, y);
}

// Biggest of `fonts` (largest first) in which s fits maxW.
const lgfx::IFont* fit(const String& s, int maxW, std::initializer_list<const lgfx::IFont*> fonts) {
  const lgfx::IFont* last = nullptr;
  for (auto f : fonts) {
    canvas.setFont(f);
    last = f;
    if (canvas.textWidth(s) <= maxW) return f;
  }
  return last;
}

// Wrapping and "..." cutting: logic/text.cpp (unit-tested), measured with the canvas font.
static int canvasWidth(const std::string& s, void*) { return canvas.textWidth(s.c_str()); }

// One line that fits maxW with font f, cut with "..." if needed.
String ellipsize(const String& s, const lgfx::IFont* f, int maxW) {
  canvas.setFont(f);
  return String(text::ellipsize(std::string(s.c_str()), maxW, canvasWidth, nullptr).c_str());
}

// Greedy word wrap with font f; if it doesn't all fit, the last line ends in "...".
int wrap(const String& s, const lgfx::IFont* f, String out[], int max, int maxW) {
  canvas.setFont(f);
  auto lines = text::wrap(std::string(s.c_str()), max, maxW, canvasWidth, nullptr);
  int n = 0;
  for (auto& l : lines) out[n++] = String(l.c_str());
  return n;
}

int paragraph(const String& s, const lgfx::IFont* f, int x, int y, int maxW, int maxLines, uint16_t color,
                     int lineH) {
  String lines[12];
  int n = wrap(s, f, lines, min(maxLines, 12), maxW);
  for (int i = 0; i < n; i++) txt(lines[i], x, y + i * lineH, f, color, top_left);
  return n;
}

// Line height for each font in wrapped text.
int lineH(const lgfx::IFont* f) {
  return f == F9 ? 13 : f == F12 ? 16 : f == F14 ? 18 : f == F18 ? 22 : f == F32 ? 36 : 26;
}

// Biggest of `fonts` (largest first) in which all of s wraps into a box maxW x boxH
// (logic/text.cpp fits()); else the last one (the text is then cut with "...").
static const lgfx::IFont* fitBox(const String& s, int maxW, int boxH, std::initializer_list<const lgfx::IFont*> fonts) {
  const lgfx::IFont* last = nullptr;
  for (auto f : fonts) {
    canvas.setFont(f);
    last = f;
    if (text::fits(std::string(s.c_str()), boxH / lineH(f), maxW, canvasWidth, nullptr)) return f;
  }
  return last;
}

// Wrapped lines centred on cx, the block centred on cy.
static void centredLines(const String& text, const lgfx::IFont* f, int cx, int cy, int maxW, int maxLines,
                         uint16_t color, int lh) {
  String lines[4];
  int n = wrap(text, f, lines, min(maxLines, 4), maxW);
  int y = cy - (n - 1) * lh / 2;
  for (int i = 0; i < n; i++) txt(lines[i], cx, y + i * lh, f, color, middle_center);
}

// "Ctrl+Option+Cmd" -> {"Ctrl", "Option", "Cmd"}
static int comboParts(const Combo& c, String out[], int max) {
  String s = comboToString(c);
  int n = 0, start = 0;
  while (n < max) {
    int plus = s.indexOf('+', start);
    out[n++] = s.substring(start, plus < 0 ? s.length() : plus);
    if (plus < 0) break;
    start = plus + 1;
  }
  return n;
}

// --- Icons and pieces ----------------------------------------------------------

void btIcon(int x, int y, uint16_t c) {  // 7 x 11, top-left at x, y
  canvas.drawLine(x + 3, y, x + 3, y + 10, c);
  canvas.drawLine(x + 3, y, x + 6, y + 3, c);
  canvas.drawLine(x + 6, y + 3, x, y + 8, c);
  canvas.drawLine(x, y + 2, x + 6, y + 7, c);
  canvas.drawLine(x + 6, y + 7, x + 3, y + 10, c);
}

// Battery: an outline filled to the level, a bolt over it while charging.
void batteryIcon(int x, int y, int pct, bool charging) {  // 18 x 9, top-left
  uint16_t c = pct >= 0 && pct <= 15 && !charging ? ERR : charging ? OK : INK;
  canvas.drawRoundRect(x, y, 16, 9, 2, DIM);
  canvas.fillRect(x + 16, y + 3, 2, 3, DIM);
  if (pct >= 0) canvas.fillRect(x + 2, y + 2, max(1, 12 * pct / 100), 5, charging ? rgb(0x1f, 0x6e, 0x45) : c);
  if (charging) {  // bolt
    canvas.fillTriangle(x + 9, y - 1, x + 4, y + 5, x + 8, y + 5, OK);
    canvas.fillTriangle(x + 7, y + 4, x + 11, y + 4, x + 6, y + 10, OK);
  }
}

// Status bar: agent dot (+ title or agent state) on the left; Bluetooth and
// battery (% + icon, bolt while charging) on the right, evenly spaced.
static void statusBar(const String& left, const Dash* d, uint16_t leftColor = INK) {
  int x = PAD;
  if (d) {
    canvas.fillCircle(x + 3, BAR_H / 2, 3, d->agentLinked ? AGENT : FAINT);
    x += 11;
  }
  if (left.length()) txt(left, x, BAR_H / 2, F12, leftColor);
  if (!d) return;
  const int gap = 8;
  int r = W - PAD;
  r -= 18;
  batteryIcon(r, BAR_H / 2 - 4, d->battery, d->charging);
  if (d->battery >= 0) {
    String pct = String(d->battery) + "%";
    canvas.setFont(F9);
    r -= canvas.textWidth(pct) + 3;
    txt(pct, r, BAR_H / 2, F9, d->charging ? OK : DIM);
  }
  r -= gap + 7;
  btIcon(r, BAR_H / 2 - 5, d->bt == 0 ? INK : d->bt == 1 ? ASK : FAINT);
}

static void pageDots(Page p) {
  int n = (int)Page::COUNT, gap = 9, x = W / 2 - (n - 1) * gap / 2;
  for (int i = 0; i < n; i++) canvas.fillCircle(x + i * gap, H - 7, i == (int)p ? 3 : 2, i == (int)p ? INK : LINE);
}

static void chip(const String& s, int cx, int cy, uint16_t dot, uint16_t bg = CHIP, uint16_t fg = INK) {
  canvas.setFont(F12);
  int w = canvas.textWidth(s) + (dot ? 26 : 18), h = 22;
  canvas.fillRoundRect(cx - w / 2, cy - h / 2, w, h, h / 2, bg);
  int tx = cx - w / 2 + 9;
  if (dot) {
    canvas.fillCircle(tx + 3, cy, 3, dot);
    tx += 11;
  }
  txt(s, tx, cy, F12, fg);
}

static void card(int x, int y, int w, int h) { canvas.fillRoundRect(x, y, w, h, 10, PANEL); }

// A button chip for the landscape screens, filled or outlined. arrow (+1 / -1): a small
// arrow at that end pointing at the real button (A, at the left or right end of the screen).
static void keyChip(const String& s, int x, int y, int w, int h, uint16_t c, uint16_t bg, bool filled, int arrow) {
  if (filled) canvas.fillRoundRect(x, y, w, h, h / 2, c);
  else canvas.drawRoundRect(x, y, w, h, h / 2, c);
  const uint16_t ink = filled ? bg : c;
  int cx = x + w / 2, room = w - 16;
  if (arrow) {
    int ax = arrow > 0 ? x + w - h / 2 + 2 : x + h / 2 - 2, cy = y + h / 2;
    canvas.fillTriangle(ax - 3 * arrow, cy - 5, ax - 3 * arrow, cy + 5, ax + 4 * arrow, cy, ink);
    cx -= arrow * 7;
    room -= 14;
  }
  const lgfx::IFont* f = fit(s, room, {F12, F9});
  txt(ellipsize(s, f, room), cx, y + h / 2, f, ink, middle_center);
}

// --- Colour helpers ----------------------------------------------------------------

// Mixes two RGB565 colours: t = 0 -> a, t = 1 -> b.
uint16_t blend(uint16_t a, uint16_t b, float t) {
  t = constrain(t, 0.0f, 1.0f);
  int ar = a >> 11, ag = (a >> 5) & 63, ab = a & 31, br = b >> 11, bg = (b >> 5) & 63, bb = b & 31;
  return ((int)(ar + (br - ar) * t) << 11) | ((int)(ag + (bg - ag) * t) << 5) | (int)(ab + (bb - ab) * t);
}

float smooth(float t) {
  t = constrain(t, 0.0f, 1.0f);
  return t * t * (3 - 2 * t);
}


// --- The eyes ------------------------------------------------------------------------

// The agent's eyes, one look per mood. Each mood has a colour, which also tints
// the background glow and the text panel on the agent screens.

uint16_t moodColor(Mood m) {
  static const uint16_t HUES[] = {WISPR, AGENT, ASK, OK, ERR};
  switch (m) {
    case Mood::LISTEN:  return BRIGHT;
    case Mood::THINK:   return VIOLET;
    case Mood::WORK:    return AMBER;
    case Mood::DONE:    return OK;
    case Mood::ERROR:   return ERR;
    case Mood::ASK:     return ASK;
    case Mood::OFFLINE: return FAINT;
    case Mood::PARTY:   return HUES[(millis() / 300) % 5];
    default:            return AGENT;
  }
}

// Eyes follow the tilt: the change in tilt from where the stick has been resting
// (a slow average), so they roll towards the low side and drift back when still.
static float tiltDX = 0, tiltDY = 0, restX = 0, restY = 1;
static bool  tiltInit = false;

static float rawX = 0, rawY = 1;  // the last reading, for gravity()

void setTilt(float ax, float ay) {
  rawX = ax;
  rawY = ay;
  if (!tiltInit) { restX = ax; restY = ay; tiltInit = true; }
  restX += (ax - restX) * 0.008f;  // ~5 s to drift back to centre at 25 fps
  restY += (ay - restY) * 0.008f;
  float dx = constrain(ax - restX, -0.6f, 0.6f), dy = constrain(ay - restY, -0.6f, 0.6f);
  tiltDX += (dx - tiltDX) * 0.6f;  // quick, with a little smoothing
  tiltDY += (dy - tiltDY) * 0.6f;
}

// Down on the screen, from the stick's axes (orientation.h): portrait x right / y down is
// (-ax, ay); upside down (ax, -ay); rotation 1 (ay, ax); rotation 3 (-ay, -ax).
void gravity(float& gx, float& gy) {
  if (orient_ > 0) { gx = rawY; gy = rawX; }
  else if (orient_ < 0) { gx = -rawY; gy = -rawX; }
  else if (flipped_) { gx = rawX; gy = -rawY; }
  else { gx = -rawX; gy = rawY; }
}

bool landscapeNow() { return orient_ != 0; }

static float lookX = NAN;  // when set, idle pupils look this way (the home screen's peek)
void lookAt(float x) { lookX = x; }

// A soft radial glow: stacked circles, faint outside, stronger in the middle.
void glow(int cx, int cy, int r, uint16_t c, float strength) {
  for (int i = 6; i >= 1; i--) canvas.fillCircle(cx, cy, r * i / 6, blend(GROUND, c, strength * (7 - i) / 6.0f));
}

static void star(float cx, float cy, float ro, float ri, float rot, uint16_t c) {
  float px[10], py[10];
  for (int i = 0; i < 10; i++) {
    float a = rot - PI / 2 + i * PI / 5, r = (i & 1) ? ri : ro;
    px[i] = cx + cosf(a) * r;
    py[i] = cy + sinf(a) * r;
  }
  for (int i = 0; i < 10; i++) canvas.fillTriangle(cx, cy, px[i], py[i], px[(i + 1) % 10], py[(i + 1) % 10], c);
}

// Two big eyes centred on cx, cy; s = scale (1 = 40 x 56 px eyes); bg = what's behind them.
void eyes(Mood m, int cx, int cy, float s, uint16_t c, uint16_t bg) {
  const uint32_t t = millis();
  float ew = 40 * s, eh = 56 * s, gap = 30 * s, r = 18 * s;
  // Blink with squash and stretch: squeeze flat and wide, then pop back a little tall
  float sy = 1, sx = 1;
  if (m == Mood::IDLE || m == Mood::LISTEN || m == Mood::ASK || m == Mood::THINK) {
    uint32_t ph = (t + (m == Mood::THINK ? 1700 : 0)) % 4200;
    if (ph < 70) { float k = ph / 70.0f; sy = 1 - 0.88f * k; sx = 1 + 0.16f * k; }
    else if (ph < 160) { float k = (ph - 70) / 90.0f; sy = 0.12f + 0.96f * k; sx = 1.16f - 0.16f * k; }
    else if (ph < 260) { float k = (ph - 160) / 100.0f; sy = 1.08f - 0.08f * k; }
  }
  float px = 0, py = 0, dil = 1, drop = 0;
  switch (m) {
    case Mood::IDLE: {
      if (!isnan(lookX)) { px = lookX; break; }
      static const int8_t G[][2] = {{0, 0}, {-7, 1}, {0, 0}, {7, -2}};
      uint32_t seg = t / 2600, in = t % 2600;
      const int8_t *from = G[(seg + 3) % 4], *to = G[seg % 4];
      float k = smooth(in / 260.0f);
      px = from[0] + (to[0] - from[0]) * k;
      py = from[1] + (to[1] - from[1]) * k;
      break;
    }
    case Mood::LISTEN: eh *= 1.06f + 0.08f * level_; dil = 1 + 0.4f * level_; py = -2; break;
    case Mood::THINK: { float a = t / 380.0f; px = cosf(a) * 6; py = -9 + sinf(a) * 3; break; }
    case Mood::WORK: eh *= 0.5f; drop = 56 * s * 0.12f; px = sinf(t / 500.0f) * 3; break;
    case Mood::ASK: eh *= 1.1f; py = -8; break;
    case Mood::OFFLINE: eh *= 0.34f; drop = 56 * s * 0.2f; py = 2; break;
    default: break;
  }
  if (m != Mood::OFFLINE) {  // follow the tilt, turned to the screen's rotation
    float f = flipped_ ? -1 : 1, sx = f * tiltDX, sy = f * tiltDY;
    if (orient_ > 0) { sx = -tiltDY; sy = tiltDX; }        // rotation 1: right edge up
    else if (orient_ < 0) { sx = tiltDY; sy = -tiltDX; }   // rotation 3: left edge up
    px = constrain(px + sx * 30, -13.0f, 13.0f);
    py = constrain(py + sy * 24, -13.0f, 13.0f);
  }
  for (int side = -1; side <= 1; side += 2) {
    float ex = cx + side * gap;
    if (m == Mood::DONE) {  // happy ^ ^
      canvas.fillArc(ex, cy + 10 * s, 21 * s, 13 * s, 200, 340, c);
    } else if (m == Mood::ERROR) {  // x x, drooping a little
      float d = 12 * s, y = cy + 4 * s;
      canvas.drawWideLine(ex - d, y - d, ex + d, y + d, 3.5f * s, c);
      canvas.drawWideLine(ex - d, y + d, ex + d, y - d, 3.5f * s, c);
    } else if (m == Mood::PARTY) {  // star eyes, spinning
      star(ex, cy, 22 * s, 10 * s, side * t / 600.0f, c);
    } else {
      float w = ew * sx, h = eh * sy, top = cy - h / 2 + drop;
      canvas.fillRoundRect(ex - w / 2, top, w, h, min(r, h / 2), c);
      if (h > 12 * s) {
        float pw = 16 * s * dil, ph = min(22 * s * dil, h - 8 * s);
        float x = ex - pw / 2 + px * s, y = top + h / 2 - ph / 2 + py * s;
        y = constrain(y, top + 3 * s, top + h - ph - 3 * s);
        canvas.fillRoundRect(x, y, pw, ph, pw / 2.2f, PUPIL);
        canvas.fillCircle(x + pw * 0.32f, y + ph * 0.3f, max(1.0f, 2.6f * s), blend(c, TFT_WHITE, 0.75f));
      }
    }
  }
  // Extras per mood
  if (m == Mood::ASK) {  // raised eyebrows
    for (int side = -1; side <= 1; side += 2) {
      float ex = cx + side * gap, by = cy - eh / 2 - 10 * s;
      canvas.drawWideLine(ex - side * 14 * s, by + 3 * s, ex + side * 12 * s, by - 5 * s, 2.6f * s, c);
    }
  } else if (m == Mood::WORK) {  // a small spinner under the eyes
    int a = (t / 3) % 360;
    canvas.fillArc(cx, cy + 34 * s, 8 * s, 5 * s, a, a + 250, c);
  } else if (m == Mood::DONE) {  // a twinkling sparkle
    float k = 0.6f + 0.4f * sinf(t / 180.0f), sx2 = cx + gap + 24 * s, sy2 = cy - 26 * s, len = 8 * s * k;
    canvas.drawWideLine(sx2 - len, sy2, sx2 + len, sy2, 1.4f * s, c);
    canvas.drawWideLine(sx2, sy2 - len, sx2, sy2 + len, 1.4f * s, c);
  } else if (m == Mood::OFFLINE) {  // "z z z" drifting up
    for (int i = 0; i < 3; i++) {
      float ph = fmodf(t / 1400.0f + i / 3.0f, 1.0f);
      float x = cx + gap + 14 * s + ph * 12 * s, y = cy - 8 * s - ph * 34 * s;
      txt("z", x, y, i == 0 ? F9 : F12, blend(c, bg, ph), middle_center);
    }
  } else if (m == Mood::LISTEN) {  // "ears": bars that follow the mic
    for (int side = -1; side <= 1; side += 2) {
      for (int i = 0; i < 3; i++) {
        float wob = 0.55f + 0.45f * sinf(t / 90.0f + i * 1.7f + (side > 0 ? 0.8f : 0));
        int h = 4 + (int)(level_ * 34 * s * wob) - i * 4;
        if (h < 3) h = 3;
        int x = cx + side * (gap + ew / 2 + 8 * s + i * 6 * s);
        canvas.fillRoundRect(x - 1, cy - h / 2, 3, h, 1, c);
      }
    }
  }
}

// Landscape agent screens. Without a reply: big eyes in the middle, the label and the
// dim line (what was heard) under them. With one: small eyes + label in a band at the
// top, the reply in a full-width panel below (the widest lines, the biggest font that fits).
static void moodScreenWide(Mood m, const String& label, const String& dimText, const String& inkText, bool bar,
                           const String& barTitle, const Dash* d, int bottomGap) {
  const uint16_t c = moodColor(m), bgTint = blend(GROUND, c, 0.2f);
  canvas.fillScreen(GROUND);
  const int top0 = bar ? BAR_H : 0, bottom = H - 4 - bottomGap, inner = W - 2 * PAD - 6;
  if (!inkText.length()) {
    String dl[2];
    int dn = dimText.length() ? wrap(dimText, F12, dl, 2, W - 2 * PAD) : 0;
    int eyesBottom = bottom - (label.length() ? 22 : 0) - dn * 15 - 4;
    int cy = (top0 + eyesBottom) / 2 + 2;
    float sc = constrain((eyesBottom - top0 - 20) / 70.0f, 0.5f, 0.9f);
    glow(W / 2, cy, (int)(76 * sc) + 10, c, 0.24f);
    eyes(m, W / 2, cy, sc, c, bgTint);
    int y = eyesBottom + 4;
    if (label.length()) {
      txt(label, W / 2, y + 9, fit(label, W - 2 * PAD, {F18, F14, F12}), c, middle_center);
      y += 22;
    }
    for (int i = 0; i < dn; i++) txt(dl[i], W / 2, y + 7 + i * 15, F12, blend(DIM, GROUND, 0.1f), middle_center);
  } else {
    // the band: eyes on the left, the label and / or the heard line beside them
    const int band = 40, ex = PAD + 25, cy = top0 + band / 2, tx = 66, tw = W - tx - PAD;
    glow(ex, cy, 30, c, 0.2f);
    eyes(m, ex, cy, 0.42f, c, bgTint);
    if (label.length()) {
      txt(label, tx, cy - (dimText.length() ? 8 : 0), fit(label, tw, {F18, F14, F12}), c);
      if (dimText.length()) txt(ellipsize(dimText, F9, tw), tx, cy + 10, F9, DIM);
    } else if (dimText.length()) {
      String dl[2];
      int dn = wrap(dimText, F12, dl, 2, tw);
      for (int i = 0; i < dn; i++) txt(dl[i], tx, cy - (dn - 1) * 8 + i * 16, F12, DIM);
    }
    // the panel
    const int panelTop = top0 + band, ph = bottom - panelTop;
    const uint16_t pb = blend(GROUND, c, 0.17f);
    canvas.fillRoundRect(4, panelTop, W - 8, ph, 12, pb);
    canvas.drawFastHLine(18, panelTop, W - 36, blend(pb, c, 0.45f));
    const lgfx::IFont* tf = fitBox(inkText, inner, ph - 10, {F14, F12, F9});
    paragraph(inkText, tf, PAD + 3, panelTop + 6, inner, (ph - 10) / lineH(tf), INK, lineH(tf));
  }
  if (bar) statusBar(barTitle, d, c);
}

// Agent screens: mood glow + big eyes on top, a mood-tinted panel with the text at
// the bottom. The panel grows with the text; the eyes shrink to make room.
static void moodScreen(Mood m, const String& label, const String& dimText, const String& inkText, bool bar,
                       const String& barTitle, const Dash* d, int bottomGap) {
  if (orient_ != 0) return moodScreenWide(m, label, dimText, inkText, bar, barTitle, d, bottomGap);
  const uint16_t c = moodColor(m);
  canvas.fillScreen(GROUND);
  const int top0 = bar ? BAR_H : 0, inner = W - 2 * PAD - 6;
  String dl[3], tl[10];
  int dn = dimText.length() ? wrap(dimText, F9, dl, 3, inner) : 0;
  const lgfx::IFont* tf = inkText.length() > 110 ? F9 : F12;
  const int tlh = tf == F9 ? 13 : 16;
  int tn = inkText.length() ? wrap(inkText, tf, tl, 10, inner) : 0;
  int need = 12 + (label.length() ? 22 : 0) + dn * 13 + (dn && tn ? 5 : 0) + tn * tlh + 6;
  int bottom = H - 4 - bottomGap;
  int panelTop = constrain(bottom - need, top0 + 96, bottom - 58);
  int cy = (top0 + panelTop) / 2 + 2;
  float sc = constrain((panelTop - top0 - 26) / 84.0f, 0.55f, 1.0f);
  glow(W / 2, cy, (int)(76 * sc) + 10, c, 0.24f);
  eyes(m, W / 2, cy, sc, c, blend(GROUND, c, 0.2f));
  const uint16_t pb = blend(GROUND, c, 0.17f);
  canvas.fillRoundRect(4, panelTop, W - 8, bottom - panelTop, 14, pb);
  canvas.drawFastHLine(18, panelTop, W - 36, blend(pb, c, 0.45f));
  int y = panelTop + 9;
  if (label.length()) {
    txt(label, PAD + 3, y + 8, fit(label, inner, {F14, F12}), c);
    y += 22;
  }
  for (int i = 0; i < dn; i++, y += 13) txt(dl[i], PAD + 3, y, F9, blend(DIM, pb, 0.1f), top_left);
  if (dn && tn) y += 5;
  for (int i = 0; i < tn && y < bottom - 8; i++, y += tlh) txt(tl[i], PAD + 3, y, tf, INK, top_left);
  if (bar) statusBar(barTitle, d, c);
}

// --- Status pages ---------------------------------------------------------------------

static String fmtUptime(uint32_t s) {
  if (s >= 86400) return String(s / 86400) + "d " + String(s % 86400 / 3600) + "h";
  if (s >= 3600) return String(s / 3600) + "h " + String(s % 3600 / 60) + "m";
  return String(s / 60) + "m " + String(s % 60) + "s";
}

static void row(const char* label, const String& value, int y, uint16_t color = INK) {
  txt(label, PAD, y, F9, DIM);
  txt(value, W - PAD, y, F12, color, middle_right);
}

static String clockOr(const Dash& d, const String& fallback) { return d.clock.length() ? d.clock : fallback; }

// --- Weather icons (drawn on a 56-unit grid, scaled to `size` px) -------------


static void cloudShape(int x, int y, float s, uint16_t c) {
  canvas.fillCircle(x + 18 * s, y + 30 * s, 10 * s, c);
  canvas.fillCircle(x + 30 * s, y + 24 * s, 13 * s, c);
  canvas.fillCircle(x + 40 * s, y + 32 * s, 9 * s, c);
  canvas.fillRoundRect(x + 12 * s, y + 30 * s, 32 * s, 11 * s, 5 * s, c);
}

static void sunShape(int cx, int cy, float r, float rays) {
  canvas.fillCircle(cx, cy, r, SUN);
  for (int i = 0; i < 8; i++) {
    float a = i * PI / 4;
    canvas.drawWideLine(cx + cosf(a) * (r + 3), cy + sinf(a) * (r + 3), cx + cosf(a) * (r + rays),
                        cy + sinf(a) * (r + rays), max(0.6f, rays / 6), SUN);
  }
}

void weatherIcon(const String& code, bool day, int x, int y, int size) {
  const float s = size / 56.0f;
  auto X = [&](float v) { return x + v * s; };
  auto Y = [&](float v) { return y + v * s; };
  if (code == "clear" && !day) {  // moon
    canvas.fillCircle(X(27), Y(28), 15 * s, SNOW);
    canvas.fillCircle(X(35), Y(21), 13 * s, GROUND);
  } else if (code == "clear") {
    sunShape(X(28), Y(28), 11 * s, 9 * s);
  } else if (code == "partly") {
    if (day) sunShape(X(21), Y(20), 8 * s, 6 * s);
    else { canvas.fillCircle(X(20), Y(19), 9 * s, SNOW); canvas.fillCircle(X(25), Y(15), 8 * s, GROUND); }
    cloudShape(X(4), Y(6), 0.95f * s, INK);
  } else if (code == "rain" || code == "storm" || code == "snow") {
    cloudShape(x, Y(-4), s, code == "storm" ? DIM : INK);
    if (code == "rain")
      for (int i = 0; i < 3; i++) canvas.drawWideLine(X(20 + i * 10), Y(42), X(16 + i * 10), Y(51), max(0.8f, 1.5f * s), RAIN);
    else if (code == "snow")
      for (auto p : {16, 28, 40}) canvas.fillCircle(X(p), Y(48), max(1.0f, 2.4f * s), SNOW);
    else {  // lightning
      canvas.fillTriangle(X(30), Y(36), X(20), Y(47), X(29), Y(45), ASK);
      canvas.fillTriangle(X(27), Y(43), X(36), Y(43), X(23), Y(56), ASK);
    }
  } else if (code == "fog") {
    cloudShape(x, Y(-8), 0.9f * s, DIM);
    for (int i = 0; i < 3; i++) canvas.drawWideLine(X(8 + i * 3), Y(38 + i * 7), X(48 - i * 3), Y(38 + i * 7), max(0.8f, 1.5f * s), INK);
  } else {  // cloudy
    cloudShape(x, y, s, DIM);
    cloudShape(X(4), Y(4), 0.9f * s, INK);
  }
}

// --- Home: big clock, weather chip, and one card from the agent ----------------

// Small icon for the card, by kind (12 x 12 around cx, cy)
static void cardIcon(const String& k, int cx, int cy, uint16_t c) {
  if (k == "claude") {  // sparkle
    canvas.drawWideLine(cx, cy - 6, cx, cy + 6, 0.8f, c);
    canvas.drawWideLine(cx - 6, cy, cx + 6, cy, 0.8f, c);
    canvas.drawWideLine(cx - 4, cy - 4, cx + 4, cy + 4, 0.8f, c);
    canvas.drawWideLine(cx + 4, cy - 4, cx - 4, cy + 4, 0.8f, c);
  } else if (k == "news") {  // lines
    for (int i = 0; i < 3; i++) canvas.fillRoundRect(cx - 6, cy - 5 + i * 4, i == 2 ? 8 : 12, 2, 1, c);
  } else if (k == "history") {  // clock face
    canvas.drawCircle(cx, cy, 6, c);
    canvas.drawLine(cx, cy, cx, cy - 4, c);
    canvas.drawLine(cx, cy, cx + 3, cy + 1, c);
  } else if (k == "reply") {  // speech bubble
    canvas.fillRoundRect(cx - 6, cy - 5, 12, 9, 3, c);
    canvas.fillTriangle(cx - 3, cy + 3, cx + 1, cy + 3, cx - 4, cy + 7, c);
  } else if (k == "meeting") {  // calendar
    canvas.drawRoundRect(cx - 6, cy - 5, 12, 11, 2, c);
    canvas.fillRect(cx - 6, cy - 5, 12, 3, c);
  } else if (k == "wiki") {  // a "W"
    txt("W", cx, cy, F12, c, middle_center);
  } else if (k == "fact") {  // a light bulb
    canvas.fillCircle(cx, cy - 2, 5, c);
    canvas.fillRect(cx - 2, cy + 3, 5, 3, c);
  } else {  // hello: a dot
    canvas.fillCircle(cx, cy, 3, c);
  }
}

static uint16_t kindColor(const String& k) {
  if (k == "claude" || k == "reply") return AGENT;
  if (k == "fact") return FACT;
  if (k == "news") return WISPR;
  if (k == "history") return BLUE;
  if (k == "wiki") return OK;
  if (k == "approval" || k == "meeting") return ASK;
  return DIM;
}

// --- Home card: cross-fades between items; now and then the eyes peek over it --------

struct CardView {
  String kind, a, b;
  bool   urgent = false, own = false;  // own: the stick's message (no rotation dots)
  int    index = 0, count = 0;
  bool same(const CardView& o) const { return kind == o.kind && a == o.a && b == o.b && urgent == o.urgent; }
};
static CardView shownCard, nextCard;
static bool     haveCard = false, transActive = false, transPeek = false, forcePeek = false;
static uint32_t transAt = 0, lastPeekAt = 0;
static int      changesSincePeek = 0;
constexpr uint32_t FADE_MS = 170, PEEK_MS = 1500;
constexpr int CARD_TOP = 146, CARD_H = 70;           // portrait: under the clock, full width
constexpr int CARD_X_WIDE = 118;                     // landscape: the right column, full height
static int cardTop_ = CARD_TOP, cardH_ = CARD_H, cardX_ = 4, cardW_ = 0;

bool homeAnimating() { return transActive; }
void forceHomePeek() { forcePeek = true; }

static void drawCard(const CardView& v, float alpha, int dip) {
  const int x = cardX_, y = cardTop_ + dip, w = cardW_ ? cardW_ : W - 8;
  const bool wide = cardX_ != 4;  // landscape: a narrow, tall card on the right
  if (v.urgent) {
    canvas.fillRoundRect(x, y, w, cardH_, 12, ASK);
    { const lgfx::IFont* f = fit(v.a, w - 14, {F14, F12});
      txt(ellipsize(v.a, f, w - 14), x + 8, y + 16, f, blend(ON_ASK, ASK, 1 - alpha)); }
    paragraph(v.b, F12, x + 8, y + 30, w - 14, wide ? 5 : 2, blend(ON_ASK, ASK, 1 - alpha), 15);
    return;
  }
  const uint16_t kc = kindColor(v.kind);
  // the card's tint fades with the text, through a neutral grey, so kinds blend into each other
  const uint16_t bg = blend(blend(GROUND, INK, 0.04f), kc, 0.13f * alpha);
  canvas.fillRoundRect(x, y, w, cardH_, 12, bg);
  if (alpha <= 0.02f) return;
  const int ty = y + 15;
  cardIcon(v.kind, x + 11, ty, blend(bg, kc, alpha));
  { const lgfx::IFont* f = fit(v.a, w - 28, {F12, F9});
    txt(ellipsize(v.a, f, w - 28), x + 22, ty, f, blend(bg, kc, alpha)); }
  if (wide) {  // up to 5 lines down the tall card
    const lgfx::IFont* f = fitBox(v.b, w - 14, 5 * 15, {F12, F9});
    paragraph(v.b, f, x + 8, y + 30, w - 14, f == F9 ? 6 : 5, blend(bg, INK, 0.9f * alpha), f == F9 ? 13 : 15);
    return;
  }
  bool longText = v.b.length() > 44;
  paragraph(v.b, longText ? F9 : F12, x + 6, y + 27, w - 14, 3,
            blend(bg, INK, 0.9f * alpha), longText ? 13 : 15);
}


// The bottom of the home screen: the card for `v`, animating from the old one.
static void homeCard(const CardView& v) {
  const uint32_t now = millis();
  if (!haveCard) { shownCard = nextCard = v; haveCard = true; }
  if (!v.same(nextCard)) {  // a new item
    nextCard = v;
    if (v.urgent || shownCard.urgent) {  // urgent: straight in
      shownCard = v;
      transActive = false;
    } else {
      transActive = true;
      transAt = now;
      changesSincePeek++;
      transPeek = forcePeek ||
                  (changesSincePeek >= 3 && now - lastPeekAt > 120000 && (esp_random() % 2 == 0));
      if (transPeek) { lastPeekAt = now; changesSincePeek = 0; forcePeek = false; }
    }
  }
  nextCard.index = v.index;  // the dots can move without a fade
  nextCard.count = v.count;
  if (!transActive) {
    shownCard = nextCard;
    drawCard(shownCard, 1, 0);
    return;
  }
  uint32_t e = now - transAt, mid = transPeek ? PEEK_MS : 0;
  if (e < FADE_MS) {  // the old item fades out
    drawCard(shownCard, 1 - e / (float)FADE_MS, 0);
  } else if (e < FADE_MS + mid) {  // the eyes peek up from behind the card, look around, duck
    float p = (e - FADE_MS) / (float)mid;
    float rise = p < 0.22f ? smooth(p / 0.22f) : p > 0.78f ? 1 - smooth((p - 0.78f) / 0.22f) : 1;
    lookX = p < 0.25f ? 0 : p < 0.5f ? -7 * smooth((p - 0.25f) / 0.1f) : p < 0.75f ? 7 * smooth((p - 0.5f) / 0.12f) : 0;
    int dip = (int)(5 * sinf(PI * p));
    eyes(Mood::IDLE, cardX_ + (cardW_ ? cardW_ : W - 8) / 2, cardTop_ + 14 - (int)(30 * rise) + dip, 0.42f,
         AGENT, GROUND);
    lookX = NAN;
    drawCard(nextCard, 0, dip);  // empty card in front hides the lower half of the eyes
  } else if (e < 2 * FADE_MS + mid) {  // the new one fades in
    drawCard(nextCard, (e - FADE_MS - mid) / (float)FADE_MS, 0);
  } else {
    transActive = false;
    shownCard = nextCard;
    drawCard(shownCard, 1, 0);
  }
}

// Landscape home: the clock with the date and weather under it on the left, the card on the right.
static void homeTopWide(const Dash& d) {
  const int colW = CARD_X_WIDE - PAD - 6;  // the left column
  if (d.clock.length()) {
    canvas.setFont(F12);
    const int aw = canvas.textWidth(d.ampm);
    const lgfx::IFont* f = fit(d.clock, colW - aw - 3, {F44, F24});
    canvas.setFont(f);
    const int cw = canvas.textWidth(d.clock);
    txt(d.clock, PAD, 50, f, INK, middle_left);
    txt(d.ampm, PAD + cw + 3, f == F44 ? 64 : 58, F12, DIM, baseline_left);
  } else {
    txt(d.name, PAD, 44, fit(d.name, colW, {F24, F12}), INK, middle_left);
    paragraph("time comes from the Mac agent", F9, PAD, 62, colW, 2, FAINT, 12);
  }
  if (d.date.length()) txt(ellipsize(d.date, F12, colW), PAD, 86, F12, DIM, middle_left);
  if (d.wxValid) {
    String t = String(d.wxTemp) + "°";
    weatherIcon(d.wxIcon, d.wxDay, PAD, 99, 18);
    txt(t, PAD + 24, 108, F14, INK, middle_left);
    txt(ellipsize(d.wxCond, F9, colW), PAD, 124, F9, DIM, middle_left);
  }
}

void drawClassicHome(const Dash& d) {
  statusBar("", &d);
  const bool wide = orient_ != 0;
  cardTop_ = wide ? BAR_H + 4 : CARD_TOP;
  cardH_ = wide ? H - BAR_H - 8 : CARD_H;
  cardX_ = wide ? CARD_X_WIDE : 4;
  cardW_ = wide ? W - CARD_X_WIDE - 4 : 0;

  // Clock + date (the agent sends the time; until then just the name)
  if (wide) {
    homeTopWide(d);
  } else if (d.clock.length()) {
    canvas.setFont(F44);
    int cw = canvas.textWidth(d.clock);
    canvas.setFont(F12);
    int aw = canvas.textWidth(d.ampm);
    int x = (W - cw - 3 - aw) / 2;
    txt(d.clock, x, 64, F44, INK, middle_left);
    txt(d.ampm, x + cw + 3, 78, F12, DIM, baseline_left);
    txt(d.date, W / 2, 98, F12, DIM, middle_center);
  } else {
    txt(d.name, W / 2, 62, fit(d.name, W - 12, {F24, F12}), INK, middle_center);  // no time yet
    txt("time comes from the Mac agent", W / 2, 92, F9, FAINT, middle_center);
  }

  // Weather chip
  if (d.wxValid && !wide) {
    String t = String(d.wxTemp) + "°";
    canvas.setFont(F14);
    int tw = canvas.textWidth(t);
    canvas.setFont(F12);
    int cw = canvas.textWidth(d.wxCond);
    int w = 8 + 18 + 5 + tw + 5 + cw + 10, x = (W - w) / 2, y = 112;
    canvas.fillRoundRect(x, y, w, 24, 12, CHIP);
    weatherIcon(d.wxIcon, d.wxDay, x + 7, y + 3, 18);
    txt(t, x + 30, y + 12, F14, INK);
    txt(d.wxCond, x + 30 + tw + 5, y + 12, F12, DIM);
  }

  // The card: the stick's own news first (things the agent can't know), then the agent's
  CardView v;
  v.kind = d.cardKind; v.a = d.cardA; v.b = d.cardB; v.urgent = d.cardUrgent;
  v.index = d.cardIndex; v.count = d.cardCount;
  if (d.bt != 0) {
    v = CardView{}; v.own = true; v.kind = "hello"; v.urgent = true;
    v.a = d.bt == 1 ? "Waiting for the Mac" : "Bluetooth off";
    v.b = d.bt == 1 ? "pair or wake it" : "menu > Reconnect BT";
  } else if (d.battery >= 0 && d.battery <= 15 && !d.charging) {
    v = CardView{}; v.own = true; v.kind = "hello"; v.urgent = true;
    v.a = "Battery low"; v.b = String(d.battery) + "% - plug me in";
  } else if (!d.agentLinked) {
    v = CardView{}; v.own = true; v.kind = "hello"; v.a = "Agent offline"; v.b = "A still dictates";
  } else if (!v.kind.length() || v.kind == "face") {
    v = CardView{}; v.own = true; v.kind = "hello"; v.a = "Hold B to ask"; v.b = "A dictates";
  }
  homeCard(v);
}

// Status: a big battery number + level bar, the last hour (when there's enough
// of it), then the device's details. Everything on its own line. Landscape: the
// battery on the left, the details on the right (no last-hour graph).
static void drawStatus(const Dash& d) {
  static const char* BT[] = {"connected", "waiting", "off"};
  statusBar("Status", &d);
  const int pct = d.battery;
  const uint16_t lc = d.charging ? OK : pct >= 0 && pct < 15 ? ERR : pct >= 0 && pct < 30 ? AMBER : INK;
  const bool wide = orient_ != 0;
  // big number
  String num = pct >= 0 ? String(pct) : "--";
  canvas.setFont(F44);
  int nw = canvas.textWidth(num);
  txt(num, PAD, wide ? 44 : 50, F44, lc, middle_left);
  txt("%", PAD + nw + 2, wide ? 54 : 60, F18, blend(lc, GROUND, 0.3f), baseline_left);
  // state + time, right-aligned beside it
  int eta = battery::etaMinutes();
  String state = d.charging ? "charging" : battery::full() ? "full" : battery::usbPower() ? "on USB" : "on battery";
  String when = "";
  if (eta > 0 && eta < 24 * 60) {
    String t = eta >= 60 ? String(eta / 60) + "h " + String(eta % 60) + "m" : String(eta) + " min";
    when = d.charging ? "full in " + t : t + " left";
  }
  if (wide) {
    const int colW = 96, x0 = 114;
    txt(state, PAD, 74, F12, d.charging ? OK : DIM);
    if (when.length()) txt(when, PAD, 90, F9, DIM);
    canvas.fillRoundRect(PAD, 102, colW, 8, 4, CHIP);
    if (pct >= 0) canvas.fillRoundRect(PAD, 102, max(8, colW * pct / 100), 8, 4, lc);
    auto rowW = [&](const char* label, const String& value, int y, uint16_t color) {
      txt(label, x0, y, F9, DIM);
      int room = W - PAD - x0 - canvas.textWidth(label) - 6;
      const lgfx::IFont* f = fit(value, room, {F12, F9});
      txt(ellipsize(value, f, room), W - PAD, y, f, color, middle_right);
    };
    rowW("Mac", String("slot ") + d.slot + " - " + BT[d.bt], 32, d.bt == 0 ? INK : ASK);
    rowW("Agent", d.agentLinked ? "linked" : "offline", 52, d.agentLinked ? AGENT : DIM);
    rowW("Wi-Fi", d.wifi, 72, d.wifi == "off" ? DIM : INK);
    rowW("Uptime", fmtUptime(d.uptimeS), 92, INK);
    txt(ellipsize(d.name + "  -  fw " + d.fw, F9, W - PAD - x0), x0 + (W - PAD - x0) / 2, 114, F9, FAINT, middle_center);
    return;
  }
  txt(state, W - PAD, when.length() ? 40 : 48, F12, d.charging ? OK : DIM, middle_right);
  if (when.length()) txt(when, W - PAD, 56, F9, DIM, middle_right);
  // level bar
  const int bx = PAD, by = 78, bw = W - 2 * PAD;
  canvas.fillRoundRect(bx, by, bw, 8, 4, CHIP);
  if (pct >= 0) canvas.fillRoundRect(bx, by, max(8, bw * pct / 100), 8, 4, lc);

  // last hour, only once there's enough to draw
  uint8_t rp[battery::RECENT];
  bool rc[battery::RECENT];
  int n = battery::recent(rp, rc);
  int y = 104;
  if (n >= 10) {
    const int x0 = PAD, x1 = W - PAD, y0 = 96, y1 = 136;
    int lo = 100, hi = 0;
    for (int i = 0; i < n; i++) { lo = min(lo, (int)rp[i]); hi = max(hi, (int)rp[i]); }
    if (hi - lo < 6) { int mid = (hi + lo) / 2; lo = max(0, mid - 3); hi = min(100, lo + 6); }
    auto px = [&](int i) { return x0 + (x1 - x0) * i / (battery::RECENT - 1); };
    auto py = [&](int v) { return y1 - (y1 - y0) * (v - lo) / max(1, hi - lo); };
    int off = battery::RECENT - n;  // newest at the right edge
    for (int i = 0; i < n; i++)
      canvas.drawFastVLine(px(i + off), py(rp[i]), y1 - py(rp[i]), blend(GROUND, rc[i] ? OK : lc, 0.18f));
    for (int i = 1; i < n; i++)
      canvas.drawWideLine(px(i - 1 + off), py(rp[i - 1]), px(i + off), py(rp[i]), 1, rc[i] ? OK : lc);
    txt("1h", x0, y1 + 7, F9, FAINT);
    txt("now", x1, y1 + 7, F9, FAINT, middle_right);
    y = 158;
  }
  const int step = n >= 10 ? 17 : 21;
  row("Mac", String("slot ") + d.slot + " - " + BT[d.bt], y, d.bt == 0 ? INK : ASK); y += step;
  row("Agent", d.agentLinked ? "linked" : "offline", y, d.agentLinked ? AGENT : DIM); y += step;
  row("Wi-Fi", d.wifi, y, d.wifi == "off" ? DIM : INK);   y += step;
  row("Uptime", fmtUptime(d.uptimeS), y);
  txt(d.name + "  -  fw " + d.fw, W / 2, H - 20, F9, FAINT, middle_center);
}

static void drawAgentPage(const Dash& d) {
  if (!d.agentLinked) {
    moodScreen(Mood::OFFLINE, "Agent offline", "start it on the Mac: stickybot start", "", true, "Agent", &d, 10);
  } else if (!d.agentHeard.length() && !d.agentText.length()) {
    moodScreen(Mood::IDLE, "Hold B and speak", "\"open PR 1234\"  -  \"what's on my calendar?\"", "", true, "Agent", &d,
               10);
  } else {
    moodScreen(Mood::IDLE, "", "\"" + d.agentHeard + "\"", d.agentText, true, "Agent", &d, 10);
  }
}

void drawPage(Page p, const Dash& d) {
  wide();  // landscape when held sideways
  canvas.fillScreen(GROUND);
  switch (p) {
    case Page::HOME:    faces::drawHome(d, orient_ != 0); break;
    case Page::AGENT:   drawAgentPage(d); break;
    case Page::STATUS:  drawStatus(d); break;
    default: break;
  }
  pageDots(p);
  canvas.pushSprite(0, 0);
}

void drawFaceDemo(const Dash& d, const String& caption) {
  wide();
  canvas.fillScreen(GROUND);
  faces::drawHome(d, orient_ != 0);
  const lgfx::IFont* f = fit(caption, W - 16, {F12, F9});
  canvas.setFont(f);
  const int w = min(W - 8, (int)canvas.textWidth(caption) + 14), y = H - 22;
  canvas.fillRoundRect((W - w) / 2, y - 8, w, 17, 8, blend(GROUND, WISPR, 0.55f));
  txt(ellipsize(caption, f, w - 10), W / 2, y, f, INK, middle_center);
  canvas.pushSprite(0, 0);
}

// --- Live screens ------------------------------------------------------------------------

// A glowing orb that swells with the mic level, with a ring pulsing outwards.
static void orb(int cx, int cy, uint16_t c) {
  const uint32_t t = millis();
  int r = (int)(30 * (0.78f + level_ * 0.38f));
  // glow
  for (int g = 3; g >= 1; g--) canvas.fillCircle(cx, cy, r + g * (4 + (int)(level_ * 5)), blend(GROUND, c, 0.07f * (4 - g)));
  // pulsing ring: grows from 0.85x to 1.35x and fades, every 1.6 s
  float ph = (t % 1600) / 1600.0f;
  int rr = (int)((r + 9) * (0.85f + 0.5f * ph));
  canvas.drawCircle(cx, cy, rr, blend(c, GROUND, ph));
  // body: dark rim to bright centre
  for (int i = 0; i < 6; i++) canvas.fillCircle(cx, cy, r - i * r / 7, blend(blend(c, TFT_BLACK, 0.55f), c, i / 5.0f));
  // highlight
  canvas.fillCircle(cx - r / 4, cy - r / 3, r / 5, blend(c, TFT_WHITE, 0.55f));
}

// A row of n bars following the mic level, from x0 to x1.
static void meter(int y, uint16_t c, int x0, int x1, int n = 22) {
  const int gap = 2, w = (x1 - x0 - (n - 1) * gap) / n;
  const uint32_t t = millis();
  for (int i = 0; i < n; i++) {
    float wob = 0.5f + 0.5f * sinf(t / 110.0f + i * 0.9f);
    float centre = 1.0f - fabsf(i - (n - 1) / 2.0f) / (n / 2.0f);  // taller in the middle
    int h = 3 + (int)(level_ * 20 * (0.35f + 0.65f * centre) * (0.6f + 0.4f * wob));
    canvas.fillRoundRect(x0 + i * (w + gap), y - h / 2, w, h, 1, h > 6 ? c : blend(GROUND, c, 0.45f));
  }
}

// Button A held: dictation.
void drawHolding(const Combo& c, bool usbMic, bool wirelessMic) {
  bool landscape = wide();
  canvas.fillScreen(GROUND);
  String parts[5];
  int n = comboParts(c, parts, 5);
  String one;
  for (int i = 0; i < n; i++) one += (i ? "+" : "") + parts[i];
  bool stickMic = usbMic || wirelessMic;
  if (landscape) {  // the orb on the left, the words and the meter on the right
    const int x0 = 118, room = W - PAD - x0;
    orb(58, 68, WISPR);
    txt("Dictating", x0, 24, F18, WISPR);
    txt(ellipsize(one + " held", F12, room), x0, 46, F12, INK);
    txt(stickMic ? "stick mic on" : "Mac's mic", x0, 62, F9, DIM);
    meter(88, WISPR, x0, W - PAD, 14);
    paragraph(usbMic ? "Wispr hears the stick (USB)" : wirelessMic ? "Wispr hears the stick (BT)" : "plug in USB for the stick mic",
              F9, x0, 106, room, 2, FAINT, 13);
    canvas.pushSprite(0, 0);
    return;
  }
  txt("Dictating", W / 2, 24, F18, WISPR, middle_center);
  txt(one + " held - " + (stickMic ? "stick mic on" : "Mac's mic"), W / 2, 46, F9, DIM, middle_center);
  orb(W / 2, 112, WISPR);
  meter(186, WISPR, PAD + 4, W - PAD - 4);
  txt(usbMic ? "Wispr hears the stick (USB)" : wirelessMic ? "Wispr hears the stick (BT)" : "plug in USB for the stick mic",
      W / 2, 216, F9, FAINT, middle_center);
  canvas.pushSprite(0, 0);
}

// Button B held: talking to the agent.
void drawTalk(bool linked) {
  wide();
  if (!linked) moodScreen(Mood::OFFLINE, "Agent offline", "start it on the Mac", "", false, "", nullptr, 0);
  else moodScreen(Mood::LISTEN, "Listening", "release to send", "", false, "", nullptr, 0);
  canvas.pushSprite(0, 0);
}

static String dots(const String& word) {
  String t = word;
  for (int i = 0, n = (millis() / 400) % 4; i < n; i++) t += ".";
  return t;
}

void drawAgent(const String& state, const String& heard, const String& text) {
  wide();
  String h = heard.length() ? "\"" + heard + "\"" : "";
  if (state == "done") moodScreen(Mood::DONE, "", h, text, false, "", nullptr, 0);
  else if (state == "error") moodScreen(Mood::ERROR, "Hmm", h, text, false, "", nullptr, 0);
  else if (state == "working") moodScreen(Mood::WORK, dots("Working"), h, text, false, "", nullptr, 0);
  else if (state == "listening") moodScreen(Mood::LISTEN, "Listening", h, "", false, "", nullptr, 0);
  else moodScreen(Mood::THINK, dots("Thinking"), h, text, false, "", nullptr, 0);
  canvas.pushSprite(0, 0);
}

void drawMood(int mood, const String& label) {
  wide();
  moodScreen((Mood)constrain(mood, 0, (int)Mood::COUNT - 1), label, "", "", false, "", nullptr, 0);
  canvas.pushSprite(0, 0);
}

// Landscape: the tool and who asks on the top line, the command in a wide box, the
// buttons along the bottom with A at the end nearest the real button.
static void promptWide(const String& tool, const String& hint, const String& from) {
  String who = from.length() ? from + " asks" : "Claude asks";
  canvas.setFont(F12);
  int ww = canvas.textWidth(who);
  txt(who, W - PAD, 15, F12, ON_ASK, middle_right);
  String t = tool.length() ? tool : "Permission";
  const int tRoom = W - 2 * PAD - ww - 8;
  const lgfx::IFont* tf = fit(t, tRoom, {F24, F18, F14});
  txt(ellipsize(t, tf, tRoom), PAD, 15, tf, ON_ASK);
  const int boxY = 30, boxH = 68, inner = W - 2 * PAD - 4;
  canvas.fillRoundRect(PAD - 4, boxY, W - 2 * (PAD - 4), boxH, 8, ON_ASK);
  const lgfx::IFont* hf = fitBox(hint, inner, boxH - 10, {F12, F9});
  paragraph(hint, hf, PAD + 2, boxY + 6, inner, (boxH - 10) / lineH(hf), rgb(0xff, 0xe2, 0xb8), lineH(hf));
  // A (approve) at its end, B (deny) at the other
  const int a = aEnd(), cw = 78, ch = 28, cy = 103;
  const int ax = a > 0 ? W - 4 - cw : 4, bx = a > 0 ? 4 : W - 4 - cw;
  keyChip("A  yes", ax, cy, cw, ch, ON_ASK, ASK, true, a);
  keyChip("B  no", bx, cy, cw, ch, ON_ASK, ASK, false, 0);
  txt("no answer:", W / 2, cy + 8, F9, ON_ASK, middle_center);
  txt("asks on the Mac", W / 2, cy + 20, F9, ON_ASK, middle_center);
}

// Claude Code permission prompt: the whole screen turns orange.
void drawPrompt(const String& tool, const String& hint, const String& from) {
  bool landscape = wide();
  canvas.fillScreen(ASK);
  if (landscape) {
    promptWide(tool, hint, from);
    canvas.pushSprite(0, 0);
    return;
  }
  txt(from.length() ? from + " asks" : "Claude asks", W / 2, 18, F12, ON_ASK, middle_center);
  String t = tool.length() ? tool : "Permission";
  txt(t, W / 2, 46, fit(t, W - 2 * PAD, {F24, F18, F14}), ON_ASK, middle_center);
  canvas.fillRoundRect(PAD - 4, 66, W - 2 * (PAD - 4), 108, 8, ON_ASK);
  paragraph(hint, F9, PAD + 2, 74, W - 2 * PAD - 4, 7, rgb(0xff, 0xe2, 0xb8), 14);
  // Buttons: A (front) approves, B (side) denies
  int y = flipped_ ? 196 : 200;
  canvas.fillRoundRect(PAD - 4, y - 14, 58, 32, 16, ON_ASK);
  txt("A  yes", PAD + 25, y + 2, F12, ASK, middle_center);
  canvas.drawRoundRect(W - PAD - 54, y - 14, 58, 32, 16, ON_ASK);
  txt("B  no", W - PAD - 25, y + 2, F12, ON_ASK, middle_center);
  txt("no answer: asks on the Mac", W / 2, 232, F9, ON_ASK, middle_center);
  canvas.pushSprite(0, 0);
}

// A question from a Claude session (the agent's stick tools): orange like an approval, but the
// question gets the room and each button its own row with the option's label.
void drawAsk(const String& question, const String& from, const String& a, const String& b) {
  bool landscape = wide();
  canvas.fillScreen(ASK);
  if (landscape) {  // the question in a wide box, the two answers side by side (A at its end)
    txt(from.length() ? from + " asks" : "Claude asks", PAD, 12, F12, ON_ASK);
    txt("shake: dismiss", W - PAD, 12, F9, ON_ASK, middle_right);
    const int boxY = 24, boxH = 74, inner = W - 2 * PAD - 4;
    canvas.fillRoundRect(PAD - 4, boxY, W - 2 * (PAD - 4), boxH, 8, ON_ASK);
    const lgfx::IFont* qf = fitBox(question, inner, boxH - 10, {F14, F12, F9});
    paragraph(question, qf, PAD + 2, boxY + 6, inner, (boxH - 10) / lineH(qf), rgb(0xff, 0xe2, 0xb8), lineH(qf));
    String la = String(prompt::button('A', a.c_str(), "yes", 16).c_str());
    String lb = String(prompt::button('B', b.c_str(), b.length() || !a.length() ? "no" : "dismiss", 16).c_str());
    const int e = aEnd(), cw = (W - 8 - 6) / 2, ch = 30, cy = 102;
    const int ax = e > 0 ? W - 4 - cw : 4, bx = e > 0 ? 4 : W - 4 - cw;
    keyChip(la, ax, cy, cw, ch, ON_ASK, ASK, true, e);
    keyChip(lb, bx, cy, cw, ch, ON_ASK, ASK, false, 0);
    canvas.pushSprite(0, 0);
    return;
  }
  txt(from.length() ? from + " asks" : "Claude asks", W / 2, 18, F12, ON_ASK, middle_center);
  canvas.fillRoundRect(PAD - 4, 34, W - 2 * (PAD - 4), 122, 8, ON_ASK);
  paragraph(question, F12, PAD + 2, 42, W - 2 * PAD - 4, 6, rgb(0xff, 0xe2, 0xb8), 18);
  String la = String(prompt::button('A', a.c_str(), "yes", 16).c_str());
  String lb = String(prompt::button('B', b.c_str(), b.length() || !a.length() ? "no" : "dismiss", 16).c_str());
  int y = flipped_ ? 168 : 172;
  canvas.fillRoundRect(PAD - 4, y - 13, W - 2 * (PAD - 4), 26, 13, ON_ASK);
  txt(la, W / 2, y, fit(la, W - 2 * PAD, {F12, F9}), ASK, middle_center);
  canvas.drawRoundRect(PAD - 4, y + 17, W - 2 * (PAD - 4), 26, 13, ON_ASK);
  txt(lb, W / 2, y + 30, fit(lb, W - 2 * PAD, {F12, F9}), ON_ASK, middle_center);
  txt("shake: dismiss", W / 2, 232, F9, ON_ASK, middle_center);
  canvas.pushSprite(0, 0);
}

void drawPairing(uint32_t code) {
  bool landscape = wide();
  canvas.fillScreen(GROUND);
  statusBar("Pair with Mac?", nullptr, ASK);
  char c[8];
  snprintf(c, sizeof c, "%03lu %03lu", (unsigned long)(code / 1000), (unsigned long)(code % 1000));
  if (landscape) {  // the code in the middle, accept at A's end, reject at the other
    txt("Code on the Mac:", W / 2, 32, F12, DIM, middle_center);
    card(PAD - 4, 42, W - 2 * (PAD - 4), 44);
    txt(c, W / 2, 64, fit(c, W - 2 * PAD, {F24, F18}), INK, middle_center);
    txt("Same number?", W / 2, 98, F12, DIM, middle_center);
    const int e = aEnd();
    chip("A  accept", e > 0 ? W * 3 / 4 : W / 4, 120, OK);
    chip("B  reject", e > 0 ? W / 4 : W * 3 / 4, 120, ERR);
    canvas.pushSprite(0, 0);
    return;
  }
  txt("Code on the Mac:", W / 2, 58, F12, DIM, middle_center);
  card(PAD - 4, 76, W - 2 * (PAD - 4), 56);
  txt(c, W / 2, 104, fit(c, W - 2 * PAD, {F24, F18}), INK, middle_center);
  txt("Same number?", W / 2, 152, F12, DIM, middle_center);
  chip("A  accept", W / 2, 186, OK);
  chip("B  reject", W / 2, 216, ERR);
  canvas.pushSprite(0, 0);
}

void drawMenu(const MenuView& m) {
  dockEnd();  // normal screens are portrait
  constexpr int ROW = 20, TOP = BAR_H + 6;
  const int visible = (H - TOP - 22) / ROW;
  canvas.fillScreen(GROUND);
  statusBar(m.title, nullptr);

  int first = m.selected - visible / 2;
  if (first > m.count - visible) first = m.count - visible;
  if (first < 0) first = 0;

  for (int i = first; i < m.count && i < first + visible; i++) {
    int y = TOP + (i - first) * ROW;
    bool sel = i == m.selected;
    if (sel) {
      canvas.fillRoundRect(3, y, W - 6, ROW - 2, 6, CHIP);
      canvas.fillRoundRect(3, y + 4, 3, ROW - 10, 1, WISPR);
    }
    const lgfx::IFont* f = fit(m.lines[i], W - 2 * PAD - 4, {F12, F9});
    txt(m.lines[i], PAD + 2, y + ROW / 2 - 1, f, sel ? INK : DIM);
  }
  if (first > 0) canvas.fillTriangle(W / 2 - 4, TOP - 2, W / 2 + 4, TOP - 2, W / 2, TOP - 6, FAINT);
  if (first + visible < m.count) {
    int ty = TOP + visible * ROW + 1;
    canvas.fillTriangle(W / 2 - 4, ty, W / 2 + 4, ty, W / 2, ty + 4, FAINT);
  }
  txt(m.hint ? m.hint : "", W / 2, H - 9, F9, FAINT, middle_center);
  canvas.pushSprite(0, 0);
}

// Lines ending in ':' are labels (grey); the others are values (white). Landscape:
// each label with its value on one row.
void drawInfo(const char* title, const String lines[], int count, const char* hint) {
  bool landscape = wide();
  canvas.fillScreen(GROUND);
  statusBar(title, nullptr);
  if (landscape) {
    card(PAD - 4, BAR_H + 4, W - 2 * (PAD - 4), H - BAR_H - 22);
    const int vx = 92;
    int y = BAR_H + 18;
    String label;
    for (int i = 0; i < count && y < H - 20; i++) {
      if (lines[i].endsWith(":")) { label = lines[i]; continue; }
      int x = label.length() ? vx : PAD + 2, room = W - PAD - 2 - x;
      if (label.length()) txt(ellipsize(label, F9, vx - PAD - 6), PAD + 2, y, F9, DIM);
      const lgfx::IFont* f = fit(lines[i], room, {F14, F12, F9});
      txt(ellipsize(lines[i], f, room), x, y, f, INK);
      label = "";
      y += 26;
    }
    txt(hint, W / 2, H - 9, F9, FAINT, middle_center);
    canvas.pushSprite(0, 0);
    return;
  }
  card(PAD - 4, BAR_H + 6, W - 2 * (PAD - 4), H - BAR_H - 34);
  int y = BAR_H + 22;
  for (int i = 0; i < count; i++) {
    bool label = lines[i].endsWith(":");
    if (label) {
      txt(lines[i], PAD + 2, y, F9, DIM);
      y += 16;
    } else {
      txt(lines[i], PAD + 2, y, fit(lines[i], W - 2 * PAD - 4, {F12, F9}), INK);
      y += 24;
    }
  }
  txt(hint, W / 2, H - 12, F9, FAINT, middle_center);
  canvas.pushSprite(0, 0);
}

// A short message (saved, restarting, standby...). `accent` colours the title.
void drawMessage(const char* title, const char* sub, uint16_t accent) {
  bool landscape = wide();
  canvas.fillScreen(GROUND);
  // The callers pass dark status colours; brighten them for text
  uint16_t c = accent == 0x03E0 ? OK : accent == 0xC300 ? ASK : accent == 0x2104 ? DIM : INK;
  if (landscape) {  // centred; the wide lines need fewer of them
    String tl[2], sl[3];
    int tn = wrap(title, F18, tl, 2, W - 2 * PAD), sn = sub ? wrap(sub, F12, sl, 3, W - 2 * PAD) : 0;
    int y = (H - (9 + tn * 24 + sn * 16)) / 2;
    canvas.fillRoundRect(W / 2 - 18, y, 36, 5, 2, c);
    y += 9 + 12;
    for (int i = 0; i < tn; i++, y += 24) txt(tl[i], W / 2, y, F18, INK, middle_center);
    for (int i = 0; i < sn; i++, y += 16) txt(sl[i], W / 2, y - 4, F12, DIM, middle_center);
    canvas.pushSprite(0, 0);
    return;
  }
  canvas.fillRoundRect(W / 2 - 18, 62, 36, 5, 2, c);
  String tl[3];
  int tn = wrap(title, F18, tl, 3, W - 2 * PAD);
  for (int i = 0; i < tn; i++) txt(tl[i], W / 2, 96 + i * 24, F18, INK, middle_center);
  if (sub) paragraph(sub, F12, PAD, 104 + tn * 24, W - 2 * PAD, 5, DIM, 16);
  canvas.pushSprite(0, 0);
}


// Firmware update over Bluetooth: the percentage, a bar, and how to cancel.
void drawUpdate(int pct, const String& version, const char* line) {
  dockEnd();  // normal screens are portrait
  canvas.fillScreen(GROUND);
  pct = constrain(pct, 0, 100);
  txt("Updating", W / 2, 40, F18, INK, middle_center);
  if (version.length()) txt("to " + version, W / 2, 62, F12, DIM, middle_center);
  String n(pct);
  canvas.setFont(F44);
  int nw = canvas.textWidth(n.c_str());
  canvas.setFont(F18);
  int pw = canvas.textWidth("%");
  int x0 = (W - nw - pw - 2) / 2;
  txt(n, x0, 118, F44, AGENT, middle_left);
  txt("%", x0 + nw + 2, 126, F18, AGENT, middle_left);
  int bx = PAD + 2, bw = W - 2 * (PAD + 2), by = 156;
  canvas.fillRoundRect(bx, by, bw, 8, 4, CHIP);
  if (pct) canvas.fillRoundRect(bx, by, max(8, bw * pct / 100), 8, 4, AGENT);
  if (line) paragraph(line, F12, PAD, 182, W - 2 * PAD, 2, DIM, 16);
  txt("B: cancel", W / 2, H - 12, F9, FAINT, middle_center);
  canvas.pushSprite(0, 0);
}

// --- Event screens (celebrate / nice / headsup) -----------------------------------

// Big icons drawn with primitives, centred on cx, cy, about `size` px across.
static void eventIcon(const String& k, int cx, int cy, int size, uint16_t c, uint16_t bg) {
  const float s = size / 64.0f, w = max(1.5f, 3.2f * s);
  auto L = [&](float x0, float y0, float x1, float y1) {
    canvas.drawWideLine(cx + x0 * s, cy + y0 * s, cx + x1 * s, cy + y1 * s, w, c);
  };
  auto node = [&](float x, float y) {
    canvas.fillCircle(cx + x * s, cy + y * s, 7 * s, c);
    canvas.fillCircle(cx + x * s, cy + y * s, 3.5f * s, bg);
  };
  if (k == "merge") {  // git merge: two nodes on a line, a third merging in
    L(-12, -20, -12, 20);
    L(14, 6, 14, -2);
    L(14, -2, -6, -16);
    node(-12, -22); node(-12, 22); node(14, 10);
  } else if (k == "pr") {  // pull request: branch line + arrow back into it
    L(-14, -18, -14, 18);
    L(14, 18, 14, -10);
    L(14, -10, 2, -18);
    L(2, -18, 8, -24);
    L(2, -18, 8, -12);
    node(-14, -22); node(-14, 22); node(14, 22);
  } else if (k == "check") {
    canvas.drawWideLine(cx - 18 * s, cy + 1 * s, cx - 5 * s, cy + 14 * s, 5 * s, c);
    canvas.drawWideLine(cx - 5 * s, cy + 14 * s, cx + 20 * s, cy - 14 * s, 5 * s, c);
  } else if (k == "x") {
    canvas.drawWideLine(cx - 16 * s, cy - 16 * s, cx + 16 * s, cy + 16 * s, 5 * s, c);
    canvas.drawWideLine(cx - 16 * s, cy + 16 * s, cx + 16 * s, cy - 16 * s, 5 * s, c);
  } else if (k == "review") {  // an eye
    canvas.fillEllipse(cx, cy, 26 * s, 15 * s, c);
    canvas.fillEllipse(cx, cy, 22 * s, 11 * s, bg);
    canvas.fillCircle(cx, cy, 9 * s, c);
    canvas.fillCircle(cx + 3 * s, cy - 3 * s, 3 * s, bg);
  } else if (k == "claude") {  // sparkle
    for (int i = 0; i < 8; i++) {
      float a = i * PI / 4, r = (i & 1) ? 14 : 24;
      canvas.drawWideLine(cx, cy, cx + cosf(a) * r * s, cy + sinf(a) * r * s, (i & 1) ? w * 0.8f : w * 1.2f, c);
    }
    canvas.fillCircle(cx, cy, 5 * s, c);
  } else if (k == "star") {  // five-point star
    float px[10], py[10];
    for (int i = 0; i < 10; i++) {
      float a = -PI / 2 + i * PI / 5, r = (i & 1) ? 11 : 26;
      px[i] = cx + cosf(a) * r * s;
      py[i] = cy + sinf(a) * r * s;
    }
    for (int i = 0; i < 10; i++) canvas.fillTriangle(cx, cy, px[i], py[i], px[(i + 1) % 10], py[(i + 1) % 10], c);
  } else {  // bell
    canvas.fillCircle(cx, cy - 6 * s, 14 * s, c);
    canvas.fillRect(cx - 14 * s, cy - 6 * s, 28 * s, 16 * s, c);
    canvas.fillTriangle(cx - 22 * s, cy + 16 * s, cx + 22 * s, cy + 16 * s, cx, cy - 4 * s, c);
    canvas.fillRoundRect(cx - 22 * s, cy + 12 * s, 44 * s, 6 * s, 3 * s, c);
    canvas.fillCircle(cx, cy + 22 * s, 4 * s, c);
    canvas.fillCircle(cx, cy - 22 * s, 3 * s, c);
  }
}

// Confetti: a burst from the middle, then a gentle rain from the top.
struct Bit { float x, y, vx, vy, spin; uint16_t c; bool on; };
static Bit bits[48];
static uint32_t confettiLast = 0;

void confettiReset(float x, float y) {  // the burst starts at x, y (the icon)
  static const uint16_t COLORS[] = {WISPR, AGENT, ASK, OK, ERR, SUN, RAIN, INK};
  for (int i = 0; i < 48; i++) {
    Bit& b = bits[i];
    float a = (esp_random() % 360) * PI / 180, v = 1.5f + (esp_random() % 100) / 25.0f;
    b = {x, y, cosf(a) * v, sinf(a) * v - 2.5f, (float)(esp_random() % 628) / 100, COLORS[i % 8], true};
  }
  confettiLast = millis();
}

void confettiDraw(bool rain) {
  uint32_t now = millis();
  float dt = constrain((now - confettiLast) / 40.0f, 0.2f, 3.0f);  // in 40 ms frames
  confettiLast = now;
  for (auto& b : bits) {
    if (!b.on) continue;
    b.vy += 0.18f * dt;                 // gravity
    b.vx *= 0.985f;
    b.x += b.vx * dt;
    b.y += b.vy * dt;
    b.spin += 0.35f * dt;
    if (b.y > H + 6 || b.x < -6 || b.x > W + 6) {
      if (rain) {  // respawn at the top
        b.x = esp_random() % W;
        b.y = -4;
        b.vx = ((int)(esp_random() % 100) - 50) / 60.0f;
        b.vy = 0.8f + (esp_random() % 100) / 80.0f;
      } else {
        b.on = false;
      }
      continue;
    }
    int ww = 1 + (int)(fabsf(sinf(b.spin)) * 4), hh = 5 - (ww > 3 ? 2 : 0);
    canvas.fillRect((int)b.x - ww / 2, (int)b.y - hh / 2, ww, hh, b.c);
  }
}

float easeOutBack(float t) {
  t = constrain(t, 0.0f, 1.0f);
  const float c1 = 1.70158f, c3 = c1 + 1;
  return 1 + c3 * powf(t - 1, 3) + c1 * powf(t - 1, 2);
}

static void centredLines(const String& text, const lgfx::IFont* f, int cy, int maxLines, uint16_t color, int lh) {
  centredLines(text, f, W / 2, cy, W - 2 * PAD, maxLines, color, lh);
}

// Landscape events: the icon on the left, the title and the line under it on the right
// (the block centred on cy). shadow: a dark copy under the title, for busy backgrounds.
constexpr int EV_X = 54, EV_TX = 106;  // icon centre, text column start
static void eventText(const String& title, std::initializer_list<const lgfx::IFont*> fonts, const String& sub,
                      int cy, uint16_t ink, uint16_t subInk, bool shadow) {
  const int w = W - PAD - EV_TX, cx = EV_TX + w / 2;
  const lgfx::IFont* f = fitBox(title, w, 2 * 24, fonts);
  String tl[2], sl[3];
  int tn = wrap(title, f, tl, 2, w), sn = sub.length() ? wrap(sub, F12, sl, 3, w) : 0;
  const int th = lineH(f), gap = sn ? 6 : 0;
  int y = cy - (tn * th + gap + sn * 15) / 2 + th / 2;
  for (int i = 0; i < tn; i++, y += th) {
    if (shadow) txt(tl[i], cx + 1, y + 1, f, GROUND, middle_center);
    txt(tl[i], cx, y, f, ink, middle_center);
  }
  y += gap - th / 2 + 7;
  for (int i = 0; i < sn; i++, y += 15) txt(sl[i], cx, y, F12, subInk, middle_center);
}

void drawEvent(uint8_t level, const String& title, const String& sub, const String& icon, uint32_t ms) {
  const bool landscape = wide();
  const float t = ms / 1000.0f;
  // where the icon goes: portrait above the text, landscape on its left
  const int ix = landscape ? EV_X : W / 2;
  if (level == 0) {
    // CELEBRATE: colour pulse, white flash at the start, confetti, party face, big icon
    const int iy = landscape ? 80 : 92;
    static uint32_t lastMs = UINT32_MAX;
    if (ms < lastMs || lastMs == UINT32_MAX) confettiReset(ix, iy);
    lastMs = ms;
    static const uint16_t HUES[] = {WISPR, AGENT, ASK, OK};
    uint16_t hue = HUES[(ms / 450) % 4];
    float pulse = 0.22f + 0.12f * sinf(t * 9);
    uint16_t bg = ms < 140 ? blend(INK, hue, ms / 140.0f) : blend(GROUND, hue, pulse);
    canvas.fillScreen(bg);
    confettiDraw(ms > 1200);
    eyes(Mood::PARTY, ix, landscape ? 24 : 30, landscape ? 0.36f : 0.42f, INK, bg);
    float pop = easeOutBack(ms / 450.0f);
    int r = (int)((landscape ? 30 : 34) * pop);
    if (r > 2) {
      canvas.fillCircle(ix, iy, r + 4, blend(hue, TFT_WHITE, 0.35f));
      canvas.fillCircle(ix, iy, r, GROUND);
      eventIcon(icon, ix, iy, (int)((landscape ? 48 : 54) * pop), blend(hue, TFT_WHITE, 0.25f), GROUND);
    }
    if (landscape) {
      eventText(title, {F24, F18, F14}, sub, H / 2, INK, blend(INK, bg, 0.15f), true);
      canvas.pushSprite(0, 0);
      return;
    }
    const lgfx::IFont* f = fit(title, W - 2 * PAD, {F24, F18, F14});
    String tl[3];
    int n = wrap(title, f, tl, 2, W - 2 * PAD);
    int y = n > 1 ? 150 : 160;
    for (int i = 0; i < n; i++) {  // text with a dark shadow so it reads on any colour
      txt(tl[i], W / 2 + 1, y + i * 24 + 1, f, GROUND, middle_center);
      txt(tl[i], W / 2, y + i * 24, f, INK, middle_center);
    }
    centredLines(sub, F12, 206, 2, blend(INK, bg, 0.15f), 15);
  } else if (level == 1) {
    // NICE: teal -> green wash sliding in, popping icon, a countdown bar
    float in = constrain(ms / 260.0f, 0.0f, 1.0f);
    int top = (int)((1 - in) * H);
    canvas.fillScreen(GROUND);
    for (int y = top; y < H; y += 4) canvas.fillRect(0, y, W, 4, blend(AGENT, OK, (float)y / H * 0.8f));
    const uint16_t ink = rgb(0x05, 0x26, 0x22);
    if (in >= 1) {
      float pop = easeOutBack((ms - 260) / 380.0f);
      int r = (int)(38 * pop), iy = landscape ? H / 2 - 4 : 84;
      if (r > 2) {
        canvas.fillCircle(ix, iy, r, ink);
        eventIcon(icon, ix, iy, (int)(54 * pop), blend(AGENT, TFT_WHITE, 0.3f), ink);
      }
      int bw = (int)((W - 2 * PAD) * (1 - constrain(ms / 4000.0f, 0.0f, 1.0f)));
      if (landscape) {
        eventText(title, {F18, F14, F12}, sub, H / 2 - 4, ink, blend(ink, AGENT, 0.25f), false);
        canvas.fillRoundRect(PAD, H - 8, bw, 4, 2, ink);
        canvas.pushSprite(0, 0);
        return;
      }
      centredLines(title, fit(title, W - 2 * PAD, {F18, F14, F12}), 152, 2, ink, 21);
      centredLines(sub, F12, 192, 3, blend(ink, AGENT, 0.25f), 15);
      canvas.fillRoundRect(PAD, H - 10, bw, 4, 2, ink);
    }
  } else {
    // HEADSUP: orange, a gentle pulse and a ring breathing round the icon; stays until a press
    float p = 0.5f + 0.5f * sinf(t * 2.6f);
    uint16_t bg = blend(ASK, rgb(0xff, 0xb8, 0x4d), p * 0.6f);
    canvas.fillScreen(bg);
    float pop = easeOutBack(ms / 400.0f);
    int r = (int)(36 * pop), iy = landscape ? H / 2 - 6 : 82;
    if (r > 2) {
      canvas.drawCircle(ix, iy, r + 6 + (int)(p * 5), ON_ASK);
      canvas.fillCircle(ix, iy, r, ON_ASK);
      eventIcon(icon, ix, iy, (int)(52 * pop), ASK, ON_ASK);
    }
    if (landscape) {
      eventText(title, {F18, F14, F12}, sub, H / 2 - 8, ON_ASK, rgb(0x4a, 0x2c, 0x05), false);
      txt("press any button", EV_TX + (W - PAD - EV_TX) / 2, H - 10, F9, ON_ASK, middle_center);
      canvas.pushSprite(0, 0);
      return;
    }
    centredLines(title, fit(title, W - 2 * PAD, {F18, F14, F12}), 148, 2, ON_ASK, 21);
    centredLines(sub, F12, 188, 3, rgb(0x4a, 0x2c, 0x05), 15);
    txt("press any button", W / 2, H - 12, F9, ON_ASK, middle_center);
  }
  canvas.pushSprite(0, 0);
}

// --- Dock: a desk clock while charging -------------------------------------------------

constexpr uint16_t NIGHT    = rgb(0x7a, 0x3b, 0x2b);  // deep amber-red, for the dark
constexpr uint16_t NIGHT_LO = rgb(0x3a, 0x1c, 0x15);

static void bolt(int x, int y, uint16_t c) {  // 7 x 11, top-left
  canvas.fillTriangle(x + 5, y, x, y + 6, x + 4, y + 6, c);
  canvas.fillTriangle(x + 3, y + 5, x + 7, y + 5, x + 2, y + 11, c);
}

static String chargeText(const Dash& d) {
  if (d.battery < 0) return "charging";
  int eta = battery::etaMinutes();
  String t = String(d.battery) + "%";
  if (battery::full() || d.battery >= 100) return t + " - full";
  if (d.charging && eta > 0 && eta < 24 * 60)
    t += " - full in " + (eta >= 60 ? String(eta / 60) + "h " + String(eta % 60) + "m" : String(eta) + " min");
  return t;
}

void drawDock(const Dash& d, int side, bool night) {
  applyOrientation(side);
  if (night != dimmed_) {
    dimmed_ = night;
    M5.Display.setBrightness(night ? 13 : brightness);  // ~5% at night
  }
  const uint32_t t = millis();
  const uint16_t ink = night ? NIGHT : INK, dim = night ? NIGHT_LO : DIM;
  const float pulse = d.charging ? 0.5f + 0.5f * sinf(t / 420.0f) : 1;
  const int colon = d.clock.indexOf(':');

  if (side != 0) {
    // A2 - landscape: the clock across the full width, battery as a see-through banner
    const uint16_t base = night ? TFT_BLACK : rgb(0x0b, 0x0d, 0x0f);
    canvas.fillScreen(base);
    if (!night)
      for (int i = 0; i < 8; i++) canvas.fillCircle(W * 3 / 10, H / 5, 170 - i * 20, blend(base, rgb(0x14, 0x22, 0x2a), i / 7.0f));
    if (d.clock.length()) {
      txt(d.clock, W / 2, 54, fit(d.clock, W - 12, {F96, F44}), ink, middle_center);
      txt(d.ampm, W - 8, 10, F12, dim, top_right);
    } else {
      txt(d.name, W / 2, 52, fit(d.name, W - 12, {F24, F12}), ink, middle_center);
    }
    // date + weather on one line
    String date = d.longDate.length() ? d.longDate : d.date;
    String temp = d.wxValid ? String(d.wxTemp) + "°" : "";
    canvas.setFont(F12);
    int w1 = canvas.textWidth(date), w2 = temp.length() ? 8 + 15 + 3 + canvas.textWidth(temp) : 0;
    int x = (W - w1 - w2) / 2, y = 102;
    txt(date, x, y, F12, dim);
    if (temp.length()) {
      if (!night) weatherIcon(d.wxIcon, d.wxDay, x + w1 + 8, y - 8, 15);
      txt(temp, x + w1 + 8 + 15 + 3, y, F12, night ? NIGHT_LO : INK);
    }
    // the banner
    const int y0 = H - 22;
    const uint16_t tint = night ? NIGHT_LO : OK;
    for (int yy = y0; yy < H; yy++) canvas.drawFastHLine(0, yy, W, blend(base, tint, 0.03f + 0.14f * (yy - y0) / 22.0f));
    canvas.drawFastHLine(0, y0, W, blend(base, tint, 0.3f));
    // the text sits on the right; the bar fills all the room to its left
    String ct = chargeText(d);
    canvas.setFont(F9);
    int tw = canvas.textWidth(ct);
    const int bx = 10, byy = H - 11;
    const int bw = max(40, W - 10 - tw - (d.charging ? 14 : 0) - 10 - bx);
    canvas.fillRoundRect(bx, byy, bw, 4, 2, blend(base, ink, 0.12f));
    if (d.battery >= 0) canvas.fillRoundRect(bx, byy, max(4, bw * d.battery / 100), 4, 2, blend(base, tint, 0.55f + 0.45f * pulse));
    txt(ct, W - 10, H - 10, F9, night ? NIGHT : OK, middle_right);
    if (d.charging) bolt(W - 10 - tw - 11, H - 15, night ? NIGHT : OK);
  } else {
    // B - portrait: hours over minutes, huge and thin
    canvas.fillScreen(night ? TFT_BLACK : GROUND);
    if (!night)
      for (int yy = 0; yy < 150; yy += 3) canvas.fillRect(0, yy, W, 3, blend(rgb(0x12, 0x1a, 0x1f), GROUND, yy / 150.0f));
    if (d.clock.length() && colon > 0) {
      txt(d.clock.substring(0, colon), W / 2, 62, F96, ink, middle_center);
      txt(d.clock.substring(colon + 1), W / 2, 150, F96, ink, middle_center);
      txt(d.ampm.length() ? d.ampm + "  -  " + d.date : d.date, W / 2, 202, F12, dim, middle_center);
    } else {
      txt(d.name, W / 2, 100, fit(d.name, W - 12, {F24, F12}), ink, middle_center);
    }
    if (d.wxValid) {
      String temp = String(d.wxTemp) + "°";
      canvas.setFont(F12);
      int w = 15 + 4 + canvas.textWidth(temp), x = (W - w) / 2;
      if (!night) weatherIcon(d.wxIcon, d.wxDay, x, 211, 15);
      txt(temp, x + 19, 219, F12, night ? NIGHT_LO : INK);
    }
    const int lx = 30, lw = W - 60, ly = H - 8;
    canvas.fillRoundRect(lx, ly, lw, 3, 1, blend(GROUND, ink, 0.12f));
    if (d.battery >= 0) canvas.fillRoundRect(lx, ly, max(3, lw * d.battery / 100), 3, 1, night ? NIGHT : blend(GROUND, OK, 0.55f + 0.45f * pulse));
  }
  canvas.pushSprite(0, 0);
}

}  // namespace ui
