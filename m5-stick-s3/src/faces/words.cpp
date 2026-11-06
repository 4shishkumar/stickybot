// Words: the time as you'd say it, "twenty-five to eleven", in big type, the hour's word in
// the agent's teal; "just after" / "nearly" small above it for the minutes in between
// (logic/clockface.cpp). The date small at the bottom, the battery as a hairline. Anything
// urgent comes up as a card at the bottom. Static: drawn when something changes.
#include "faces.h"
#include "common.h"
#include "../logic/clockface.h"

namespace faces {

using namespace ui;

namespace {

static int fontWidth(const std::string& s, void*) { return canvas.textWidth(s.c_str()); }

class Words : public Face {
 public:
  const char* id() const override { return "words"; }
  const char* title() const override { return "Words"; }
  const char* about() const override { return "The time in words, the way you'd say it: twenty-five to eleven"; }

  void draw(const Dash& d, bool wide, uint32_t) override {
    canvas.fillScreen(GROUND);
    for (int y = 0; y < H / 2; y += 4) canvas.fillRect(0, y, W, 4, blend(rgb(0x17, 0x1a, 0x2a), GROUND, 2.0f * y / H));
    const Urgent u = urgent(d);
    // the room for the words: above the card (or the date)
    const int top = wide ? 8 : 14, bottom = u.on ? (wide ? 84 : 184) : (wide ? 100 : 206);
    if (d.minute < 0) {
      txt(d.name, W / 2, (top + bottom) / 2 - 8, fit(d.name, W - 12, {F24, F18, F12}), INK, middle_center);
      txt("time comes from the Mac agent", W / 2, (top + bottom) / 2 + 16, F9, FAINT, middle_center);
    } else {
      words(clockface::words(d.minute / 60, d.minute % 60), top, bottom);
    }
    if (u.on) {
      if (wide) signBoard(10, 86, W - 20, 38, u.a, u.b, u.bg, u.ink);
      else signBoard(6, 188, W - 12, 42, u.a, u.b, u.bg, u.ink);
    } else {
      String date = d.agentLinked ? d.date : String("agent offline");
      txt(ellipsize(date, F12, W - 12), W / 2, wide ? 112 : 218, F12, FAINT, middle_center);
    }
    if (d.battery >= 0) {  // the battery, a hairline
      const int x = 30, w = W - 60;
      canvas.fillRect(x, H - 2, w, 1, blend(GROUND, INK, 0.1f));
      canvas.fillRect(x, H - 2, max(2, w * d.battery / 100), 1, d.charging ? ui::OK : d.battery <= 15 ? ERR : FAINT);
    }
  }
  // frameMs 0: drawn when something changes (the minute, a card)

 private:
  // The lead small, the words stacked big (the biggest font they fit in), centred in top..bottom.
  void words(const clockface::Words& w, int top, int bottom) {
    const int maxW = W - 10;
    const lgfx::IFont* f = F32;
    std::vector<std::string> lines;
    for (const lgfx::IFont* tryF : {F32, F24, F18}) {
      f = tryF;
      canvas.setFont(f);
      lines = clockface::stack(w.text, maxW, fontWidth, nullptr);
      if ((int)lines.size() * lineH(f) + (w.lead.length() ? 20 : 0) <= bottom - top) break;
    }
    const int lh = lineH(f), leadH = w.lead.length() ? 20 : 0;
    int y = (top + bottom) / 2 - ((int)lines.size() * lh + leadH) / 2;
    if (leadH) {
      txt(w.lead.c_str(), W / 2, y + 8, F14, DIM, middle_center);
      y += leadH;
    }
    for (const std::string& l : lines) {
      line(String(l.c_str()), w.hour.c_str(), W / 2, y + lh / 2, f);
      y += lh;
    }
  }

  // One line centred on cx, the hour's word in teal.
  void line(const String& l, const String& hour, int cx, int cy, const lgfx::IFont* f) {
    canvas.setFont(f);
    const int space = canvas.textWidth(" ");
    int x = cx - (int)canvas.textWidth(l) / 2, from = 0;
    while (from <= (int)l.length()) {
      int sp = l.indexOf(' ', from);
      if (sp < 0) sp = l.length();
      String word = l.substring(from, sp);
      txt(word, x, cy, f, word == hour ? AGENT : INK, middle_left);
      canvas.setFont(f);
      x += canvas.textWidth(word) + space;
      from = sp + 1;
    }
  }
};

}  // namespace

Face& wordsFace() {
  static Words f;
  return f;
}

}  // namespace faces
