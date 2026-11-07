// The compiled face container (.sfb): headers, image headers, PackBits and frame order, and
// that no truncated or corrupt file is read past its end.
#include <unity.h>
#include <string.h>
#include <vector>
#include "logic/sfb.h"

void setUp() {}
void tearDown() {}

static void u16(std::vector<uint8_t>& v, uint16_t x) { v.push_back(x & 255); v.push_back(x >> 8); }
static void u32(std::vector<uint8_t>& v, uint32_t x) { for (int i = 0; i < 4; i++) v.push_back(x >> (8 * i)); }
static void pad(std::vector<uint8_t>& v) { while (v.size() % 4) v.push_back(0); }

// A 4 x 2 sprite sheet, 2 frames of 2 x 2, 2 bits per pixel, 3 colours, index 0 clear:
//   row 0: 1 2 | 0 1      row 1: 2 2 | 1 0
static std::vector<uint8_t> sheet(bool packbits, const char* manifest = "{\"id\":\"tiny\"}") {
  std::vector<uint8_t> v = {'S', 'F', 'B', '1'};
  u16(v, 1);
  u16(v, 1);
  u32(v, strlen(manifest));
  u16(v, 1);
  u16(v, 0);
  v.insert(v.end(), manifest, manifest + strlen(manifest));
  pad(v);
  const uint8_t rows[2] = {0b01100001, 0b10100100};
  std::vector<uint8_t> data;
  if (packbits) data = {0x00, rows[0], 0x00, rows[1]};  // two literal runs of one byte
  else data = {rows[0], rows[1]};
  u16(v, 4);
  u16(v, 2);
  u16(v, 2);
  v.push_back(2);    // bits
  v.push_back(2);    // colours - 1
  v.push_back(0);    // clear
  v.push_back(packbits ? 1 : 0);
  u16(v, 0);
  u32(v, data.size());
  u16(v, 0x0000);
  u16(v, 0xF800);
  u16(v, 0x07E0);
  pad(v);
  v.insert(v.end(), data.begin(), data.end());
  pad(v);
  return v;
}

void test_reads_a_sheet() {
  for (bool pb : {false, true}) {
    std::vector<uint8_t> v = sheet(pb);
    sfb::File f;
    TEST_ASSERT_EQUAL(sfb::Error::NONE, sfb::parse(v.data(), v.size(), f));
    TEST_ASSERT_EQUAL(1, f.nImages);
    TEST_ASSERT_EQUAL(13, f.manifestLen);
    TEST_ASSERT_EQUAL(0, memcmp(f.manifest, "{\"id\":\"tiny\"}", 13));
    const sfb::Image& im = f.images[0];
    TEST_ASSERT_EQUAL(2, im.frames);
    TEST_ASSERT_EQUAL(2, im.fw);
    TEST_ASSERT_EQUAL(0, im.clear);
    TEST_ASSERT_EQUAL_HEX16(0xF800, im.paletteAt(1));
    uint8_t px[8];
    TEST_ASSERT_TRUE(sfb::unpack(im, px, nullptr));
    const uint8_t want[8] = {1, 2, 2, 2,   0, 1, 1, 0};  // frame 0 (2 x 2), then frame 1
    TEST_ASSERT_EQUAL_UINT8_ARRAY(want, px, 8);
  }
}

void test_refuses_bad_headers() {
  std::vector<uint8_t> v = sheet(false);
  sfb::File f;
  std::vector<uint8_t> b = v;
  b[0] = 'X';
  TEST_ASSERT_EQUAL(sfb::Error::NOT_SFB, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[6] = 2;  // face format 2
  TEST_ASSERT_EQUAL(sfb::Error::FORMAT, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[4] = 9;
  TEST_ASSERT_EQUAL(sfb::Error::CONTAINER, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[8] = 0xFF;  // manifest longer than the file
  TEST_ASSERT_EQUAL(sfb::Error::SIZES, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[32 + 6] = 3;  // 3 bits per pixel
  TEST_ASSERT_EQUAL(sfb::Error::IMAGE_HEADER, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[32 + 4] = 3;  // frames of 3 in a width of 4
  TEST_ASSERT_EQUAL(sfb::Error::IMAGE_HEADER, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[32 + 8] = 7;  // clear index past the palette
  TEST_ASSERT_EQUAL(sfb::Error::IMAGE_HEADER, sfb::parse(b.data(), b.size(), f));
  b = v;
  b[32 + 12] = 0xFF;  // data longer than the file
  b[32 + 13] = 0xFF;
  TEST_ASSERT_EQUAL(sfb::Error::IMAGE_SIZE, sfb::parse(b.data(), b.size(), f));
  b.assign(sfb::MAX_SIZE + 1, 0);
  memcpy(b.data(), v.data(), v.size());
  TEST_ASSERT_EQUAL(sfb::Error::TOO_BIG, sfb::parse(b.data(), b.size(), f));
}

void test_every_truncation_is_refused_or_safe() {
  for (bool pb : {false, true}) {
    std::vector<uint8_t> v = sheet(pb);
    for (size_t n = 0; n < v.size(); n++) {
      std::vector<uint8_t> b(v.begin(), v.begin() + n);  // its own allocation: ASan-friendly
      sfb::File f;
      if (sfb::parse(b.data(), b.size(), f) != sfb::Error::NONE) continue;
      uint8_t px[8];
      sfb::unpack(f.images[0], px, nullptr);  // whatever it says, it stays in bounds
    }
  }
}

void test_bad_data() {
  std::vector<uint8_t> v = sheet(true);
  sfb::File f;
  sfb::parse(v.data(), v.size(), f);
  sfb::Image im = f.images[0];
  uint8_t px[8];
  const char* why = nullptr;
  const uint8_t longRun[] = {0x81, 0x00};  // 128 copies: past the image
  im.data = longRun;
  im.dataLen = 2;
  TEST_ASSERT_FALSE(sfb::unpack(im, px, &why));
  TEST_ASSERT_EQUAL_STRING("image data too long", why);
  const uint8_t shortData[] = {0x00, 0x61};  // one row of two
  im.data = shortData;
  TEST_ASSERT_FALSE(sfb::unpack(im, px, &why));
  TEST_ASSERT_EQUAL_STRING("short image data", why);
  const uint8_t index3[] = {0xFF, 0xFF};  // index 3 with 3 colours
  im.enc = 0;
  im.data = index3;
  TEST_ASSERT_FALSE(sfb::unpack(im, px, &why));
  TEST_ASSERT_EQUAL_STRING("palette index", why);
}

int main() {
  UNITY_BEGIN();
  RUN_TEST(test_reads_a_sheet);
  RUN_TEST(test_refuses_bad_headers);
  RUN_TEST(test_every_truncation_is_refused_or_safe);
  RUN_TEST(test_bad_data);
  return UNITY_END();
}
