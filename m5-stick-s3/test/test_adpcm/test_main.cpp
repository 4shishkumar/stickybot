// IMA-ADPCM codec: the frame format the stick and the Mac agent share.
#include <unity.h>
#include <math.h>
#include <vector>
#include "logic/adpcm.h"

void setUp() {}
void tearDown() {}

static std::vector<int16_t> sine(size_t n, float freq, float amp, int rate = 16000) {
  std::vector<int16_t> v(n);
  for (size_t i = 0; i < n; i++) v[i] = (int16_t)(amp * sinf(2 * M_PI * freq * i / rate));
  return v;
}

static double snrDb(const std::vector<int16_t>& a, const std::vector<int16_t>& b) {
  double sig = 0, err = 0;
  for (size_t i = 0; i < a.size(); i++) { sig += (double)a[i] * a[i]; err += (double)(a[i] - b[i]) * (a[i] - b[i]); }
  return 10 * log10(sig / (err + 1e-9));
}

void test_header_layout() {
  adpcm::State st;
  st.pred = -2;  // 0xFFFE
  st.idx = 42;
  uint8_t f[8] = {};
  adpcm::writeHeader(f, 0x1234, st);
  TEST_ASSERT_EQUAL_HEX8(0x34, f[0]);  // seq, little-endian
  TEST_ASSERT_EQUAL_HEX8(0x12, f[1]);
  TEST_ASSERT_EQUAL_HEX8(0xFE, f[2]);  // predictor
  TEST_ASSERT_EQUAL_HEX8(0xFF, f[3]);
  TEST_ASSERT_EQUAL_UINT8(42, f[4]);   // step index
}

void test_frame_length_and_samples() {
  int16_t pcm[320] = {};
  uint8_t f[200];
  adpcm::State st;
  TEST_ASSERT_EQUAL(165u, adpcm::encodeFrame(0, st, pcm, 320, f));  // 5 + 160
  TEST_ASSERT_EQUAL(320u, adpcm::frameSamples(165));
  adpcm::State st2;
  TEST_ASSERT_EQUAL(5u + 2u, adpcm::encodeFrame(0, st2, pcm, 3, f));  // odd count rounds up
  TEST_ASSERT_EQUAL(0u, adpcm::frameSamples(5));
}

void test_round_trip_quality() {
  auto in = sine(3200, 440, 12000);
  std::vector<int16_t> out(in.size());
  adpcm::State enc;
  uint8_t f[200];
  for (size_t at = 0; at < in.size(); at += 320) {
    size_t len = adpcm::encodeFrame(at / 320, enc, &in[at], 320, f);
    uint16_t seq = 0;
    TEST_ASSERT_EQUAL(320u, adpcm::decodeFrame(f, len, &out[at], 320, &seq));
    TEST_ASSERT_EQUAL(at / 320, seq);
  }
  TEST_ASSERT_GREATER_THAN(20.0, snrDb(in, out));  // speech-grade
}

void test_frames_decode_independently() {
  // Decoding frame 2 alone gives the same samples as decoding 1 then 2: each
  // frame carries the codec state, so a lost frame doesn't corrupt the next.
  auto in = sine(640, 300, 9000);
  adpcm::State enc;
  uint8_t f1[200], f2[200];
  size_t l1 = adpcm::encodeFrame(0, enc, &in[0], 320, f1);
  size_t l2 = adpcm::encodeFrame(1, enc, &in[320], 320, f2);
  std::vector<int16_t> seq(640), alone(320);
  adpcm::decodeFrame(f1, l1, &seq[0], 320);
  adpcm::decodeFrame(f2, l2, &seq[320], 320);
  adpcm::decodeFrame(f2, l2, &alone[0], 320);
  TEST_ASSERT_EQUAL_INT16_ARRAY(&seq[320], &alone[0], 320);
}

void test_extremes_clamp() {
  std::vector<int16_t> in(400);
  for (size_t i = 0; i < in.size(); i++) in[i] = (i / 20) % 2 ? 32767 : -32768;  // full-scale square
  adpcm::State enc, dec;
  for (int16_t s : in) {
    uint8_t c = adpcm::encodeSample(s, enc);
    TEST_ASSERT_TRUE(c < 16);
    adpcm::decodeSample(c, dec);
    TEST_ASSERT_TRUE(enc.idx >= 0 && enc.idx <= 88);
  }
  TEST_ASSERT_EQUAL(enc.pred, dec.pred);  // encoder tracks the decoder exactly
  TEST_ASSERT_EQUAL(enc.idx, dec.idx);
}

void test_decode_caps_output_and_bad_index() {
  uint8_t f[10] = {0, 0, 0, 0, 200, 0x77, 0x77, 0x77, 0x77, 0x77};  // index 200 -> clamped
  int16_t out[4];
  TEST_ASSERT_EQUAL(4u, adpcm::decodeFrame(f, sizeof f, out, 4));
  TEST_ASSERT_EQUAL(0u, adpcm::decodeFrame(f, 3, out, 4));  // shorter than a header
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_header_layout);
  RUN_TEST(test_frame_length_and_samples);
  RUN_TEST(test_round_trip_quality);
  RUN_TEST(test_frames_decode_independently);
  RUN_TEST(test_extremes_clamp);
  RUN_TEST(test_decode_caps_output_and_bad_index);
  return UNITY_END();
}
