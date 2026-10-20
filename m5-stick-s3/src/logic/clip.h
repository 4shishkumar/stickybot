// Sound clips from the Mac: packet parsing and putting frames in place.
//   packets: [0][samples u32][rate u16] start, [1][ADPCM frame] data, [2] end
//   queue  : the Bluetooth task stores packets as [len u8][packet]...
// Frames go to seq * frameSize, so a lost frame is 20 ms of silence instead of
// shifting the rest. Pure C++ (native-testable).
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace clip {

struct Sink {
  virtual void begin(uint32_t samples, uint16_t rate) = 0;
  virtual void frame(const uint8_t* f, size_t len) = 0;
  virtual void end() = 0;
  virtual ~Sink() = default;
};

// Walks a [len][packet]... queue; returns bytes consumed (a cut-off packet stays).
size_t parseQueue(const uint8_t* q, size_t len, Sink& sink);

// Decodes frames into a caller-owned buffer of `samples`.
struct Assembler {
  int16_t* buf = nullptr;
  uint32_t samples = 0, filled = 0, frames = 0, frameSize = 0;
  void reset(int16_t* b, uint32_t n);   // b zeroed by the caller
  void add(const uint8_t* f, size_t len);
  uint32_t expectedFrames() const { return frameSize ? (samples + frameSize - 1) / frameSize : 0; }
};

}  // namespace clip
