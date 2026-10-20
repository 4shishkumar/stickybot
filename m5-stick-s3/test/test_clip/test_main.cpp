// Sound clips from the Mac: packet queue parsing and frame placement.
#include <unity.h>
#include <string>
#include <vector>
#include "logic/adpcm.h"
#include "logic/clip.h"

void setUp() {}
void tearDown() {}

struct Recorder : clip::Sink {
  std::string log;
  uint32_t samples = 0;
  uint16_t rate = 0;
  void begin(uint32_t s, uint16_t r) override { log += "B"; samples = s; rate = r; }
  void frame(const uint8_t*, size_t len) override { log += "F" + std::to_string(len); }
  void end() override { log += "E"; }
};

static void put(std::vector<uint8_t>& q, std::vector<uint8_t> pkt) {
  q.push_back((uint8_t)pkt.size());
  q.insert(q.end(), pkt.begin(), pkt.end());
}

void test_parse_queue_in_order() {
  std::vector<uint8_t> q;
  put(q, {0, 0x80, 0x3E, 0x00, 0x00, 0x80, 0x3E});  // start: 16000 samples, 16000 Hz
  put(q, {1, 9, 9, 9, 9, 9, 9});                    // data: a 6-byte frame
  put(q, {2});                                      // end
  Recorder r;
  TEST_ASSERT_EQUAL(q.size(), clip::parseQueue(q.data(), q.size(), r));
  TEST_ASSERT_EQUAL_STRING("BF6E", r.log.c_str());
  TEST_ASSERT_EQUAL_UINT32(16000, r.samples);
  TEST_ASSERT_EQUAL_UINT16(16000, r.rate);
}

void test_parse_stops_at_a_cut_packet() {
  std::vector<uint8_t> q;
  put(q, {2});
  q.push_back(10);  // says 10 bytes follow, only 2 do
  q.push_back(1);
  q.push_back(0);
  Recorder r;
  TEST_ASSERT_EQUAL(2u, clip::parseQueue(q.data(), q.size(), r));
  TEST_ASSERT_EQUAL_STRING("E", r.log.c_str());
}

void test_parse_ignores_short_start_and_empty() {
  std::vector<uint8_t> q;
  put(q, {0, 1, 2});  // start packet too short
  q.push_back(0);     // empty packet
  Recorder r;
  clip::parseQueue(q.data(), q.size(), r);
  TEST_ASSERT_EQUAL_STRING("", r.log.c_str());
}

static std::vector<uint8_t> frameOf(uint16_t seq, int16_t value, size_t n = 320) {
  std::vector<int16_t> pcm(n, value);
  std::vector<uint8_t> f(adpcm::HEADER + (n + 1) / 2);
  adpcm::State st;
  st.pred = value;  // start at the value: decodes ~flat
  st.idx = 0;
  adpcm::encodeFrame(seq, st, pcm.data(), n, f.data());
  return f;
}

void test_frames_land_by_sequence_and_gaps_stay_silent() {
  std::vector<int16_t> buf(960, 0);  // three 320-sample frames
  clip::Assembler a;
  a.reset(buf.data(), buf.size());
  auto f0 = frameOf(0, 1000), f2 = frameOf(2, 2000);  // frame 1 "lost"
  a.add(f0.data(), f0.size());
  a.add(f2.data(), f2.size());
  TEST_ASSERT_EQUAL_UINT32(320, a.frameSize);
  TEST_ASSERT_EQUAL_UINT32(2, a.frames);
  TEST_ASSERT_EQUAL_UINT32(960, a.filled);
  TEST_ASSERT_INT16_WITHIN(60, 1000, buf[100]);
  TEST_ASSERT_EQUAL_INT16(0, buf[500]);              // the lost frame: silence
  TEST_ASSERT_INT16_WITHIN(60, 2000, buf[800]);
  TEST_ASSERT_EQUAL_UINT32(3, a.expectedFrames());
}

void test_frames_past_the_end_are_dropped() {
  std::vector<int16_t> buf(400, 0);
  clip::Assembler a;
  a.reset(buf.data(), buf.size());
  auto f0 = frameOf(0, 500), f1 = frameOf(1, 500), f9 = frameOf(9, 500);
  a.add(f0.data(), f0.size());
  a.add(f1.data(), f1.size());  // partly fits
  a.add(f9.data(), f9.size());  // beyond: ignored
  TEST_ASSERT_EQUAL_UINT32(400, a.filled);
  TEST_ASSERT_EQUAL_UINT32(3, a.frames);
}

void test_no_buffer_is_safe() {
  clip::Assembler a;
  a.reset(nullptr, 1000);
  auto f0 = frameOf(0, 1);
  a.add(f0.data(), f0.size());
  TEST_ASSERT_EQUAL_UINT32(0, a.filled);
  TEST_ASSERT_EQUAL_UINT32(0, a.samples);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_parse_queue_in_order);
  RUN_TEST(test_parse_stops_at_a_cut_packet);
  RUN_TEST(test_parse_ignores_short_start_and_empty);
  RUN_TEST(test_frames_land_by_sequence_and_gaps_stay_silent);
  RUN_TEST(test_frames_past_the_end_are_dropped);
  RUN_TEST(test_no_buffer_is_safe);
  return UNITY_END();
}
