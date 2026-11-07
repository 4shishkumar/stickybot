// The compiled face file (.sfb, docs/faces/FORMAT.md section 15): its header, the image
// headers and the images' palette indices (1/2/4/8 bits, maybe PackBits). Everything is
// checked against the file's size before it is read: a face comes from outside. Pure C++
// (native-testable; the engine in faces/pkg/ parses the manifest on top of this).
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace sfb {

constexpr uint16_t CONTAINER = 1;              // bytes 4-5
constexpr uint16_t FORMAT = 1;                 // the face format this firmware reads (bytes 6-7)
constexpr uint32_t MAX_SIZE = 64 * 1024;       // the whole file
constexpr uint32_t MAX_MANIFEST = 16 * 1024;   // minified JSON
constexpr int      MAX_IMAGES = 16;
constexpr int      MAX_SIDE = 240;             // an image
constexpr int      MAX_FRAME_SIDE = 128;       // a sprite sheet's frame
constexpr int      MAX_FRAMES = 32;
constexpr uint32_t MAX_PIXELS = 96 * 1024;     // all the images together (one byte each in memory)

struct Image {
  uint16_t w = 0, h = 0;
  uint16_t fw = 0;              // a frame's width (the whole width when it has one frame)
  uint16_t frames = 1;
  uint8_t  bits = 8;            // per pixel in the file
  uint16_t colors = 0;          // palette entries (1-256)
  int16_t  clear = -1;          // the transparent index, -1 none
  uint8_t  enc = 0;             // 0 raw, 1 PackBits
  uint32_t dataLen = 0;
  const uint8_t* palette = nullptr;  // colors x u16 LE (RGB565)
  const uint8_t* data = nullptr;
  uint16_t paletteAt(int i) const { return palette[i * 2] | palette[i * 2 + 1] << 8; }
};

struct File {
  uint16_t container = 0, format = 0;
  const char* manifest = nullptr;    // not terminated: manifestLen bytes
  uint32_t manifestLen = 0;
  int      nImages = 0;
  Image    images[MAX_IMAGES];
  uint32_t pixels = 0;               // all the images' pixels
};

// Why parse() refused a file (short words, for logs and @faces "err").
enum class Error : uint8_t { NONE, NOT_SFB, CONTAINER, FORMAT, TOO_BIG, SIZES, IMAGE_HEADER, IMAGE_SIZE, TOO_MANY_PIXELS };
const char* errorText(Error e);

// Checks the header and every image header against n; fills f. (The data is checked by unpack.)
Error parse(const uint8_t* p, size_t n, File& f);

// An image's indices, one byte per pixel, frame by frame (frame k's fw x h pixels together, row
// by row), into out (w x h bytes). false (and why) if the data is short, too long a run, or an
// index is past the palette.
bool unpack(const Image& im, uint8_t* out, const char** why = nullptr);

}  // namespace sfb
