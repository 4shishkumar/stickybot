// The IR remote's codes (logic/ir_codec.*, logic/ir_names.*): every protocol's encoder against its
// decoder, against known codes from published tables (built here bit by bit, not with the
// encoder), with a receiver's distortion, repeats and toggles; raw captures (cleaning, comparing,
// the compact text form); code strings; names and the @ir command line.
#include <unity.h>
#include <math.h>
#include <string.h>
#include <vector>
#include "logic/ir_codec.h"
#include "logic/ir_names.h"
#include "logic/ir_doc.h"
#include "logic/ir_learn.h"

using namespace ircodec;

void setUp() {}
void tearDown() {}

static Code make(Proto p, uint32_t a, uint32_t c, uint8_t bits = 0, uint16_t vendor = 0) {
  Code x;
  x.proto = p;
  x.addr = a;
  x.cmd = c;
  x.bits = bits;
  x.vendor = vendor;
  return x;
}

// Pulse distance timings for `nbits` of v, MSB of v first (the "as transmitted" hex codes in
// LIRC / remote databases)
static std::vector<uint16_t> pd(uint16_t hm, uint16_t hs, uint16_t bm, uint16_t one, uint16_t zero, uint64_t v, int nbits) {
  std::vector<uint16_t> t = {hm, hs};
  for (int i = nbits - 1; i >= 0; i--) {
    t.push_back(bm);
    t.push_back((v >> i) & 1 ? one : zero);
  }
  t.push_back(bm);
  return t;
}

// What a TSOP-style receiver does: marks come out longer, spaces shorter
static std::vector<uint16_t> distort(std::vector<uint16_t> t, int by = 110) {
  for (size_t i = 0; i < t.size(); i++) t[i] = i % 2 == 0 ? t[i] + by : t[i] - by;
  return t;
}

static Code decoded(const std::vector<uint16_t>& t) {
  Code c;
  TEST_ASSERT_TRUE_MESSAGE(decode(t.data(), t.size(), c), "didn't decode");
  return c;
}

static void roundTrip(const Code& c, bool toggle = false) {
  uint16_t t[MAX_FRAME];
  size_t n = encode(c, t, MAX_FRAME, toggle);
  TEST_ASSERT_TRUE(n > 0);
  TEST_ASSERT_TRUE(n % 2 == 1);
  Code back;
  TEST_ASSERT_TRUE(decode(t, n, back));
  TEST_ASSERT_TRUE(back == c);
  std::vector<uint16_t> v(t, t + n);
  TEST_ASSERT_TRUE(decoded(distort(v)) == c);
}

void test_round_trips() {
  roundTrip(make(Proto::NEC, 0x04, 0x08));
  roundTrip(make(Proto::NEC, 0x7F00, 0x15));        // extended address
  roundTrip(make(Proto::NEC, 0x86, 0x6B87));        // a command without its inverse
  roundTrip(make(Proto::SAMSUNG, 0x07, 0x02));
  roundTrip(make(Proto::SAMSUNG, 0x0E07, 0x0C));
  roundTrip(make(Proto::LG, 0x88, 0x0C05));
  roundTrip(make(Proto::SONY, 1, 21, 12));
  roundTrip(make(Proto::SONY, 0xA4, 0x2F, 15));
  roundTrip(make(Proto::SONY, 0x1A3A, 0x7C, 20));
  roundTrip(make(Proto::RC5, 0, 12));
  roundTrip(make(Proto::RC5, 0x1F, 0x7F));
  roundTrip(make(Proto::RC5, 5, 0x40), true);
  roundTrip(make(Proto::RC6, 0, 0x0C));
  roundTrip(make(Proto::RC6, 0xFF, 0x00), true);
  roundTrip(make(Proto::RC6, 0x5A, 0xA5));
  roundTrip(make(Proto::KASEIKYO, 0x8, 0x3D, 0, PANASONIC_VENDOR));
  roundTrip(make(Proto::KASEIKYO, 0xABC, 0x12, 0, 0x5432));
}

// LG TVs (NEC): power is 0x20DF10EF in LIRC's LG files = address 0x04, command 0x08
void test_known_nec_lg_tv_power() {
  Code c = decoded(pd(9000, 4500, 560, 1690, 560, 0x20DF10EFull, 32));
  TEST_ASSERT_TRUE(c == make(Proto::NEC, 0x04, 0x08));
  c = decoded(distort(pd(9000, 4500, 560, 1690, 560, 0x20DF40BFull, 32)));  // volume up
  TEST_ASSERT_TRUE(c == make(Proto::NEC, 0x04, 0x02));
}

// Samsung TVs: power 0xE0E040BF = Samsung32 address 0x07, command 0x02
void test_known_samsung_power() {
  Code c = decoded(pd(4500, 4500, 560, 1690, 560, 0xE0E040BFull, 32));
  TEST_ASSERT_TRUE(c == make(Proto::SAMSUNG, 0x07, 0x02));
  c = decoded(pd(4500, 4500, 560, 1690, 560, 0xE0E0F00Full, 32));  // mute
  TEST_ASSERT_TRUE(c == make(Proto::SAMSUNG, 0x07, 0x0F));
}

// Panasonic TV power 0x40040100BCBD (48 bits as transmitted) = address 0x8, command 0x3D
void test_known_panasonic_power() {
  Code c = decoded(pd(3456, 1728, 432, 1296, 432, 0x40040100BCBDull, 48));
  TEST_ASSERT_TRUE(c == make(Proto::KASEIKYO, 0x8, 0x3D, 0, PANASONIC_VENDOR));
  char s[40];
  TEST_ASSERT_TRUE(format(c, s, sizeof s) > 0);
  TEST_ASSERT_EQUAL_STRING("panasonic:8:3d", s);
}

// Sony TV power: SIRC-12, device 1, command 21 (SB-Projects' table), LSB first, marks 1200 / 600
void test_known_sony_power() {
  std::vector<uint16_t> t = {2400, 600};
  const uint32_t v = 21 | 1 << 7;
  for (int i = 0; i < 12; i++) {
    t.push_back((v >> i) & 1 ? 1200 : 600);
    if (i < 11) t.push_back(600);
  }
  Code c = decoded(distort(t, 90));
  TEST_ASSERT_TRUE(c == make(Proto::SONY, 1, 21, 12));
}

// RC5 address 0, command 12 (TV standby): 1 1 T 00000 001100, Manchester 889 us
void test_known_rc5_standby() {
  const int bits[14] = {1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0};
  std::vector<uint8_t> lv;
  for (int b : bits) {
    lv.push_back(b ? 0 : 1);
    lv.push_back(b ? 1 : 0);
  }
  std::vector<uint16_t> t;
  size_t i = 1;  // skip the start bit's first (space) half
  while (i < lv.size()) {
    size_t j = i;
    while (j < lv.size() && lv[j] == lv[i]) j++;
    if (!(lv[i] == 0 && j == lv.size())) t.push_back((j - i) * 889);
    i = j;
  }
  Code c = decoded(t);
  TEST_ASSERT_TRUE(c == make(Proto::RC5, 0, 12));
}

void test_toggle_is_not_part_of_the_code() {
  uint16_t a[MAX_FRAME], b[MAX_FRAME];
  Code c = make(Proto::RC6, 0, 0x0C);
  size_t na = encode(c, a, MAX_FRAME, false), nb = encode(c, b, MAX_FRAME, true);
  TEST_ASSERT_FALSE(na == nb && !memcmp(a, b, na * 2));  // the frames differ...
  Code x, y;
  TEST_ASSERT_TRUE(decode(a, na, x));
  TEST_ASSERT_TRUE(decode(b, nb, y));
  TEST_ASSERT_TRUE(x == y);  // ...the code doesn't
}

void test_nec_repeat_frame_and_captures() {
  uint16_t r[8];
  TEST_ASSERT_EQUAL(3, encodeRepeat(make(Proto::NEC, 4, 8), r, 8));
  TEST_ASSERT_TRUE(isNecRepeat(r, 3));
  Code dummy;
  TEST_ASSERT_FALSE(decode(r, 3, dummy));
  // A held NEC button: the frame, a 40 ms gap, two repeat frames
  uint16_t t[MAX_RAW];
  size_t n = encode(make(Proto::NEC, 4, 8), t, MAX_RAW);
  const size_t frame = n;
  for (int k = 0; k < 2; k++) {
    t[n++] = 40000;
    t[n++] = 9100;
    t[n++] = 2200;
    t[n++] = 600;
  }
  TEST_ASSERT_EQUAL(frame, firstFrame(t, n));
  Code c;
  TEST_ASSERT_TRUE(decodeCapture(t, n, c));
  TEST_ASSERT_TRUE(c == make(Proto::NEC, 4, 8));
  TEST_ASSERT_EQUAL(frame, cleanRaw(t, n));  // the repeats go
  // the others repeat whole frames; Sony three at least
  TEST_ASSERT_EQUAL(3, minFrames(Proto::SONY));
  TEST_ASSERT_EQUAL(45000, periodUs(Proto::SONY));
  TEST_ASSERT_EQUAL(0, encodeRepeat(make(Proto::SONY, 1, 21, 12), r, 8));  // a whole frame: doesn't fit in 8
  uint16_t big[MAX_FRAME];
  TEST_ASSERT_EQUAL(25, encodeRepeat(make(Proto::SONY, 1, 21, 12), big, MAX_FRAME));
}

void test_not_a_protocol() {
  // an air conditioner's long frame: 3400/1700 header, 100 bits of 420 / 1300 / 420, not kaseikyo's 48
  std::vector<uint16_t> t = {3400, 1700};
  for (int i = 0; i < 100; i++) {
    t.push_back(420);
    t.push_back(i % 3 ? 1300 : 420);
  }
  t.push_back(420);
  Code c;
  TEST_ASSERT_FALSE(decode(t.data(), t.size(), c));
  TEST_ASSERT_FALSE(decodeCapture(t.data(), t.size(), c));
  // a broken NEC (one space way off)
  std::vector<uint16_t> nec = pd(9000, 4500, 560, 1690, 560, 0x20DF10EFull, 32);
  nec[11] = 3000;
  TEST_ASSERT_FALSE(decode(nec.data(), nec.size(), c));
  // an even count / too short
  TEST_ASSERT_FALSE(decode(nec.data(), nec.size() - 1, c));
  TEST_ASSERT_FALSE(decode(nec.data(), 1, c));
  // LG with a wrong checksum
  std::vector<uint16_t> lg = pd(8500, 4250, 550, 1600, 550, (0x88ull << 20) | (0x0C05 << 4) | 0x3, 28);
  TEST_ASSERT_FALSE(decode(lg.data(), lg.size(), c));
}

void test_clean_and_compare_raw() {
  std::vector<uint16_t> a = {3400, 1700, 420, 1300, 420, 420, 420, 30000, 3400, 1700, 420, 420, 420, 5000};
  size_t n = cleanRaw(a.data(), a.size());  // the trailing space goes, the frames stay
  TEST_ASSERT_EQUAL(13, n);
  std::vector<uint16_t> b(a.begin(), a.begin() + n);
  for (size_t i = 0; i < b.size(); i++) b[i] = b[i] * 110 / 100;  // 10 % slower
  TEST_ASSERT_TRUE(sameRaw(a.data(), n, b.data(), n));
  b[3] = 2600;  // a 0 became a 1
  TEST_ASSERT_FALSE(sameRaw(a.data(), n, b.data(), n));
  TEST_ASSERT_FALSE(sameRaw(a.data(), n, b.data(), n - 2));
  b[3] = a[3];
  b[7] = 45000;  // the gap between frames is timed loosely
  TEST_ASSERT_TRUE(sameRaw(a.data(), n, b.data(), n));
}

void test_code_strings() {
  struct { const char* s; Code c; } cases[] = {
      {"nec:4:8", make(Proto::NEC, 4, 8)},
      {"nec:7f00:15", make(Proto::NEC, 0x7F00, 0x15)},
      {"samsung:7:2", make(Proto::SAMSUNG, 7, 2)},
      {"lg:88:c05", make(Proto::LG, 0x88, 0xC05)},
      {"sony12:1:15", make(Proto::SONY, 1, 0x15, 12)},
      {"sony15:a4:2f", make(Proto::SONY, 0xA4, 0x2F, 15)},
      {"sony20:1a3a:7c", make(Proto::SONY, 0x1A3A, 0x7C, 20)},
      {"rc5:0:c", make(Proto::RC5, 0, 12)},
      {"rc6:0:c", make(Proto::RC6, 0, 12)},
      {"panasonic:8:3d", make(Proto::KASEIKYO, 8, 0x3D, 0, PANASONIC_VENDOR)},
      {"kaseikyo:5432:abc:12", make(Proto::KASEIKYO, 0xABC, 0x12, 0, 0x5432)},
  };
  for (auto& k : cases) {
    Parsed p;
    TEST_ASSERT_TRUE_MESSAGE(parse(k.s, p, nullptr), k.s);
    TEST_ASSERT_TRUE_MESSAGE(p.code == k.c, k.s);
    char out[40];
    TEST_ASSERT_TRUE(format(p.code, out, sizeof out) > 0);
    TEST_ASSERT_EQUAL_STRING(k.s, out);
  }
  Parsed p;
  TEST_ASSERT_TRUE(parse("NEC:04:08", p, nullptr));  // upper case, leading zeros
  TEST_ASSERT_TRUE(p.code == make(Proto::NEC, 4, 8));
  TEST_ASSERT_TRUE(parse("sony:1:15", p, nullptr));
  const char* bad[] = {"", "nec", "nec:4", "nec:4:8:1", "nec:x:8", "nec:10000:8", "sony12:20:1", "sony12:1:80",
                       "rc5:20:1", "lg:100:1", "kaseikyo:1:2", "tv:1:2", "nec::8", "panasonic:1000:1",
                       "nec:123456789:1"};
  for (const char* s : bad) TEST_ASSERT_FALSE_MESSAGE(parse(s, p, nullptr), s);
  char tiny[6];
  TEST_ASSERT_EQUAL(0, format(make(Proto::NEC, 4, 8), tiny, sizeof tiny));
}

void test_raw_strings() {
  static uint16_t raw[MAX_RAW];
  Parsed p;
  TEST_ASSERT_TRUE(parse("raw:38:9000,4500,560,1690,560", p, raw));
  TEST_ASSERT_EQUAL(5, p.n);
  TEST_ASSERT_EQUAL(38, p.khz);
  TEST_ASSERT_EQUAL(1690, raw[3]);
  TEST_ASSERT_TRUE(parse("raw:36:900,400", p, raw));  // a trailing space isn't sent
  TEST_ASSERT_EQUAL(1, p.n);
  TEST_ASSERT_TRUE(parse("raw:38:3400,1700,420,1300:0123232", p, raw));  // compact
  TEST_ASSERT_EQUAL(7, p.n);
  TEST_ASSERT_EQUAL(1300, raw[5]);
  TEST_ASSERT_EQUAL(420, raw[6]);
  const char* bad[] = {"raw:38:", "raw:38:1,,2", "raw:38:1,2,", "raw:10:1", "raw:99:1", "raw:38:0,5,5",
                       "raw:38:70000", "raw:38:1,2:013", "raw:38:1,2:z", "raw:38:1,2:", "raw:38", "raw:38:a,b"};
  for (const char* s : bad) TEST_ASSERT_FALSE_MESSAGE(parse(s, p, raw), s);
  TEST_ASSERT_FALSE(parse("raw:38:1,2,3", p, nullptr));  // no room

  // An air conditioner frame: written compact, read back within the tolerance
  std::vector<uint16_t> t = {3400, 1700};
  for (int i = 0; i < 280; i++) {
    t.push_back(420 + (i * 7) % 40);           // jitter
    t.push_back(i % 3 ? 1300 - (i % 5) * 9 : 430);
  }
  t.push_back(440);
  t.push_back(29000);
  t.push_back(3400);
  t.push_back(1700);
  t.push_back(430);
  static char s[8192];
  size_t len = formatRaw(t.data(), t.size(), 38, s, sizeof s);
  TEST_ASSERT_TRUE(len > 0);
  TEST_ASSERT_TRUE(len < t.size() + 60);  // one character per duration + the values
  TEST_ASSERT_TRUE(parse(s, p, raw));
  TEST_ASSERT_EQUAL(t.size(), p.n);
  TEST_ASSERT_TRUE(sameRaw(t.data(), t.size(), raw, p.n));
  // too varied to cluster in 36 values: written plainly
  std::vector<uint16_t> wild;
  for (int i = 0; i < 41; i++) wild.push_back((uint16_t)(100 * pow(1.15, i)));
  len = formatRaw(wild.data(), wild.size(), 40, s, sizeof s);
  TEST_ASSERT_TRUE(len > 0);
  TEST_ASSERT_TRUE(parse(s, p, raw));
  TEST_ASSERT_EQUAL(41, p.n);
  TEST_ASSERT_EQUAL(40, p.khz);
  TEST_ASSERT_EQUAL(wild[40], raw[40]);
  TEST_ASSERT_EQUAL(0, formatRaw(wild.data(), wild.size(), 38, s, 50));  // doesn't fit
}

void test_names() {
  char d[irnames::DEVICE_LEN + 1], b[irnames::BUTTON_LEN + 1];
  TEST_ASSERT_TRUE(irnames::deviceId("  Living Room_TV ", d));
  TEST_ASSERT_EQUAL_STRING("living-room-tv", d);
  TEST_ASSERT_TRUE(irnames::deviceId("AC!", d));
  TEST_ASSERT_EQUAL_STRING("ac", d);
  TEST_ASSERT_TRUE(irnames::deviceId("a very long device name indeed", d));
  TEST_ASSERT_EQUAL_STRING("a-very-long-devi", d);
  TEST_ASSERT_FALSE(irnames::deviceId(" -- ", d));
  TEST_ASSERT_FALSE(irnames::deviceId("../..", d) && strchr(d, '/'));
  TEST_ASSERT_TRUE(irnames::buttonName("  Volume   Up ", b));
  TEST_ASSERT_EQUAL_STRING("volume up", b);
  TEST_ASSERT_TRUE(irnames::buttonName("Cool 24°C", b));
  TEST_ASSERT_EQUAL_STRING("cool 24c", b);
  TEST_ASSERT_TRUE(irnames::buttonName("ch+", b));
  TEST_ASSERT_EQUAL_STRING("ch+", b);
  TEST_ASSERT_FALSE(irnames::buttonName("\"/\"", b));
  const char* names[] = {"power", "button 1", "button 3"};
  TEST_ASSERT_EQUAL(2, irnames::nextNumber(names, 3, "button"));
  TEST_ASSERT_EQUAL(1, irnames::nextNumber(names, 0, "remote"));
}

void test_query() {
  char q[120];
  TEST_ASSERT_TRUE(irnames::normalizeQuery("list", q, sizeof q));
  TEST_ASSERT_EQUAL_STRING("op=list", q);
  irnames::normalizeQuery("send dev=tv btn=volume up repeat=3", q, sizeof q);
  TEST_ASSERT_EQUAL_STRING("op=send&dev=tv&btn=volume%20up&repeat=3", q);
  irnames::normalizeQuery("op=send&dev=tv&btn=power", q, sizeof q);
  TEST_ASSERT_EQUAL_STRING("op=send&dev=tv&btn=power", q);
  irnames::normalizeQuery("learn once dev=ac", q, sizeof q);
  TEST_ASSERT_EQUAL_STRING("op=learn&once=1&dev=ac", q);
  irnames::normalizeQuery("", q, sizeof q);
  TEST_ASSERT_EQUAL_STRING("", q);
  TEST_ASSERT_EQUAL(0, irnames::normalizeQuery("send dev=tv btn=power", q, 10));
}

void test_device_file() {
  irdoc::Device d;
  d.id = "tv";
  const char* why = nullptr;
  TEST_ASSERT_TRUE(d.put("Power", "nec:4:8", true, why));
  TEST_ASSERT_TRUE(d.put("volume up", "nec:4:2", false, why));
  TEST_ASSERT_TRUE(d.put("cool 24", "raw:38:3400,1700,420,1300:0123232", false, why));
  TEST_ASSERT_FALSE(d.put("bad", "nec:zz:1", false, why));
  TEST_ASSERT_EQUAL_STRING("code", why);
  TEST_ASSERT_FALSE(d.put("///", "nec:4:8", false, why));
  TEST_ASSERT_EQUAL_STRING("name", why);
  TEST_ASSERT_TRUE(d.put("POWER", "nec:4:9", true, why));  // same name: replaced, order kept
  TEST_ASSERT_EQUAL(3, d.buttons.size());
  TEST_ASSERT_EQUAL_STRING("nec:4:9", d.buttons[0].code.c_str());
  TEST_ASSERT_EQUAL_STRING("button 1", d.nextName().c_str());

  std::string text = irdoc::write(d);
  TEST_ASSERT_EQUAL_STRING(
      "{\"v\":1,\"dev\":\"tv\",\"buttons\":[{\"n\":\"power\",\"c\":\"nec:4:9\",\"f\":1},"
      "{\"n\":\"volume up\",\"c\":\"nec:4:2\"},{\"n\":\"cool 24\",\"c\":\"raw:38:3400,1700,420,1300:0123232\"}]}",
      text.c_str());
  irdoc::Device back;
  TEST_ASSERT_TRUE(irdoc::read(text.data(), text.size(), back, why));
  TEST_ASSERT_EQUAL_STRING("tv", back.id.c_str());
  TEST_ASSERT_EQUAL(3, back.buttons.size());
  TEST_ASSERT_TRUE(back.buttons[0].fav);
  TEST_ASSERT_FALSE(back.buttons[1].fav);
  TEST_ASSERT_EQUAL_STRING(text.c_str(), irdoc::write(back).c_str());

  TEST_ASSERT_TRUE(back.rename("volume up", "Vol +", why));
  TEST_ASSERT_EQUAL(1, back.find("vol +"));
  TEST_ASSERT_FALSE(back.rename("vol +", "power", why));
  TEST_ASSERT_EQUAL_STRING("exists", why);
  TEST_ASSERT_FALSE(back.rename("nope", "x", why));
  TEST_ASSERT_TRUE(back.remove("power"));
  TEST_ASSERT_FALSE(back.remove("power"));
  TEST_ASSERT_EQUAL(2, back.buttons.size());
}

void test_device_file_is_checked() {
  irdoc::Device d;
  const char* why = nullptr;
  const char* bad[][2] = {
      {"", "size"}, {"{", "json"}, {"{\"v\":2,\"dev\":\"tv\"}", "format"},
      {"{\"v\":1,\"dev\":\"../x\"}", "dev"}, {"{\"v\":1,\"dev\":\"TV\"}", "dev"}, {"{\"v\":1}", "dev"}};
  for (auto& b : bad) {
    TEST_ASSERT_FALSE_MESSAGE(irdoc::read(b[0], strlen(b[0]), d, why), b[0]);
    TEST_ASSERT_EQUAL_STRING_MESSAGE(b[1], why, b[0]);
  }
  // bad buttons are skipped, good ones kept; duplicates merge
  const char* mixed = "{\"v\":1,\"dev\":\"ac\",\"buttons\":[{\"n\":\"on\",\"c\":\"nec:1:2\"},{\"n\":\"\",\"c\":\"nec:1:3\"},"
                      "{\"n\":\"x\",\"c\":\"raw:38:\"},{\"n\":\"ON\",\"c\":\"nec:1:4\"},{\"c\":1},5]}";
  TEST_ASSERT_TRUE(irdoc::read(mixed, strlen(mixed), d, why));
  TEST_ASSERT_EQUAL(1, d.buttons.size());
  TEST_ASSERT_EQUAL_STRING("nec:1:4", d.buttons[0].code.c_str());
  // the limit
  std::string many = "{\"v\":1,\"dev\":\"big\",\"buttons\":[";
  for (int i = 0; i < irnames::MAX_BUTTONS + 5; i++)
    many += std::string(i ? "," : "") + "{\"n\":\"b" + std::to_string(i) + "\",\"c\":\"nec:1:" + std::to_string(i % 10) + "\"}";
  many += "]}";
  TEST_ASSERT_TRUE(irdoc::read(many.data(), many.size(), d, why));
  TEST_ASSERT_EQUAL(irnames::MAX_BUTTONS, d.buttons.size());
  TEST_ASSERT_FALSE(d.put("one more", "nec:1:1", false, why));
  TEST_ASSERT_EQUAL_STRING("full", why);
}

static std::vector<uint16_t> acFrame(int flip = -1) {
  std::vector<uint16_t> t = {3400, 1700};
  for (int i = 0; i < 120; i++) {
    t.push_back(430);
    t.push_back((i % 3 == 0) != (i == flip) ? 1290 : 440);
  }
  t.push_back(430);
  return t;
}

void test_learning() {
  static irlearn::Learner L;
  char code[2048];
  // a protocol code: two presses agree (the second one held: repeat frames)
  L.start(false);
  std::vector<uint16_t> blip = {300};
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::IGNORED, (int)L.capture(blip.data(), blip.size()));
  std::vector<uint16_t> nec = distort(pd(9000, 4500, 560, 1690, 560, 0x20DF10EFull, 32));
  std::vector<uint16_t> a = nec;
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::FIRST, (int)L.capture(a.data(), a.size()));
  std::vector<uint16_t> rep = {9000, 2250, 560};
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::IGNORED, (int)L.capture(rep.data(), rep.size()));
  std::vector<uint16_t> b = nec;
  b.push_back(40000);
  b.insert(b.end(), rep.begin(), rep.end());
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::DONE, (int)L.capture(b.data(), b.size()));
  TEST_ASSERT_TRUE(L.decoded());
  TEST_ASSERT_TRUE(L.code(code, sizeof code) > 0);
  TEST_ASSERT_EQUAL_STRING("nec:4:8", code);

  // a different button the second time: press again, that one is the first now
  L.start(false);
  a = nec;
  L.capture(a.data(), a.size());
  std::vector<uint16_t> other = pd(9000, 4500, 560, 1690, 560, 0x20DF40BFull, 32);
  b = other;
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::MISMATCH, (int)L.capture(b.data(), b.size()));
  TEST_ASSERT_EQUAL(1, L.mismatches());
  b = other;
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::DONE, (int)L.capture(b.data(), b.size()));
  L.code(code, sizeof code);
  TEST_ASSERT_EQUAL_STRING("nec:4:2", code);

  // raw: two presses of an air conditioner button, a little jitter -> the compact raw code
  L.start(false);
  a = acFrame();
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::FIRST, (int)L.capture(a.data(), a.size()));
  b = distort(acFrame(), 60);
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::DONE, (int)L.capture(b.data(), b.size()));
  TEST_ASSERT_FALSE(L.decoded());
  TEST_ASSERT_TRUE(L.code(code, sizeof code) > 0);
  TEST_ASSERT_EQUAL(0, strncmp(code, "raw:38:", 7));
  ircodec::Parsed p;
  static uint16_t raw[MAX_RAW];
  TEST_ASSERT_TRUE(parse(code, p, raw));
  std::vector<uint16_t> ref = acFrame();
  TEST_ASSERT_TRUE(sameRaw(ref.data(), ref.size(), raw, p.n));
  // ...a changed bit doesn't match
  L.start(false);
  a = acFrame();
  L.capture(a.data(), a.size());
  b = acFrame(7);
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::MISMATCH, (int)L.capture(b.data(), b.size()));
  // ...held longer the second time (two frames): the same first frame is enough, one frame kept
  L.start(false);
  a = acFrame();
  L.capture(a.data(), a.size());
  b = acFrame();
  b.push_back(30000);
  std::vector<uint16_t> again = acFrame();
  b.insert(b.end(), again.begin(), again.end());
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::DONE, (int)L.capture(b.data(), b.size()));
  TEST_ASSERT_EQUAL(ref.size(), L.rawCount());
  // once: one press is enough
  L.start(true);
  a = acFrame(3);
  TEST_ASSERT_EQUAL((int)irlearn::Outcome::DONE, (int)L.capture(a.data(), a.size()));
  TEST_ASSERT_TRUE(L.done());
}

int main(int, char**) {
  UNITY_BEGIN();
  RUN_TEST(test_round_trips);
  RUN_TEST(test_known_nec_lg_tv_power);
  RUN_TEST(test_known_samsung_power);
  RUN_TEST(test_known_panasonic_power);
  RUN_TEST(test_known_sony_power);
  RUN_TEST(test_known_rc5_standby);
  RUN_TEST(test_toggle_is_not_part_of_the_code);
  RUN_TEST(test_nec_repeat_frame_and_captures);
  RUN_TEST(test_not_a_protocol);
  RUN_TEST(test_clean_and_compare_raw);
  RUN_TEST(test_code_strings);
  RUN_TEST(test_raw_strings);
  RUN_TEST(test_names);
  RUN_TEST(test_query);
  RUN_TEST(test_device_file);
  RUN_TEST(test_device_file_is_checked);
  RUN_TEST(test_learning);
  return UNITY_END();
}
