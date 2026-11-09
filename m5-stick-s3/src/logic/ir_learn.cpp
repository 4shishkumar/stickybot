#include "ir_learn.h"
#include <string.h>

namespace irlearn {

using namespace ircodec;

void Learner::keep(const uint16_t* t, size_t n, bool dec, const Code& c) {
  memcpy(raw_, t, n * sizeof(uint16_t));
  n_ = n;
  decoded_ = dec;
  code_ = c;
  have_ = true;
}

Outcome Learner::capture(uint16_t* t, size_t n) {
  if (done_ || !t) return Outcome::IGNORED;
  if (n > MAX_RAW) n = MAX_RAW;
  n = cleanRaw(t, n);
  Code c;
  const bool dec = decodeCapture(t, n, c);
  if (!dec && n < 7) return Outcome::IGNORED;  // a blip, or only repeat frames
  if (dec) n = firstFrame(t, n);                // a protocol code: its first frame is the code
  if (!have_ || once_) {
    keep(t, n, dec, c);
    if (once_) {
      done_ = true;
      return Outcome::DONE;
    }
    return Outcome::FIRST;
  }
  bool same = false, useNew = false;
  if (dec || decoded_) {
    same = dec && decoded_ && c == code_;
  } else if (sameRaw(raw_, n_, t, n)) {
    same = true;
  } else {
    // a held button sends more frames: the same first frame is the same button (keep the shorter)
    const size_t fa = firstFrame(raw_, n_), fb = firstFrame(t, n);
    same = sameRaw(raw_, fa, t, fb);
    useNew = same && n < n_;
  }
  if (!same) {
    mismatches_++;
    keep(t, n, dec, c);  // this one is the first press now
    return Outcome::MISMATCH;
  }
  if (useNew) keep(t, n, dec, c);
  done_ = true;
  return Outcome::DONE;
}

size_t Learner::code(char* out, size_t max) const {
  if (!have_) return 0;
  if (decoded_) return format(code_, out, max);
  return formatRaw(raw_, n_, DEFAULT_KHZ, out, max);
}

}  // namespace irlearn
