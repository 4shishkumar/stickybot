// IMA-ADPCM, 4 bits a sample: the stick's audio frames (mic -> Mac agent, sound
// clips Mac -> stick). Frame layout (PROTOCOL.md, "Mac link"), little-endian:
//   [seq u16][predictor i16][step index u8][samples, 4 bits each, low nibble first]
// Each frame carries the codec state at its start, so it decodes on its own.
// Pure C++ (no Arduino): also built by the native unit tests.
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace adpcm {

struct State {
  int pred = 0;  // last sample
  int idx = 0;   // step index 0..88
};

constexpr size_t HEADER = 5;

uint8_t encodeSample(int16_t s, State& st);   // returns the 4-bit code, advances st
int16_t decodeSample(uint8_t code, State& st);

void writeHeader(uint8_t* out, uint16_t seq, const State& st);
// Encodes n samples (header + n/2 bytes, rounded up); returns the frame length.
size_t encodeFrame(uint16_t seq, State& st, const int16_t* pcm, size_t n, uint8_t* out);
// Samples a frame of `len` bytes holds (2 per data byte).
inline size_t frameSamples(size_t len) { return len > HEADER ? (len - HEADER) * 2 : 0; }
// Decodes up to `cap` samples into out; returns how many were written. seq optional.
size_t decodeFrame(const uint8_t* f, size_t len, int16_t* out, size_t cap, uint16_t* seq = nullptr);

}  // namespace adpcm
