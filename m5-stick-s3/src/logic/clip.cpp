#include "clip.h"
#include "adpcm.h"

namespace clip {

size_t parseQueue(const uint8_t* q, size_t len, Sink& sink) {
  size_t i = 0;
  while (i < len) {
    size_t n = q[i];
    if (i + 1 + n > len) break;  // incomplete: leave it for later
    const uint8_t* p = q + i + 1;
    i += 1 + n;
    if (!n) continue;
    if (p[0] == 0 && n >= 7)
      sink.begin(p[1] | (p[2] << 8) | (p[3] << 16) | ((uint32_t)p[4] << 24), p[5] | (p[6] << 8));
    else if (p[0] == 1)
      sink.frame(p + 1, n - 1);
    else if (p[0] == 2)
      sink.end();
  }
  return i;
}

void Assembler::reset(int16_t* b, uint32_t n) {
  buf = b;
  samples = b ? n : 0;
  filled = frames = frameSize = 0;
}

void Assembler::add(const uint8_t* f, size_t len) {
  if (!buf || len < adpcm::HEADER + 1) return;
  uint32_t n = adpcm::frameSamples(len);
  if (!frameSize) frameSize = n;  // the first frame is a full one
  uint32_t seq = f[0] | (f[1] << 8);
  uint32_t at = seq * frameSize;
  frames++;
  if (at >= samples) return;
  size_t got = adpcm::decodeFrame(f, len, buf + at, samples - at);
  if (at + got > filled) filled = at + got;
}

}  // namespace clip
