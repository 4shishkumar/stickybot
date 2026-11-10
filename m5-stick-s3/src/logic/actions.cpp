#include "actions.h"

namespace actions {

// The order is the hint bar's: the most useful first (the rest drop off a narrow screen).
// "×" is in the stick's fonts (tools/make_fonts.py).

// Home: the hints show for a moment after the screen wakes.
static const Binding HOME[] = {
    {In::B_HOLD, Do::TALK, "hold B", "talk"},
    {In::A_PRESS, Do::COMBO, "A", "dictate"},
    {In::B_TRIPLE, Do::LAUNCHER, "3×B", "apps"},
    {In::B_CLICK, Do::ENTER, "B", "enter"},
    {In::B_DOUBLE, Do::UNLOCK, nullptr, nullptr},
    {In::SHAKE, Do::ESCAPE, nullptr, nullptr},
    {In::FLICK, Do::PAGES, nullptr, nullptr},
};

// B held: talking to the agent
static const Binding TALK[] = {
    {In::RELEASE, Do::SEND, "let go", "send"},
};

// The agent's screens before the answer (thinking / working) and after an error
static const Binding AGENT[] = {
    {In::B_CLICK, Do::CANCEL, "B", "cancel", BUSY},
    {In::SHAKE, Do::CANCEL, nullptr, nullptr, BUSY},
    {In::B_CLICK, Do::CLOSE, "B", "close", 0, BUSY},
    {In::SHAKE, Do::CLOSE, nullptr, nullptr, 0, BUSY},
    {In::B_HOLD, Do::TALK, "hold B", "again", 0, BUSY},
    {In::FLICK, Do::NONE, nullptr, nullptr},
};

// The reply view (the answer, "done"): A is the reader's while it shows (not the key combo)
static const Binding REPLY[] = {
    {In::A_CLICK, Do::NEXT_PAGE, "A", "next", PAGED},
    {In::A_HOLD, Do::LISTEN, "hold A", "listen", SPEECH, SPEAKING},
    {In::A_HOLD, Do::STOP_LISTEN, "hold A", "stop", SPEAKING},
    {In::B_CLICK, Do::CLOSE, "B", "close"},
    {In::TILT, Do::SCROLL, "tilt", "scroll", PAGED},
    {In::B_HOLD, Do::TALK, "hold B", "ask", 0, PAGED},
    {In::SHAKE, Do::CLOSE, nullptr, nullptr},
    {In::A_PRESS, Do::NONE, nullptr, nullptr},  // no key combo here
    {In::FLICK, Do::NONE, nullptr, nullptr},
};

#define SCREEN(name, t) {name, t, (uint8_t)(sizeof(t) / sizeof(t[0]))}
static const Screen SCREENS[] = {
    SCREEN("home", HOME),
    SCREEN("talk", TALK),
    SCREEN("agent", AGENT),
    SCREEN("reply", REPLY),
};
static_assert(sizeof(SCREENS) / sizeof(SCREENS[0]) == (size_t)Id::COUNT, "a Screen per Id");

const Screen& screen(Id id) { return SCREENS[(int)id < (int)Id::COUNT ? (int)id : 0]; }

static bool applies(const Binding& b, uint8_t flags) {
  return (flags & b.need) == b.need && !(flags & b.unless);
}

Do lookup(const Screen& s, In in, uint8_t flags) {
  for (uint8_t i = 0; i < s.count; i++)
    if (s.bindings[i].in == in && applies(s.bindings[i], flags)) return s.bindings[i].act;
  return Do::NONE;
}

std::vector<Hint> hints(const Screen& s, uint8_t flags) {
  std::vector<Hint> out;
  for (uint8_t i = 0; i < s.count; i++) {
    const Binding& b = s.bindings[i];
    if (!b.key || !applies(b, flags)) continue;
    bool accent = b.act == Do::LISTEN || b.act == Do::STOP_LISTEN;
    out.push_back({b.key, b.label ? b.label : "", accent});
  }
  return out;
}

std::vector<std::vector<int>> layout(const std::vector<Hint>& h, int maxW, int maxRows, int gap, text::Width w,
                                     void* ctx) {
  std::vector<std::vector<int>> rows;
  std::vector<int> widths;
  int used = 0;
  for (int i = 0; i < (int)h.size(); i++) {
    int iw = w(h[i].key + " " + h[i].label, ctx);
    if (iw > maxW) continue;
    if (!rows.empty() && used + gap + iw <= maxW) {
      rows.back().push_back(i);
      used += gap + iw;
    } else if ((int)rows.size() < maxRows) {
      rows.push_back({i});
      used = iw;
    }
    // else: no room left; a shorter one later may still fit a row's end
  }
  return rows;
}

const char* inName(In in) {
  static const char* const N[] = {"A press", "A click", "hold A", "B click", "double B", "triple B",
                                  "hold B",  "release", "tilt",   "shake",   "flick"};
  return (int)in < (int)In::COUNT ? N[(int)in] : "?";
}

const char* doName(Do d) {
  static const char* const N[] = {"-",     "key combo", "Enter",  "unlock",      "talk",
                                  "apps",  "Escape",    "pages",  "next page",   "scroll",
                                  "listen", "stop",     "close",  "cancel",      "send"};
  return (int)d < (int)Do::COUNT ? N[(int)d] : "?";
}

}  // namespace actions
