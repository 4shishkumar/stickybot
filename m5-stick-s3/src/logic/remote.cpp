#include "remote.h"
#include <stdio.h>

namespace remote {

const char* name(Btn b) {
  switch (b) {
    case Btn::A_PRESS:    return "A press";
    case Btn::A_RELEASE:  return "A release";
    case Btn::A_CLICK:    return "A click";
    case Btn::A_HOLD:     return "A hold";
    case Btn::B_PRESS:    return "B press";
    case Btn::B_RELEASE:  return "B release";
    case Btn::B_CLICK:    return "B click";
    case Btn::B_DOUBLE:   return "B double";
    case Btn::B_HOLD:     return "B hold";
    case Btn::CHORD:      return "A+B";
    case Btn::CHORD_HOLD: return "A+B hold";
    default:              return "none";
  }
}

void Buttons::reset(bool aDown, bool bDown) {
  a_ = One();
  b_ = One();
  if (aDown) a_.state = State::IGNORED;
  if (bDown) b_.state = State::IGNORED;
  lastA = aDown;
  lastB = bDown;
  bothAt = 0;
  chord = chordHeld = false;
  clickWaiting = false;
  Btn x;
  while (q.pop(x)) {}
}

Btn Buttons::next() {
  Btn b;
  return q.pop(b) ? b : Btn::NONE;
}

void Buttons::press(One& o, bool isA, uint32_t now) {
  o.state = State::PENDING;
  o.at = now;
  o.held = o.wasDouble = false;
  (void)isA;
}

void Buttons::release(One& o, bool isA, uint32_t now) {
  emit(isA ? Btn::A_RELEASE : Btn::B_RELEASE);
  if (!o.held && !o.wasDouble) {
    if (isA) emit(Btn::A_CLICK);
    else if (timing.doubleMsB) {
      clickWaiting = true;
      clickAt = now;
    } else emit(Btn::B_CLICK);
  }
  o.state = State::UP;
}

void Buttons::feed(uint32_t now, bool a, bool b) {
  bool aDown = a && !lastA, bDown = b && !lastB;
  lastA = a;
  lastB = b;
  if (a_.state == State::IGNORED) { if (!a) a_.state = State::UP; aDown = false; }
  if (b_.state == State::IGNORED) { if (!b) b_.state = State::UP; bDown = false; }

  // A chord: both down together, everything else from them is swallowed until both are up
  if (chord) {
    if (a && b && !chordHeld && now - chordAt >= CHORD_HOLD_MS) {
      chordHeld = true;
      emit(Btn::CHORD_HOLD);
    }
    if (!a && !b) {
      if (!chordHeld) emit(Btn::CHORD);
      chord = chordHeld = false;
      a_.state = b_.state = State::UP;
    }
    return;
  }
  if ((aDown && b_.state == State::PENDING) || (bDown && a_.state == State::PENDING) || (aDown && bDown)) {
    chord = true;
    chordHeld = false;
    chordAt = now;
    clickWaiting = false;  // a waiting B click is dropped: the chord takes over
    a_.state = b_.state = State::IGNORED;
    return;
  }

  // Both down but not pressed together (B already scrolling, A already dragging): held that way
  // CHORD_HOLD_MS still leaves; what the first one was doing ends first
  if (a && b) {
    if (!bothAt) bothAt = now ? now : 1;
    if (now - bothAt >= CHORD_HOLD_MS) {
      if (a_.state == State::DOWN) emit(Btn::A_RELEASE);
      if (b_.state == State::DOWN) emit(Btn::B_RELEASE);
      chord = chordHeld = true;
      clickWaiting = false;
      a_.state = b_.state = State::IGNORED;
      bothAt = 0;
      emit(Btn::CHORD_HOLD);
      return;
    }
  } else {
    bothAt = 0;
  }

  if (aDown) press(a_, true, now);
  if (bDown) {
    if (clickWaiting && timing.doubleMsB && now - clickAt < timing.doubleMsB) {
      clickWaiting = false;
      b_.state = State::DOWN;
      b_.at = now;
      b_.held = false;
      b_.wasDouble = true;
      emit(Btn::B_PRESS);
      emit(Btn::B_DOUBLE);
    } else {
      press(b_, false, now);
    }
  }

  for (int i = 0; i < 2; i++) {
    One& o = i == 0 ? a_ : b_;
    bool isA = i == 0, down = isA ? a : b;
    uint16_t holdMs = isA ? timing.holdMsA : timing.holdMsB;
    if (o.state == State::PENDING) {
      if (!down) {  // a quick tap, inside the chord window
        emit(isA ? Btn::A_PRESS : Btn::B_PRESS);
        release(o, isA, now);
        continue;
      }
      if (now - o.at < CHORD_MS) continue;
      emit(isA ? Btn::A_PRESS : Btn::B_PRESS);
      o.state = State::DOWN;
    }
    if (o.state == State::DOWN) {
      if (!down) {
        release(o, isA, now);
      } else if (!o.held && !o.wasDouble && now - o.at >= holdMs) {
        o.held = true;
        emit(isA ? Btn::A_HOLD : Btn::B_HOLD);
      }
    }
  }

  if (clickWaiting && now - clickAt >= timing.doubleMsB) {
    clickWaiting = false;
    emit(Btn::B_CLICK);
  }
}

// --- Talk timer --------------------------------------------------------------------------

void TalkTimer::start(uint32_t now) {
  if (running_) return;
  running_ = started_ = true;
  startedAt = now;
}

void TalkTimer::pause(uint32_t now) {
  if (!running_) return;
  banked += now - startedAt;
  running_ = false;
}

Phase TalkTimer::phase(uint32_t now, uint16_t targetMin) const {
  if (!started_) return Phase::IDLE;
  if (!targetMin) return Phase::RUNNING;
  uint32_t t = (uint32_t)targetMin * 60000u, el = elapsedMs(now);
  uint32_t warn = t / 10;
  if (warn < 60000u) warn = 60000u;
  if (warn > 300000u) warn = 300000u;
  if (el >= t) return Phase::OVER;
  if (el + warn >= t) return Phase::WARN;
  return Phase::RUNNING;
}

void formatClock(int32_t ms, char* out, int size) {
  bool neg = ms < 0;
  uint32_t s = (uint32_t)(neg ? -(int64_t)ms : ms) / 1000u;
  uint32_t h = s / 3600, m = (s / 60) % 60, sec = s % 60;
  if (h) snprintf(out, size, "%s%lu:%02lu:%02lu", neg ? "-" : "", (unsigned long)h, (unsigned long)m, (unsigned long)sec);
  else snprintf(out, size, "%s%lu:%02lu", neg ? "-" : "", (unsigned long)m, (unsigned long)sec);
}

}  // namespace remote
