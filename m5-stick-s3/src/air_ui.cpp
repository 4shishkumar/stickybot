// The remote modes' screens (air_ui.h). Uses the drawing kit the faces share (ui_draw.h).
#include "air_ui.h"
#include "ui_draw.h"
#include "logic/remote.h"

namespace airui {

using namespace ui;

static uint16_t modeColor(Screen s) {
  switch (s) {
    case Screen::PRESENTER: return AMBER;
    case Screen::MEDIA:     return WISPR;
    default:                return AGENT;
  }
}

static void topBar(const char* title, const View& v) {
  canvas.fillCircle(PAD + 3, BAR_H / 2, 3, v.connected ? ui::OK : ASK);
  txt(title, PAD + 11, BAR_H / 2, F12, modeColor(v.screen));
  int r = W - PAD - 18;
  batteryIcon(r, BAR_H / 2 - 4, v.battery, v.charging);
  if (v.battery >= 0) txt(String(v.battery) + "%", r - 3, BAR_H / 2, F9, v.charging ? ui::OK : v.battery <= 15 ? ERR : DIM, middle_right);
}

// Up to three lines of hints at the bottom (portrait), or one or two (landscape)
static void hints(const char* const lines[], int n, int cx = -1) {
  if (cx < 0) cx = W / 2;
  for (int i = 0; i < n; i++) {
    int y = H - 8 - (n - 1 - i) * 13;
    txt(lines[i], cx, y, fit(lines[i], W - 8, {F9}), FAINT, middle_center);
  }
}

// A rounded label, its colours faded towards the background by t (0 full .. 1 gone)
static void chipAt(const String& s, int cx, int cy, uint16_t c, float t = 0) {
  if (t >= 1) return;
  const lgfx::IFont* f = F12;
  canvas.setFont(f);
  int w = canvas.textWidth(s) + 18, h = 20;
  canvas.fillRoundRect(cx - w / 2, cy - h / 2, w, h, h / 2, blend(blend(GROUND, c, 0.28f), GROUND, t));
  txt(s, cx, cy, f, blend(c, GROUND, t), middle_center);
}

// Not connected / the Mac kept its old report map: said over the middle of the screen
static bool problem(const View& v, int top, int bottom = 46) {
  if (v.connected && !v.mouseUnknown) return false;
  const int x = PAD - 2, w = W - 2 * x, h = H - top - bottom;
  canvas.fillRoundRect(x, top, w, h, 10, PANEL);
  canvas.fillRoundRect(x + 10, top + 10, 30, 4, 2, ASK);
  if (!v.connected) {
    txt("Not connected", x + 10, top + 28, F14, INK);
    paragraph(W > H ? "Waiting for the Mac." : "Waiting for the Mac. Pair the stick in its Bluetooth settings if it never was.", F12, x + 10, top + 46,
              w - 20, (h - 50) / 15, DIM, 15);
  } else {
    txt("Pair again once", x + 10, top + 28, F14, INK);
    paragraph(W > H ? "Mac: Bluetooth settings, forget the stick, then connect it again."
                    : "The Mac doesn't know the stick is a mouse yet. In Bluetooth settings: forget the stick, then connect it again.",
              F12, x + 10, top + 46, w - 20, (h - 50) / 15, DIM, 15);
  }
  return true;
}

// --- Icons --------------------------------------------------------------------------------

static void pointerIcon(int cx, int cy, uint16_t c) {
  canvas.fillTriangle(cx - 9, cy - 17, cx - 9, cy + 9, cx + 10, cy + 3, c);
  canvas.fillTriangle(cx - 2, cy + 2, cx + 3, cy, cx + 9, cy + 15, c);
  canvas.fillTriangle(cx - 2, cy + 2, cx + 9, cy + 15, cx + 4, cy + 17, c);
}

static void scrollIcon(int cx, int cy, uint16_t c) {
  canvas.fillRoundRect(cx - 9, cy - 15, 18, 30, 9, c);
  canvas.fillRoundRect(cx - 6, cy - 12, 12, 24, 6, GROUND);
  canvas.fillRoundRect(cx - 2, cy - 8, 4, 8, 2, c);
  canvas.fillTriangle(cx - 6, cy - 21, cx + 6, cy - 21, cx, cy - 27, c);
  canvas.fillTriangle(cx - 6, cy + 21, cx + 6, cy + 21, cx, cy + 27, c);
}

static void pauseIcon(int cx, int cy, uint16_t c) {
  canvas.fillRoundRect(cx - 11, cy - 14, 8, 28, 2, c);
  canvas.fillRoundRect(cx + 3, cy - 14, 8, 28, 2, c);
}

static void restIcon(int cx, int cy, uint16_t c) {
  txt("z", cx - 6, cy + 4, F24, c, middle_center);
  txt("z", cx + 8, cy - 10, F14, c, middle_center);
}

static void playIcon(int cx, int cy, uint16_t c) {
  canvas.fillTriangle(cx - 14, cy - 13, cx - 14, cy + 13, cx + 4, cy, c);
  canvas.fillRect(cx + 7, cy - 12, 4, 24, c);
  canvas.fillRect(cx + 14, cy - 12, 4, 24, c);
}

static void skipIcon(int cx, int cy, int dir, uint16_t c) {
  int s = dir;  // +1 next, -1 previous
  canvas.fillTriangle(cx - 15 * s, cy - 11, cx - 15 * s, cy + 11, cx - 2 * s, cy, c);
  canvas.fillTriangle(cx - 3 * s, cy - 11, cx - 3 * s, cy + 11, cx + 10 * s, cy, c);
  if (s > 0) canvas.fillRect(cx + 11, cy - 11, 4, 22, c);
  else canvas.fillRect(cx - 14, cy - 11, 4, 22, c);
}

static void speakerIcon(int cx, int cy, int kind, uint16_t c) {  // kind: +1 up, -1 down, 0 mute
  int x = cx - 8;
  canvas.fillRect(x - 10, cy - 5, 6, 10, c);
  canvas.fillTriangle(x - 4, cy - 5, x - 4, cy + 5, x + 5, cy - 12, c);
  canvas.fillTriangle(x - 4, cy + 5, x + 5, cy + 12, x + 5, cy - 12, c);
  int px = cx + 12;
  if (kind > 0) {
    canvas.fillRect(px - 6, cy - 1, 13, 3, c);
    canvas.fillRect(px - 1, cy - 6, 3, 13, c);
  } else if (kind < 0) {
    canvas.fillRect(px - 6, cy - 1, 13, 3, c);
  } else {
    for (int d = -1; d <= 1; d++) {
      canvas.drawLine(px - 6 + d, cy - 6, px + 6 + d, cy + 6, c);
      canvas.drawLine(px - 6 + d, cy + 6, px + 6 + d, cy - 6, c);
    }
  }
}

static void ring(int cx, int cy, int r, uint16_t c, bool lit) {
  if (lit) glow(cx, cy, r + 4, c, 0.35f);
  canvas.fillCircle(cx, cy, r, blend(GROUND, c, lit ? 0.22f : 0.1f));
  canvas.drawCircle(cx, cy, r, blend(GROUND, c, lit ? 0.8f : 0.35f));
}

// --- Air mouse ------------------------------------------------------------------------------

static const char* mouseWord(uint8_t s) {
  switch (s) {
    case PAUSED:    return "Paused";
    case RESTING:   return "Resting";
    case SCROLLING: return "Scrolling";
    case DRAGGING:  return "Dragging";
    default:        return "Pointing";
  }
}

static uint16_t mouseColor(uint8_t s) {
  switch (s) {
    case PAUSED:    return ASK;
    case RESTING:   return FAINT;
    case SCROLLING: return ui::BLUE;
    case DRAGGING:  return WISPR;
    default:        return AGENT;
  }
}

static void mouseIcon(int cx, int cy, uint8_t s) {
  uint16_t c = mouseColor(s);
  ring(cx, cy, 34, c, s != RESTING && s != PAUSED);
  if (s == SCROLLING) scrollIcon(cx, cy, c == FAINT ? DIM : INK);
  else if (s == PAUSED) pauseIcon(cx, cy, c);
  else if (s == RESTING) restIcon(cx, cy, DIM);
  else pointerIcon(cx + 1, cy, INK);
}

static void dots(int cx, int y, uint8_t n, uint16_t c) {
  const int gap = 9, x0 = cx - 9 * gap / 2;
  for (int i = 0; i < 10; i++) canvas.fillCircle(x0 + i * gap, y, i < n ? 3 : 2, i < n ? c : LINE);
}

static void drawMouse(const View& v, bool wide) {
  topBar("Air mouse", v);
  static const char* const H3[] = {"A click   B right-click", "hold B + tilt: scroll", "A+B pause   hold A+B: exit"};
  static const char* const H2[] = {"A click  B right  hold B + tilt: scroll", "A+B pause   hold A+B: Home"};
  if (wide) {
    if (!problem(v, BAR_H + 4, 24)) {
      mouseIcon(54, 66, v.state);
      txt(mouseWord(v.state), 104, 50, F18, INK);
      txt("Speed", 104, 76, F9, DIM);
      dots(104 + 45 + 26, 92, v.sens, mouseColor(v.state) == FAINT ? DIM : mouseColor(v.state));
    }
    hints(H2, 2);
  } else {
    if (!problem(v, BAR_H + 6)) {
      mouseIcon(W / 2, 84, v.state);
      txt(mouseWord(v.state), W / 2, 140, F18, INK, middle_center);
      txt("Speed", W / 2, 162, F9, DIM, middle_center);
      dots(W / 2, 176, v.sens, mouseColor(v.state) == FAINT ? DIM : mouseColor(v.state));
    }
    hints(H3, 3);
  }
  if (v.flash.length()) chipAt(v.flash, W / 2, BAR_H + 12, mouseColor(v.state), v.flashT);
}

// --- Presenter --------------------------------------------------------------------------------

static uint16_t phaseColor(uint8_t phase, bool running) {
  if (phase == (uint8_t)remote::Phase::IDLE) return DIM;
  if (phase == (uint8_t)remote::Phase::OVER) return ERR;
  if (phase == (uint8_t)remote::Phase::WARN) return ASK;
  return running ? INK : DIM;
}

static void drawPresenter(const View& v, bool wide) {
  topBar("Presenter", v);
  char clock[16];
  remote::formatClock((int32_t)v.elapsedMs, clock, sizeof clock);
  const uint16_t c = phaseColor(v.phase, v.timerRunning);
  const bool idle = v.phase == (uint8_t)remote::Phase::IDLE;
  // the line under the clock: the target, how far over, or how it starts
  String sub;
  if (idle) sub = "starts on the first click";
  else if (!v.targetMin) sub = "no target";
  else if (v.phase == (uint8_t)remote::Phase::OVER) {
    char over[16];
    remote::formatClock((int32_t)(v.elapsedMs - v.targetMin * 60000u), over, sizeof over);
    sub = String("+") + over + " over " + v.targetMin + " min";
  } else {
    char left[16];
    remote::formatClock((int32_t)(v.targetMin * 60000u - v.elapsedMs), left, sizeof left);
    sub = String(left) + " left of " + v.targetMin + " min";
  }
  float frac = v.targetMin ? (float)v.elapsedMs / (v.targetMin * 60000.0f) : 0;
  if (frac > 1) frac = 1;
  // what's on now: laser, black screen, paused
  String state;
  uint16_t stateC = DIM;
  if (v.laser) { state = "Laser"; stateC = ERR; }
  else if (v.black) { state = "Screen black"; stateC = ui::BLUE; }
  else if (!idle && !v.timerRunning) { state = "Timer paused"; stateC = ASK; }

  if (wide) {
    if (!problem(v, BAR_H + 4, 24)) {
      const lgfx::IFont* f = fit(clock, W - 20, {F56, F44, F32});
      txt(clock, W / 2, 54, f, c, middle_center);
      if (state.length()) txt(state, W / 2, 90, F12, stateC, middle_center);  // laser / black / paused first
      else txt(ellipsize(sub, F12, W - 2 * PAD), W / 2, 90, F12, c == INK ? DIM : c, middle_center);
      if (v.targetMin) {
        canvas.fillRoundRect(PAD, 101, W - 2 * PAD, 5, 2, LINE);
        if (frac > 0) canvas.fillRoundRect(PAD, 101, (int)((W - 2 * PAD) * frac), 5, 2, c == DIM || c == INK ? ui::OK : c);
      }
    }
    static const char* const H1[] = {"A next  B back  hold A: laser  2xB: black"};
    hints(H1, 1);
  } else {
    if (!problem(v, BAR_H + 6)) {
      const lgfx::IFont* f = fit(clock, W - 10, {F56, F44, F32});
      txt(clock, W / 2, 74, f, c, middle_center);
      { const lgfx::IFont* sf = fit(sub, W - 2 * PAD, {F12, F9}); txt(ellipsize(sub, sf, W - 2 * PAD), W / 2, 114, sf, c == INK ? DIM : c, middle_center); }
      if (v.targetMin) {
        canvas.fillRoundRect(PAD, 140, W - 2 * PAD, 6, 3, LINE);
        if (frac > 0) canvas.fillRoundRect(PAD, 140, (int)((W - 2 * PAD) * frac), 6, 3, c == DIM || c == INK ? ui::OK : c);
      }
      if (!idle && v.slide) txt(String(v.slide > 0 ? "+" : "") + v.slide + (abs(v.slide) == 1 ? " slide" : " slides"), W / 2, 162, F12, DIM, middle_center);
      if (state.length()) chipAt(state, W / 2, 186, stateC);
    }
    static const char* const H3[] = {"A next   B back", "hold A: laser  2xB: black", "hold A+B: exit"};
    hints(H3, 3);
  }
  if (v.flash.length()) chipAt(v.flash, W / 2, BAR_H + 12, AMBER, v.flashT);
}

// --- Media ----------------------------------------------------------------------------------

static const char* mediaWord(uint8_t a) {
  switch (a) {
    case PLAY_PAUSE: return "Play / pause";
    case NEXT:       return "Next track";
    case PREVIOUS:   return "Previous track";
    case VOL_UP:     return "Volume up";
    case VOL_DOWN:   return "Volume down";
    case MUTE:       return "Mute";
    default:         return "Ready";
  }
}

static void mediaIcon(int cx, int cy, const View& v) {
  uint16_t c = WISPR;
  bool lit = v.flashT < 1;
  ring(cx, cy, 34, c, lit);
  // the knob: how far the twist is towards the next volume step
  float k = v.knob < -1 ? -1 : v.knob > 1 ? 1 : v.knob;
  if (k > 0.05f) canvas.fillArc(cx, cy, 40, 37, 270, 270 + k * 85, ui::BLUE);
  else if (k < -0.05f) canvas.fillArc(cx, cy, 40, 37, 270 + k * 85, 270, ui::BLUE);
  uint16_t ic = lit ? INK : DIM;
  switch (v.lastAction) {
    case NEXT:     skipIcon(cx, cy, 1, ic); break;
    case PREVIOUS: skipIcon(cx, cy, -1, ic); break;
    case VOL_UP:   speakerIcon(cx, cy, 1, ic); break;
    case VOL_DOWN: speakerIcon(cx, cy, -1, ic); break;
    case MUTE:     speakerIcon(cx, cy, 0, ic); break;
    default:       playIcon(cx, cy, ic); break;
  }
}

static void drawMedia(const View& v, bool wide) {
  topBar("Media", v);
  if (wide) {
    if (!problem(v, BAR_H + 4, 24)) {
      mediaIcon(54, 68, v);
      txt(mediaWord(v.lastAction), 104, 58, fit(mediaWord(v.lastAction), W - 110, {F18, F14}), INK);
      txt("twist: volume", 104, 80, F9, DIM);
      txt("flick: next / previous", 104, 93, F9, DIM);
    }
    static const char* const H1[] = {"A play / pause   B mute   hold A+B: Home"};
    hints(H1, 1);
  } else {
    if (!problem(v, BAR_H + 6)) {
      mediaIcon(W / 2, 86, v);
      txt(mediaWord(v.lastAction), W / 2, 146, fit(mediaWord(v.lastAction), W - 10, {F18, F14}), INK, middle_center);
    }
    static const char* const H3[] = {"twist: volume  flick: track", "A play / pause   B mute", "hold A+B: exit"};
    hints(H3, 3);
  }
}

void draw(const View& v) {
  bool wide = ui::turnToHeldSide();
  canvas.fillScreen(GROUND);
  switch (v.screen) {
    case Screen::PRESENTER: drawPresenter(v, wide); break;
    case Screen::MEDIA:     drawMedia(v, wide); break;
    default:                drawMouse(v, wide); break;
  }
  canvas.pushSprite(0, 0);
}

}  // namespace airui
