// Learning a button (firmware 2.5.0, src/ir/ir.cpp): the captures the receiver hands over, made
// into one code. The button is pressed twice and the two must agree (the same protocol code, or
// the same raw timings within 25 %); a remote that sends its whole state (air conditioners) can
// be learned from one press ("once"). Pure C++.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include "ir_codec.h"

namespace irlearn {

enum class Outcome : uint8_t {
  IGNORED,   // noise, a lone repeat frame: still waiting
  FIRST,     // got the first press: press it again
  MISMATCH,  // the second press didn't match the first: press again (this one counts as the first)
  DONE,      // learned: code()
};

class Learner {
 public:
  void start(bool once) {
    once_ = once;
    have_ = false;
    done_ = false;
    mismatches_ = 0;
  }
  // One capture (frames joined by their gaps, a mark first). Changes t (cleaned in place).
  Outcome capture(uint16_t* t, size_t n);
  bool  done() const { return done_; }
  bool  waitingSecond() const { return have_ && !done_; }
  int   mismatches() const { return mismatches_; }
  // The learned code as a code string ("nec:4:8", "raw:38:..."); 0 if it doesn't fit
  size_t code(char* out, size_t max) const;
  bool  decoded() const { return decoded_; }
  const ircodec::Code& protoCode() const { return code_; }
  size_t rawCount() const { return n_; }

 private:
  void keep(const uint16_t* t, size_t n, bool dec, const ircodec::Code& c);
  bool once_ = false, have_ = false, done_ = false, decoded_ = false;
  int mismatches_ = 0;
  ircodec::Code code_;
  uint16_t raw_[ircodec::MAX_RAW];
  size_t n_ = 0;
};

}  // namespace irlearn
