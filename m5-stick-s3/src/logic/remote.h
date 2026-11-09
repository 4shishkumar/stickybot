// The remote modes' buttons and the presenter's talk timer. Pure C++ (native-testable).
//
// Buttons: fed the raw state of A and B every loop, it turns them into what the air mouse,
// presenter and media remote act on:
//   A_PRESS / A_RELEASE  the raw edges (a press is held back CHORD_MS, see below)
//   A_CLICK              released before holdMs
//   A_HOLD               still down after holdMs (once per press)
//   (the same for B, plus B_DOUBLE when B's doubleMs is set: then a single B_CLICK waits
//    doubleMs for a second press, and the second press reports B_DOUBLE instead)
//   CHORD                A and B pressed together (within CHORD_MS), both let go before
//                        CHORD_HOLD_MS: pause / resume
//   CHORD_HOLD           A and B held together for CHORD_HOLD_MS: leave the mode (also when one
//                        was pressed first and is already holding: its RELEASE comes first)
// A chord swallows everything else from both buttons until both are up.
#pragma once
#include <stdint.h>
#include "fixed_queue.h"

namespace remote {

enum class Btn : uint8_t {
  NONE,
  A_PRESS, A_RELEASE, A_CLICK, A_HOLD,
  B_PRESS, B_RELEASE, B_CLICK, B_DOUBLE, B_HOLD,
  CHORD, CHORD_HOLD,
};
const char* name(Btn b);

struct ButtonTiming {
  uint16_t holdMsA = 300;
  uint16_t holdMsB = 300;
  uint16_t doubleMsB = 0;  // 0 = no double press on B (a click is reported on release)
};

class Buttons {
 public:
  static constexpr uint32_t CHORD_MS = 70;
  static constexpr uint32_t CHORD_HOLD_MS = 1200;

  void configure(const ButtonTiming& t) { timing = t; }
  // Starting with a button already down (the press that opened the mode): it's ignored
  // until it's let go.
  void reset(bool aDown, bool bDown);
  void feed(uint32_t now, bool a, bool b);
  Btn  next();  // NONE when there's nothing more
  bool aHeld() const { return a_.state == State::DOWN && a_.held; }
  bool bHeld() const { return b_.state == State::DOWN && b_.held; }
  bool chording() const { return chord; }

 private:
  enum class State : uint8_t { UP, PENDING, DOWN, IGNORED };
  struct One {
    State    state = State::UP;
    uint32_t at = 0;        // when it went down
    bool     held = false;  // HOLD reported for this press
    bool     wasDouble = false;
  };
  void press(One& o, bool isA, uint32_t now);
  void release(One& o, bool isA, uint32_t now);
  void emit(Btn b) { q.push(b); }

  ButtonTiming timing;
  One   a_, b_;
  bool  lastA = false, lastB = false;
  bool  chord = false, chordHeld = false;
  uint32_t chordAt = 0;
  uint32_t bothAt = 0;  // both down, not pressed together (e.g. B already scrolling): since when
  bool  clickWaiting = false;  // B: a click waiting to see if a second press follows
  uint32_t clickAt = 0;
  FixedQueue<Btn, 12> q;
};

// --- Talk timer --------------------------------------------------------------------------

enum class Phase : uint8_t { IDLE, RUNNING, WARN, OVER };

class TalkTimer {
 public:
  void     reset() { running_ = false; startedAt = 0; banked = 0; started_ = false; }
  void     start(uint32_t now);         // no-op when it runs already
  void     pause(uint32_t now);
  void     toggle(uint32_t now) { running_ ? pause(now) : start(now); }
  bool     running() const { return running_; }
  bool     started() const { return started_; }
  uint32_t elapsedMs(uint32_t now) const { return banked + (running_ ? now - startedAt : 0); }
  // targetMin 0 = no target: RUNNING (or IDLE before the start). With a target: WARN in
  // the last WARN_PCT of it (at least 1, at most 5 minutes), OVER past it.
  Phase    phase(uint32_t now, uint16_t targetMin) const;

 private:
  bool     running_ = false, started_ = false;
  uint32_t startedAt = 0, banked = 0;
};

// "4:07", "12:30", "1:02:03"; negative ms (time left past the target) gets a "-"
void formatClock(int32_t ms, char* out, int size);

}  // namespace remote
