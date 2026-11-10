// The reply view (2.6.0): the agent's whole answer on the stick, to scroll through.
// Pure C++ (native-testable): ui.cpp draws it, main.cpp drives it.
//
//   Assembly   the full reply arrives after "done" in numbered parts
//              ({"src":"agent","reply":{"rid","i","t"}}, PROTOCOL.md); kept up to MAX bytes
//   wrapAll    every line of a text (paragraphs at '\n', words too long for a line broken),
//              UTF-8 safe
//   Pager      which lines are shown: A = next page (back to the top after the last),
//              tilt = a line at a time, and the scroll thumb
//   TiltScroll the stick tipped forward / back past a dead zone scrolls down / up, faster the
//              further it's tipped; small drifts in how it's held are followed slowly
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <string>
#include <vector>
#include "text.h"

namespace reader {

constexpr size_t MAX_BYTES = 3072;  // the longest reply kept (the Mac sends at most 3000)

// Cuts s to at most maxBytes without splitting a UTF-8 character.
void capUtf8(std::string& s, size_t maxBytes);

// All the lines of s at maxW: '\n' starts a new paragraph (one empty line kept between
// paragraphs that had a blank line), a word wider than a line is broken between characters.
std::vector<std::string> wrapAll(const std::string& s, int maxW, text::Width w, void* ctx);

class Assembly {
 public:
  enum class Step : uint8_t { IGNORED, ADDED, COMPLETE, BROKEN };
  // A new "done": `parts` pieces follow for reply `rid` (0 = none; the last one is dropped).
  void begin(uint32_t rid, int parts);
  // Piece i of reply rid. Out of order or for another reply: the assembly is given up (BROKEN)
  // or ignored; COMPLETE once the last one is in.
  Step add(uint32_t rid, int i, const char* t);
  bool complete() const { return parts_ > 0 && next_ == parts_; }
  bool waiting() const { return parts_ > 0 && next_ < parts_; }
  uint32_t rid() const { return rid_; }
  const std::string& text() const { return text_; }

 private:
  uint32_t    rid_ = 0;
  int         parts_ = 0, next_ = 0;
  std::string text_;
};

struct Pager {
  int total = 0;    // lines in the text
  int visible = 1;  // lines on the screen
  int top = 0;      // first one shown

  int  maxTop() const { return total > visible ? total - visible : 0; }
  bool paged() const { return total > visible; }
  void clamp();
  // A: the next page (one line of the last page stays for context); after the last page, the top.
  void next();
  // Tilt: n lines (+ down); true if it moved.
  bool scroll(int n);
  int  pages() const;  // pages of `visible` lines
  int  page() const;   // 1-based, the one the bottom line shown is on
  // The scroll thumb on a track trackH px tall: its top and height (min 10 px).
  void thumb(int trackH, int& y, int& h) const;
};

// The screen's tilt in degrees from the gravity along the screen's "down" (gy, g) and out
// of the screen (az, g): 90 = held upright, 0 = lying face up.
float pitchDeg(float gy, float az);

class TiltScroll {
 public:
  static constexpr float DEAD_DEG = 12.0f;    // no scrolling within this of where it was held
  static constexpr float LINES_PER_DEG = 0.3f;  // lines / s for each degree past the dead zone
  static constexpr float MAX_RATE = 10.0f;    // lines / s

  void begin() { primed_ = false; acc_ = 0; }
  // dt in seconds, the pitch now (pitchDeg); whole lines to scroll (+ = down: the top tipped
  // away from you). The rest carries over.
  int feed(float dt, float deg);
  float offset() const { return primed_ ? base_ - last_ : 0; }  // + = tipped away

 private:
  bool  primed_ = false;
  float base_ = 0, last_ = 0, acc_ = 0;
};

}  // namespace reader
