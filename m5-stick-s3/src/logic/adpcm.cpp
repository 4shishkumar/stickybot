#include "adpcm.h"

namespace adpcm {

static const int16_t STEP[89] = {
  7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97,
  107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796,
  876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871,
  5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623,
  27086, 29794, 32767};
static const int8_t IDX_ADJ[16] = {-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8};

static inline int clampi(int v, int lo, int hi) { return v < lo ? lo : v > hi ? hi : v; }

uint8_t encodeSample(int16_t s, State& st) {
  int step = STEP[st.idx], diff = s - st.pred, delta = step >> 3;
  uint8_t code = 0;
  if (diff < 0) { code = 8; diff = -diff; }
  if (diff >= step) { code |= 4; diff -= step; delta += step; }
  step >>= 1;
  if (diff >= step) { code |= 2; diff -= step; delta += step; }
  step >>= 1;
  if (diff >= step) { code |= 1; delta += step; }
  st.pred = clampi(st.pred + ((code & 8) ? -delta : delta), -32768, 32767);
  st.idx = clampi(st.idx + IDX_ADJ[code], 0, 88);
  return code;
}

int16_t decodeSample(uint8_t code, State& st) {
  int step = STEP[st.idx], d = step >> 3;
  if (code & 4) d += step;
  if (code & 2) d += step >> 1;
  if (code & 1) d += step >> 2;
  st.pred = clampi((code & 8) ? st.pred - d : st.pred + d, -32768, 32767);
  st.idx = clampi(st.idx + IDX_ADJ[code & 15], 0, 88);
  return (int16_t)st.pred;
}

void writeHeader(uint8_t* out, uint16_t seq, const State& st) {
  out[0] = seq & 0xFF;
  out[1] = seq >> 8;
  out[2] = (uint16_t)st.pred & 0xFF;
  out[3] = (uint16_t)st.pred >> 8;
  out[4] = (uint8_t)st.idx;
}

size_t encodeFrame(uint16_t seq, State& st, const int16_t* pcm, size_t n, uint8_t* out) {
  writeHeader(out, seq, st);
  for (size_t i = 0; i < n; i++) {
    uint8_t code = encodeSample(pcm[i], st);
    uint8_t& b = out[HEADER + i / 2];
    if (i & 1) b |= code << 4;
    else b = code;
  }
  return HEADER + (n + 1) / 2;
}

size_t decodeFrame(const uint8_t* f, size_t len, int16_t* out, size_t cap, uint16_t* seq) {
  if (len < HEADER) return 0;
  if (seq) *seq = f[0] | (f[1] << 8);
  State st;
  st.pred = (int16_t)(f[2] | (f[3] << 8));
  st.idx = f[4] > 88 ? 88 : f[4];
  size_t n = 0;
  for (size_t b = HEADER; b < len; b++)
    for (int half = 0; half < 2; half++) {
      int16_t s = decodeSample(half ? f[b] >> 4 : f[b] & 15, st);
      if (n < cap) out[n++] = s;
    }
  return n;
}

}  // namespace adpcm
