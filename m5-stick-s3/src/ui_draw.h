// The drawing kit shared by ui.cpp and the faces (src/faces/): the canvas, the fonts,
// the palette and the pieces (text, icons, the eyes). Not for the rest of the firmware:
// they draw through ui.h.
#pragma once
#include <M5Unified.h>
#include "ui.h"

namespace ui {

constexpr uint16_t rgb(uint8_t r, uint8_t g, uint8_t b) {
  return ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3);
}
constexpr uint16_t GROUND = rgb(0x11, 0x13, 0x16);
constexpr uint16_t PANEL  = rgb(0x18, 0x1b, 0x1f);
constexpr uint16_t CHIP   = rgb(0x22, 0x27, 0x2c);
constexpr uint16_t LINE   = rgb(0x2b, 0x30, 0x36);
constexpr uint16_t INK    = rgb(0xe6, 0xe9, 0xec);
constexpr uint16_t DIM    = rgb(0x98, 0xa1, 0xab);
constexpr uint16_t FAINT  = rgb(0x5b, 0x63, 0x6d);
constexpr uint16_t AGENT  = rgb(0x2e, 0xc4, 0xb6);  // teal: the Mac agent
constexpr uint16_t WISPR  = rgb(0x9b, 0x7b, 0xff);  // purple: dictation (button A)
constexpr uint16_t ASK    = rgb(0xff, 0x9f, 0x1c);  // orange: needs you
constexpr uint16_t OK     = rgb(0x3d, 0xdc, 0x84);
constexpr uint16_t ERR    = rgb(0xff, 0x52, 0x52);
constexpr uint16_t ON_ASK = rgb(0x1a, 0x12, 0x05);  // text on orange
constexpr uint16_t VIOLET = rgb(0xa6, 0x8c, 0xff);  // thinking
constexpr uint16_t AMBER  = rgb(0xff, 0xb3, 0x3c);  // working
constexpr uint16_t BRIGHT = rgb(0x6c, 0xf2, 0xe2);  // listening
constexpr uint16_t BLUE   = rgb(0x5a, 0xa9, 0xff);  // history
constexpr uint16_t FACT   = rgb(0xf5, 0xb7, 0x3b);  // fun facts
constexpr uint16_t PUPIL  = rgb(0x0a, 0x0c, 0x0e);
constexpr uint16_t SUN    = rgb(0xff, 0xc9, 0x4d);
constexpr uint16_t RAIN   = rgb(0x6f, 0xb7, 0xff);
constexpr uint16_t SNOW   = rgb(0xdf, 0xe8, 0xf1);

constexpr int BAR_H = 20;
constexpr int PAD   = 8;

extern M5Canvas canvas;  // everything draws here, then pushSprite
extern int      W, H;    // its size now (135 x 240 portrait, 240 x 135 landscape)
extern const lgfx::IFont *F9, *F12, *F14, *F18, *F24, *F44, *F96;  // F44 / F96: digits only

// Text
void txt(const String& s, int x, int y, const lgfx::IFont* f, uint16_t color, textdatum_t datum = middle_left);
const lgfx::IFont* fit(const String& s, int maxW, std::initializer_list<const lgfx::IFont*> fonts);
String ellipsize(const String& s, const lgfx::IFont* f, int maxW);
int    wrap(const String& s, const lgfx::IFont* f, String out[], int max, int maxW);
int    paragraph(const String& s, const lgfx::IFont* f, int x, int y, int maxW, int maxLines, uint16_t color, int lineH);
int    lineH(const lgfx::IFont* f);

// Colour
uint16_t blend(uint16_t a, uint16_t b, float t);  // t 0 -> a, 1 -> b
float    smooth(float t);                          // smoothstep 0..1
float    easeOutBack(float t);

// Pieces
void btIcon(int x, int y, uint16_t c);                         // 7 x 11, top-left
void batteryIcon(int x, int y, int pct, bool charging);        // 18 x 9, top-left
void weatherIcon(const String& code, bool day, int x, int y, int size);
void glow(int cx, int cy, int r, uint16_t c, float strength);
void confettiReset(float x, float y);
void confettiDraw(bool rain);

// The eyes, one look per mood (also tints the agent screens)
enum class Mood : uint8_t { IDLE, LISTEN, THINK, WORK, DONE, ERROR, ASK, OFFLINE, PARTY, COUNT };
uint16_t moodColor(Mood m);
// Two big eyes centred on cx, cy; s = scale (1 = 40 x 56 px eyes); bg = what's behind them.
void eyes(Mood m, int cx, int cy, float s, uint16_t c, uint16_t bg);
void lookAt(float x);  // idle pupils look this way (NAN: their own glances)

// Which way is down on the screen now (unit-ish vector, from the accelerometer): +x right,
// +y down. Portrait held upright: (0, 1).
void gravity(float& gx, float& gy);
bool landscapeNow();  // the screen is turned landscape right now

// Classic Home (the clock, weather and the agent's card): the "classic" face.
void drawClassicHome(const Dash& d);

}  // namespace ui
