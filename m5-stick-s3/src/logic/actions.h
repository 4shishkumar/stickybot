// Context actions (2.6.0): each screen declares what its buttons and motions do, and the
// hint bar at the bottom of the screen is made from the same table, so the two can't drift
// apart. Pure C++ (native-testable). docs/CONTEXT-ACTIONS.md has every screen's map.
//
// A binding can depend on the screen's state (flags): the reply view's "A: next" only with
// more than one page, "hold A: listen" only when the Mac agent can speak, "stop" while it does.
//
// The global gestures stay the same everywhere they aren't listed: hold B = talk to the agent,
// double B = unlock the Mac, triple B = the Launcher, shake = Escape.
#pragma once
#include <stdint.h>
#include <string>
#include <vector>
#include "text.h"

namespace actions {

enum class In : uint8_t {
  A_PRESS,   // A down (the key combo: pressed while A is held)
  A_CLICK,   // A released before the hold time
  A_HOLD,    // A held past the hold time (once)
  B_CLICK,   // B released, one click (decided)
  B_DOUBLE,
  B_TRIPLE,
  B_HOLD,
  RELEASE,   // the held button let go (talk: send)
  TILT,      // tipped forward / back
  SHAKE,
  FLICK,     // a quick twist left / right
  COUNT
};

enum class Do : uint8_t {
  NONE,
  COMBO,        // the key combo (dictation)
  ENTER,
  UNLOCK,       // type the Mac password
  TALK,         // talk to the Mac agent
  LAUNCHER,
  ESCAPE,
  PAGES,        // Home / Agent / Status
  NEXT_PAGE,    // the reply's next page
  SCROLL,       // the reply, a line at a time
  LISTEN,       // the Mac reads the reply aloud
  STOP_LISTEN,
  CLOSE,        // back to Home
  CANCEL,       // drop the request and close
  SEND,         // the talk goes to the agent
  COUNT
};

// Screen state the bindings can depend on
enum Flag : uint8_t {
  PAGED    = 1 << 0,  // the text is longer than the screen
  SPEECH   = 1 << 1,  // the Mac agent can read it aloud
  SPEAKING = 1 << 2,  // ... and does right now
  BUSY     = 1 << 3,  // the agent is still on the request
};

struct Binding {
  In          in;
  Do          act;
  const char* key;    // the hint bar's key ("A", "hold A"); nullptr = not in the hint bar
  const char* label;  // ... and what it does ("next")
  uint8_t     need = 0;    // flags that must all be set
  uint8_t     unless = 0;  // flags that must all be clear
};

struct Screen {
  const char*    name;
  const Binding* bindings;
  uint8_t        count;
};

enum class Id : uint8_t { HOME, TALK, AGENT, REPLY, COUNT };

const Screen& screen(Id id);
// The action for this input on this screen in this state (NONE = the screen doesn't use it:
// the caller does what it always did).
Do lookup(const Screen& s, In in, uint8_t flags);

struct Hint {
  std::string key, label;
  bool        accent;  // the reply's listen / stop: drawn as a highlighted chip
};
std::vector<Hint> hints(const Screen& s, uint8_t flags);

// Hints laid out in rows no wider than maxW (each hint "key label", `gap` px between them):
// greedy, in order; hints that don't fit in maxRows are left out (the table lists the most
// useful first). Returns the hints' indices per row.
std::vector<std::vector<int>> layout(const std::vector<Hint>& h, int maxW, int maxRows, int gap, text::Width w,
                                     void* ctx);

const char* inName(In in);  // "A click", for logs and the docs
const char* doName(Do d);   // "next page"

}  // namespace actions
