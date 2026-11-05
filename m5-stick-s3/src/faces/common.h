// Pieces the faces share: what must stay visible, the time, signs and speech bubbles.
// (Internal to src/faces/.)
#pragma once
#include "faces.h"
#include "../ui_draw.h"

namespace faces {

// Something the user must see, whatever the face: the Mac not connected / Bluetooth off,
// the battery low, an approval or a question waiting (the agent's urgent card).
struct Urgent {
  bool     on = false;
  String   a, b;           // two lines, like Classic's card
  uint16_t bg = 0, ink = 0;
};
Urgent urgent(const ui::Dash& d);
String quietNotice(const ui::Dash& d);   // "agent offline" (not urgent: A still dictates), else ""

// The time (and AM / PM) with a dark shadow so it reads on any sky; returns its width.
int  timeText(const ui::Dash& d, int x, int y, const lgfx::IFont* f, uint16_t ink, uint16_t shade);
// Battery (% + icon) and Bluetooth, right-aligned at x, centred on y; agent dot too.
void statusIcons(const ui::Dash& d, int right, int cy, uint16_t ink);
void shadowTxt(const String& s, int x, int y, const lgfx::IFont* f, uint16_t ink, uint16_t shade,
               textdatum_t datum = middle_left);

// A board with up to two lines (the biggest fonts that fit): the bot's sign, the floating banner.
void signBoard(int x, int y, int w, int h, const String& a, const String& b, uint16_t bg, uint16_t ink);
// A speech bubble with a small tail pointing down at tailX.
void speech(int x, int y, int w, int h, int tailX, const String& a, const String& b, uint16_t bg, uint16_t ink);
// A small rounded label ("+10 XP") centred on cx, cy.
void pill(const String& s, int cx, int cy, uint16_t bg, uint16_t ink);

uint32_t hash32(uint32_t x);                          // cheap well-mixed hash (stars, rain, sand)
float    frac01(uint32_t x);                          // hash32(x) as 0..1
uint16_t c565(uint8_t r, uint8_t g, uint8_t b);
bool     isNightMinute(int minute);                    // 22:00-6:00 (unknown time: false)

// Demo (demo.cpp) -> registry (faces.cpp): which face and which fake world to show.
void setDemo(int face, const world::Snapshot* w);
void demoKick(world::Kick k);

}  // namespace faces
