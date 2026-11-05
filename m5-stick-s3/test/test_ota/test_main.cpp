// Firmware update over Bluetooth: the receiver's state machine (offsets, acks,
// gaps, resume, sha / size checks, cancel, refusals) and the packet queue.
#include <unity.h>
#include <string.h>
#include <string>
#include <vector>
#include "logic/ota_rx.h"
#include "logic/packet_ring.h"

using otarx::Error;
using otarx::Receiver;
using otarx::Reply;
using Kind = otarx::Reply::Kind;

// A flash in memory. Its "sha256" is a stand-in digest (the state machine only
// compares 32 bytes); the tests compute the same one for the expected value.
static void digest(const std::vector<uint8_t>& d, size_t n, uint8_t out[32]) {
  uint32_t h = 2166136261u;
  for (int i = 0; i < 32; i++) {
    for (size_t k = i; k < n; k += 32) h = (h ^ d[k]) * 16777619u;
    h = (h ^ (uint32_t)n ^ (uint32_t)i) * 16777619u;
    out[i] = (uint8_t)(h >> 8);
  }
}

struct FakeSink : otarx::Sink {
  std::vector<uint8_t> flash;
  int opens = 0, aborts = 0, commits = 0;
  bool failOpen = false, failWrite = false, failCommit = false;
  bool open(uint32_t) override { opens++; flash.clear(); return !failOpen; }
  bool write(const uint8_t* p, size_t n) override {
    if (failWrite) return false;
    flash.insert(flash.end(), p, p + n);
    return true;
  }
  bool sha256(uint32_t size, uint8_t out[32]) override {
    if (flash.size() < size) return false;
    digest(flash, size, out);
    return true;
  }
  bool commit() override { commits++; return !failCommit; }
  void abort() override { aborts++; }
};

static std::vector<uint8_t> image(size_t n) {
  std::vector<uint8_t> d(n);
  for (size_t i = 0; i < n; i++) d[i] = (uint8_t)(i * 7 + (i >> 8));
  if (n >= 14) { d[0] = 0xE9; d[12] = 0x09; d[13] = 0x00; }  // ESP32-S3 app image header
  return d;
}

static std::string shaHex(const std::vector<uint8_t>& d) {
  uint8_t out[32];
  digest(d, d.size(), out);
  static const char* X = "0123456789abcdef";
  std::string s;
  for (uint8_t b : out) { s += X[b >> 4]; s += X[b & 15]; }
  return s;
}

static otarx::Conditions ok() {
  otarx::Conditions c;
  c.batteryPct = 80;
  c.partitionSize = 0x330000;
  c.running = "1.8.0";
  return c;
}

struct Session {
  FakeSink sink;
  Receiver rx{sink};
  std::vector<uint8_t> img;
  std::string sha;
  otarx::Begin b;
  uint32_t t = 1000;
  explicit Session(size_t n = 10000) : img(image(n)), sha(shaHex(img)) {
    b.size = (uint32_t)n;
    b.sha256 = sha.c_str();
    b.version = "1.8.1";
  }
  Reply begin() { return rx.begin(b, ok(), t); }
  Reply send(uint32_t off, size_t n) {
    if (off + n > img.size()) n = img.size() - off;
    return rx.data(off, img.data() + off, n, t);
  }
  // sends [from, to) in 240-byte packets, collecting the acks
  std::vector<uint32_t> stream(uint32_t from, uint32_t to) {
    std::vector<uint32_t> acks;
    for (uint32_t o = from; o < to; o += 240) {
      Reply r = send(o, (to - o) < 240 ? to - o : 240);
      t += 2;
      if (r.kind == Kind::ACK) acks.push_back(r.offset);
    }
    return acks;
  }
};

void setUp() {}
void tearDown() {}

void test_versions() {
  TEST_ASSERT_EQUAL(0, otarx::compareVersions("1.8.0", "1.8.0"));
  TEST_ASSERT_EQUAL(1, otarx::compareVersions("1.8.1-test", "1.8.0"));
  TEST_ASSERT_EQUAL(-1, otarx::compareVersions("1.7.2", "1.8.0"));
  TEST_ASSERT_EQUAL(1, otarx::compareVersions("1.10.0", "1.9.9"));
  TEST_ASSERT_EQUAL(0, otarx::compareVersions("1.8.0-test", "1.8.0"));
  TEST_ASSERT_EQUAL(1, otarx::compareVersions("2", "1.9"));
}

void test_sha_parsing() {
  uint8_t out[32];
  std::string good(64, 'a');
  good[0] = 'F';
  TEST_ASSERT_TRUE(otarx::parseSha256(good.c_str(), out));
  TEST_ASSERT_EQUAL_HEX8(0xFA, out[0]);
  TEST_ASSERT_FALSE(otarx::parseSha256("abc", out));
  std::string bad(64, 'g');
  TEST_ASSERT_FALSE(otarx::parseSha256(bad.c_str(), out));
  TEST_ASSERT_FALSE(otarx::parseSha256(nullptr, out));
}

void test_refusals() {
  Session s;
  auto c = ok();
  TEST_ASSERT_EQUAL(Error::NONE, otarx::refuse(s.b, c));
  c.busy = "dictation";
  TEST_ASSERT_EQUAL(Error::BUSY, otarx::refuse(s.b, c));
  c = ok();
  c.batteryPct = 29;
  TEST_ASSERT_EQUAL(Error::BATTERY, otarx::refuse(s.b, c));
  c.usbPower = true;  // low but plugged in: fine
  TEST_ASSERT_EQUAL(Error::NONE, otarx::refuse(s.b, c));
  c = ok();
  c.batteryPct = -1;  // unknown counts as low
  TEST_ASSERT_EQUAL(Error::BATTERY, otarx::refuse(s.b, c));
  c = ok();
  c.batteryPct = 30;
  TEST_ASSERT_EQUAL(Error::NONE, otarx::refuse(s.b, c));
  c = ok();
  c.pendingVerify = true;
  TEST_ASSERT_EQUAL(Error::PENDING, otarx::refuse(s.b, c));
  c = ok();
  s.b.compressed = true;
  TEST_ASSERT_EQUAL(Error::UNSUPPORTED, otarx::refuse(s.b, c));
  s.b.compressed = false;
  s.b.size = 0x330001;
  TEST_ASSERT_EQUAL(Error::SIZE, otarx::refuse(s.b, c));
  s.b.size = 0;
  TEST_ASSERT_EQUAL(Error::SIZE, otarx::refuse(s.b, c));
  s.b.size = 100;
  s.b.sha256 = "nothex";
  TEST_ASSERT_EQUAL(Error::SHA, otarx::refuse(s.b, c));
}

void test_downgrade_and_same_need_force() {
  Session s;
  auto c = ok();
  s.b.version = "1.7.2";
  TEST_ASSERT_EQUAL(Error::DOWNGRADE, otarx::refuse(s.b, c));
  s.b.version = "1.8.0";
  TEST_ASSERT_EQUAL(Error::SAME, otarx::refuse(s.b, c));
  s.b.force = true;
  TEST_ASSERT_EQUAL(Error::NONE, otarx::refuse(s.b, c));
  s.b.version = "1.7.2";
  TEST_ASSERT_EQUAL(Error::NONE, otarx::refuse(s.b, c));
}

void test_refused_begin_doesnt_open() {
  Session s;
  auto c = ok();
  c.busy = "talk";
  Reply r = s.rx.begin(s.b, c, s.t);
  TEST_ASSERT_EQUAL(Kind::REFUSED, r.kind);
  TEST_ASSERT_EQUAL(Error::BUSY, r.error);
  TEST_ASSERT_EQUAL(0, s.sink.opens);
  TEST_ASSERT_FALSE(s.rx.receiving());
  TEST_ASSERT_EQUAL_STRING("busy", otarx::errorName(r.error));
}

void test_full_transfer_with_acks() {
  Session s(20000);
  Reply r = s.begin();
  TEST_ASSERT_EQUAL(Kind::READY, r.kind);
  TEST_ASSERT_EQUAL(0u, r.offset);
  TEST_ASSERT_FALSE(r.resumed);
  auto acks = s.stream(0, 20000);
  // an ack every ACK_EVERY bytes (on packet boundaries) and one at the very end
  TEST_ASSERT_TRUE(acks.size() >= 4);
  for (size_t i = 1; i < acks.size(); i++) TEST_ASSERT_TRUE(acks[i] - acks[i - 1] >= otarx::ACK_EVERY || acks[i] == 20000);
  TEST_ASSERT_EQUAL(20000u, acks.back());
  TEST_ASSERT_EQUAL(100, s.rx.percent());
  r = s.rx.end(s.t);
  TEST_ASSERT_EQUAL(Kind::DONE, r.kind);
  TEST_ASSERT_EQUAL(1, s.sink.commits);
  TEST_ASSERT_TRUE(s.sink.flash == s.img);
  // a repeated end (its reply was lost) gets the same answer, no second commit
  TEST_ASSERT_EQUAL(Kind::DONE, s.rx.end(s.t).kind);
  TEST_ASSERT_EQUAL(1, s.sink.commits);
}

void test_timed_ack_when_the_stream_pauses() {
  Session s;
  s.begin();
  TEST_ASSERT_EQUAL(Kind::NONE, s.send(0, 240).kind);  // below ACK_EVERY
  TEST_ASSERT_EQUAL(Kind::NONE, s.rx.poll(s.t + 10).kind);
  Reply r = s.rx.poll(s.t + otarx::ACK_IDLE_MS);
  TEST_ASSERT_EQUAL(Kind::ACK, r.kind);
  TEST_ASSERT_EQUAL(240u, r.offset);
  TEST_ASSERT_EQUAL(Kind::NONE, s.rx.poll(s.t + 2 * otarx::ACK_IDLE_MS).kind);  // nothing new
}

void test_dropped_packet_asks_for_a_resend() {
  Session s;
  s.begin();
  s.send(0, 240);
  Reply r = s.send(480, 240);  // 240..480 got lost
  TEST_ASSERT_EQUAL(Kind::GAP, r.kind);
  TEST_ASSERT_EQUAL(240u, r.offset);
  TEST_ASSERT_EQUAL(Kind::NONE, s.send(720, 240).kind);  // more after the gap: no GAP flood
  TEST_ASSERT_EQUAL(Kind::GAP, s.rx.poll(s.t + otarx::GAP_RETRY_MS).kind);  // ...but repeated in time
  s.t += otarx::GAP_RETRY_MS + 1;
  TEST_ASSERT_EQUAL(Kind::NONE, s.send(240, 240).kind);  // the resend fills it
  TEST_ASSERT_EQUAL(480u, s.rx.next());
  s.stream(480, 10000);
  TEST_ASSERT_EQUAL(Kind::DONE, s.rx.end(s.t).kind);
  TEST_ASSERT_TRUE(s.sink.flash == s.img);
}

void test_duplicates_and_overlaps() {
  Session s;
  s.begin();
  s.send(0, 1000);
  s.t += 1;
  TEST_ASSERT_EQUAL(Kind::NONE, s.send(0, 1000).kind);  // all old
  TEST_ASSERT_EQUAL(1000u, s.rx.next());
  s.send(500, 1000);  // half old, half new: takes the new half
  TEST_ASSERT_EQUAL(1500u, s.rx.next());
  s.stream(1500, 10000);
  TEST_ASSERT_EQUAL(Kind::DONE, s.rx.end(s.t).kind);
  TEST_ASSERT_TRUE(s.sink.flash == s.img);
}

void test_duplicates_get_a_fresh_ack() {
  // the agent timed out (its ack was lost) and rewound: re-ack even with nothing new
  Session s;
  s.begin();
  s.send(0, 240);
  TEST_ASSERT_EQUAL(Kind::ACK, s.rx.poll(s.t + otarx::ACK_IDLE_MS).kind);
  s.t += 2 * otarx::ACK_IDLE_MS;
  s.send(0, 240);
  Reply r = s.rx.poll(s.t + otarx::ACK_IDLE_MS);
  TEST_ASSERT_EQUAL(Kind::ACK, r.kind);
  TEST_ASSERT_EQUAL(240u, r.offset);
}

void test_resume_after_reconnect() {
  Session s;
  s.begin();
  s.stream(0, 4800);
  uint32_t at = s.rx.next();
  s.t += 30000;  // link down for a while
  Reply r = s.begin();
  TEST_ASSERT_EQUAL(Kind::READY, r.kind);
  TEST_ASSERT_TRUE(r.resumed);
  TEST_ASSERT_EQUAL(at, r.offset);
  TEST_ASSERT_EQUAL(1, s.sink.opens);  // not reopened (nothing erased)
  s.stream(at, 10000);
  TEST_ASSERT_EQUAL(Kind::DONE, s.rx.end(s.t).kind);
  TEST_ASSERT_TRUE(s.sink.flash == s.img);
}

void test_resume_ignores_the_version_check() {
  // resuming the same image is fine even if it would now count as "same" (force not resent)
  Session s;
  s.begin();
  s.stream(0, 2400);
  auto c = ok();
  c.running = "1.8.1";
  Reply r = s.rx.begin(s.b, c, s.t);
  TEST_ASSERT_EQUAL(Kind::READY, r.kind);
  TEST_ASSERT_TRUE(r.resumed);
}

void test_a_different_image_starts_over() {
  Session s;
  s.begin();
  s.stream(0, 2400);
  Session other(8000);
  Reply r = s.rx.begin(other.b, ok(), s.t);
  TEST_ASSERT_EQUAL(Kind::READY, r.kind);
  TEST_ASSERT_FALSE(r.resumed);
  TEST_ASSERT_EQUAL(0u, r.offset);
  TEST_ASSERT_EQUAL(1, s.sink.aborts);
  TEST_ASSERT_EQUAL(2, s.sink.opens);
  TEST_ASSERT_EQUAL(8000u, s.rx.size());
}

void test_refusal_keeps_an_open_session() {
  Session s;
  s.begin();
  s.stream(0, 2400);
  auto c = ok();
  c.busy = "approval";
  TEST_ASSERT_EQUAL(Kind::REFUSED, s.rx.begin(s.b, c, s.t).kind);
  TEST_ASSERT_TRUE(s.rx.receiving());
  TEST_ASSERT_EQUAL(0, s.sink.aborts);
  TEST_ASSERT_TRUE(s.begin().resumed);
}

void test_sha_mismatch_aborts() {
  Session s;
  std::string wrong = s.sha;
  wrong[0] = wrong[0] == 'a' ? 'b' : 'a';
  s.b.sha256 = wrong.c_str();
  s.begin();
  s.stream(0, 10000);
  Reply r = s.rx.end(s.t);
  TEST_ASSERT_EQUAL(Kind::ERROR, r.kind);
  TEST_ASSERT_EQUAL(Error::SHA, r.error);
  TEST_ASSERT_EQUAL(0, s.sink.commits);
  TEST_ASSERT_EQUAL(1, s.sink.aborts);
  TEST_ASSERT_FALSE(s.rx.receiving());
}

void test_corrupted_byte_fails_the_sha() {
  Session s;
  s.begin();
  s.stream(0, 5000);
  s.img[6000] ^= 0x40;  // what arrives differs from what the manifest hashed
  s.stream(5000, 10000);
  TEST_ASSERT_EQUAL(Error::SHA, s.rx.end(s.t).error);
}

void test_end_before_all_bytes_keeps_the_session() {
  Session s;
  s.begin();
  s.stream(0, 6000);
  Reply r = s.rx.end(s.t);
  TEST_ASSERT_EQUAL(Kind::ERROR, r.kind);
  TEST_ASSERT_EQUAL(Error::INCOMPLETE, r.error);
  TEST_ASSERT_EQUAL(6000u, r.offset);
  TEST_ASSERT_TRUE(s.rx.receiving());
  s.stream(6000, 10000);
  TEST_ASSERT_EQUAL(Kind::DONE, s.rx.end(s.t).kind);
}

void test_data_past_the_end_fails() {
  Session s(1000);
  s.begin();
  s.send(0, 900);
  std::vector<uint8_t> more(200, 1);
  Reply r = s.rx.data(900, more.data(), more.size(), s.t);
  TEST_ASSERT_EQUAL(Kind::ERROR, r.kind);
  TEST_ASSERT_EQUAL(Error::SIZE, r.error);
  TEST_ASSERT_EQUAL(1, s.sink.aborts);
}

void test_not_an_app_image() {
  Session s;
  s.img[0] = 0x00;  // e.g. a merged image / random file
  s.sha = shaHex(s.img);
  s.b.sha256 = s.sha.c_str();
  s.begin();
  Reply r = s.send(0, 240);
  TEST_ASSERT_EQUAL(Kind::ERROR, r.kind);
  TEST_ASSERT_EQUAL(Error::IMAGE, r.error);
  Session c3;
  c3.img[12] = 0x05;  // ESP32-C3's chip id
  c3.begin();
  TEST_ASSERT_EQUAL(Error::IMAGE, c3.send(0, 240).error);
}

void test_header_split_over_tiny_packets() {
  Session s;
  s.begin();
  for (uint32_t o = 0; o < 20; o += 5) TEST_ASSERT_NOT_EQUAL(Kind::ERROR, s.send(o, 5).kind);
  s.stream(20, 10000);
  TEST_ASSERT_EQUAL(Kind::DONE, s.rx.end(s.t).kind);
}

void test_cancel() {
  Session s;
  s.begin();
  s.stream(0, 2400);
  Reply r = s.rx.cancel();
  TEST_ASSERT_EQUAL(Kind::CANCELLED, r.kind);
  TEST_ASSERT_EQUAL(1, s.sink.aborts);
  TEST_ASSERT_FALSE(s.rx.receiving());
  TEST_ASSERT_EQUAL(Kind::NONE, s.send(2400, 240).kind);  // late packets are ignored
  TEST_ASSERT_EQUAL(Kind::NONE, s.rx.cancel().kind);
  TEST_ASSERT_EQUAL(Error::NOT_STARTED, s.rx.end(s.t).error);
  Reply again = s.begin();  // a new begin starts from 0
  TEST_ASSERT_EQUAL(0u, again.offset);
  TEST_ASSERT_FALSE(again.resumed);
}

void test_idle_timeout() {
  Session s;
  s.begin();
  s.stream(0, 2400);
  s.send(2400, 240);  // the last data at s.t
  TEST_ASSERT_NOT_EQUAL(Kind::ERROR, s.rx.poll(s.t + otarx::IDLE_TIMEOUT_MS - 1).kind);
  Reply r = s.rx.poll(s.t + otarx::IDLE_TIMEOUT_MS);
  TEST_ASSERT_EQUAL(Kind::ERROR, r.kind);
  TEST_ASSERT_EQUAL(Error::TIMEOUT, r.error);
  TEST_ASSERT_EQUAL(1, s.sink.aborts);
}

void test_flash_failures() {
  Session s;
  s.sink.failOpen = true;
  Reply r = s.begin();
  TEST_ASSERT_EQUAL(Error::OPEN, r.error);
  TEST_ASSERT_FALSE(s.rx.receiving());
  Session w;
  w.begin();
  w.sink.failWrite = true;
  TEST_ASSERT_EQUAL(Error::WRITE, w.send(0, 240).error);
  Session m;
  m.sink.failCommit = true;
  m.begin();
  m.stream(0, 10000);
  r = m.rx.end(m.t);
  TEST_ASSERT_EQUAL(Error::IMAGE, r.error);
  TEST_ASSERT_FALSE(m.rx.receiving());
}

void test_packet_header() {
  Session s;
  s.begin();
  uint8_t pkt[4 + 240];
  pkt[0] = 0; pkt[1] = 0; pkt[2] = 0; pkt[3] = 0;
  memcpy(pkt + 4, s.img.data(), 240);
  s.rx.packet(pkt, sizeof(pkt), s.t);
  TEST_ASSERT_EQUAL(240u, s.rx.next());
  uint32_t off = 240;
  pkt[0] = off & 0xFF; pkt[1] = off >> 8; pkt[2] = 0; pkt[3] = 0;
  memcpy(pkt + 4, s.img.data() + 240, 240);
  s.rx.packet(pkt, sizeof(pkt), s.t);
  TEST_ASSERT_EQUAL(480u, s.rx.next());
  TEST_ASSERT_EQUAL(Kind::NONE, s.rx.packet(pkt, 4, s.t).kind);  // header only: ignored
}

void test_packet_ring() {
  PacketRing<64> q;
  uint8_t a[10], b[20], out[32];
  uint16_t n;
  memset(a, 1, sizeof(a));
  memset(b, 2, sizeof(b));
  TEST_ASSERT_TRUE(q.empty());
  TEST_ASSERT_FALSE(q.pop(out, sizeof(out), n));
  TEST_ASSERT_TRUE(q.push(a, 10));
  TEST_ASSERT_TRUE(q.push(b, 20));
  TEST_ASSERT_EQUAL(34, (int)q.used());
  TEST_ASSERT_FALSE(q.push(b, 29));  // 34 + 31 > 64: refused whole
  TEST_ASSERT_TRUE(q.push(b, 28));   // exactly full
  TEST_ASSERT_EQUAL(0, (int)q.free());
  TEST_ASSERT_TRUE(q.pop(out, sizeof(out), n));
  TEST_ASSERT_EQUAL(10, n);
  TEST_ASSERT_EQUAL(1, out[9]);
  // wraps around the end of the buffer
  TEST_ASSERT_TRUE(q.push(a, 8));
  TEST_ASSERT_TRUE(q.pop(out, sizeof(out), n));
  TEST_ASSERT_EQUAL(20, n);
  TEST_ASSERT_TRUE(q.pop(out, sizeof(out), n));
  TEST_ASSERT_EQUAL(28, n);
  TEST_ASSERT_TRUE(q.pop(out, sizeof(out), n));
  TEST_ASSERT_EQUAL(8, n);
  TEST_ASSERT_EQUAL(1, out[7]);
  TEST_ASSERT_TRUE(q.empty());
  // a packet bigger than the reader's buffer is dropped, the next one still comes out
  q.push(b, 20);
  q.push(a, 4);
  TEST_ASSERT_TRUE(q.pop(out, 10, n));
  TEST_ASSERT_EQUAL(4, n);
  TEST_ASSERT_FALSE(q.push(a, 0));
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_versions);
  RUN_TEST(test_sha_parsing);
  RUN_TEST(test_refusals);
  RUN_TEST(test_downgrade_and_same_need_force);
  RUN_TEST(test_refused_begin_doesnt_open);
  RUN_TEST(test_full_transfer_with_acks);
  RUN_TEST(test_timed_ack_when_the_stream_pauses);
  RUN_TEST(test_dropped_packet_asks_for_a_resend);
  RUN_TEST(test_duplicates_and_overlaps);
  RUN_TEST(test_duplicates_get_a_fresh_ack);
  RUN_TEST(test_resume_after_reconnect);
  RUN_TEST(test_resume_ignores_the_version_check);
  RUN_TEST(test_a_different_image_starts_over);
  RUN_TEST(test_refusal_keeps_an_open_session);
  RUN_TEST(test_sha_mismatch_aborts);
  RUN_TEST(test_corrupted_byte_fails_the_sha);
  RUN_TEST(test_end_before_all_bytes_keeps_the_session);
  RUN_TEST(test_data_past_the_end_fails);
  RUN_TEST(test_not_an_app_image);
  RUN_TEST(test_header_split_over_tiny_packets);
  RUN_TEST(test_cancel);
  RUN_TEST(test_idle_timeout);
  RUN_TEST(test_flash_failures);
  RUN_TEST(test_packet_header);
  RUN_TEST(test_packet_ring);
  return UNITY_END();
}
