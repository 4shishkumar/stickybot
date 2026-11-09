#include "sfb.h"
#include <string.h>

namespace sfb {

const char* errorText(Error e) {
  switch (e) {
    case Error::NONE:            return "ok";
    case Error::NOT_SFB:         return "not a compiled face";
    case Error::CONTAINER:       return "unknown container version";
    case Error::FORMAT:          return "unknown face format";
    case Error::TOO_BIG:         return "file too big";
    case Error::SIZES:           return "bad sizes";
    case Error::IMAGE_HEADER:    return "bad image header";
    case Error::IMAGE_SIZE:      return "image past the end of the file";
    case Error::TOO_MANY_PIXELS: return "images too big";
  }
  return "?";
}

static uint16_t u16(const uint8_t* p) { return p[0] | p[1] << 8; }
static uint32_t u32(const uint8_t* p) { return p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }
static uint32_t pad4(uint32_t n) { return (4 - n % 4) % 4; }

Error parse(const uint8_t* p, size_t n, File& f) {
  f = File{};
  if (n < 16 || memcmp(p, "SFB1", 4)) return Error::NOT_SFB;
  if (n > MAX_SIZE) return Error::TOO_BIG;
  f.container = u16(p + 4);
  f.format = u16(p + 6);
  if (f.container != CONTAINER) return Error::CONTAINER;
  if (f.format != FORMAT) return Error::FORMAT;
  f.manifestLen = u32(p + 8);
  f.nImages = u16(p + 12);
  if (f.manifestLen == 0 || f.manifestLen > MAX_MANIFEST || 16 + (size_t)f.manifestLen > n || f.nImages > MAX_IMAGES)
    return Error::SIZES;
  f.manifest = (const char*)p + 16;
  size_t pos = 16 + f.manifestLen + pad4(f.manifestLen);
  for (int i = 0; i < f.nImages; i++) {
    if (pos + 16 > n) return Error::IMAGE_SIZE;
    const uint8_t* h = p + pos;
    Image& im = f.images[i];
    im.w = u16(h);
    im.h = u16(h + 2);
    const uint16_t fw = u16(h + 4);
    im.bits = h[6];
    im.colors = h[7] + 1;
    im.clear = h[8] == 255 ? -1 : h[8];
    im.enc = h[9];
    im.dataLen = u32(h + 12);
    if ((im.bits != 1 && im.bits != 2 && im.bits != 4 && im.bits != 8) || im.colors > (1 << im.bits) ||
        im.w < 1 || im.h < 1 || im.w > MAX_SIDE || im.h > MAX_SIDE || im.enc > 1 ||
        (im.clear >= 0 && im.clear >= im.colors))
      return Error::IMAGE_HEADER;
    if (fw) {  // a sprite sheet: frames of fw side by side
      if (im.w % fw || fw > MAX_FRAME_SIDE || im.h > MAX_FRAME_SIDE || im.w / fw > MAX_FRAMES) return Error::IMAGE_HEADER;
      im.fw = fw;
      im.frames = im.w / fw;
    } else {
      im.fw = im.w;
      im.frames = 1;
    }
    pos += 16;
    const size_t palBytes = im.colors * 2 + pad4(im.colors * 2);
    if (pos + palBytes > n) return Error::IMAGE_SIZE;
    im.palette = p + pos;
    pos += palBytes;
    if (im.dataLen > n || pos + im.dataLen > n) return Error::IMAGE_SIZE;
    im.data = p + pos;
    pos += im.dataLen + pad4(im.dataLen);
    f.pixels += (uint32_t)im.w * im.h;
    if (f.pixels > MAX_PIXELS) return Error::TOO_MANY_PIXELS;
  }
  return Error::NONE;
}

namespace {
// The indices as they come (rows padded to a byte, MSB first), one packed byte at a time, out
// as one byte per pixel, frame by frame.
struct Writer {
  const Image& im;
  uint8_t* out;
  uint32_t stride, at = 0, total;
  int per;
  bool bad = false;
  Writer(const Image& i, uint8_t* o) : im(i), out(o) {
    stride = (i.w * i.bits + 7) / 8;
    total = stride * i.h;
    per = 8 / i.bits;
  }
  void put(uint8_t b) {
    const uint32_t y = at / stride, x0 = (at % stride) * per;
    at++;
    const uint8_t mask = (1 << im.bits) - 1;
    for (int k = 0; k < per; k++) {
      const uint32_t x = x0 + k;
      if (x >= im.w) break;  // the row's padding
      const uint8_t v = (b >> (8 - im.bits * (k + 1))) & mask;
      if (v >= im.colors) bad = true;
      const uint32_t f = x / im.fw, fx = x % im.fw;
      out[f * im.fw * im.h + y * im.fw + fx] = v < im.colors ? v : 0;
    }
  }
};
}  // namespace

bool unpack(const Image& im, uint8_t* out, const char** why) {
  Writer w(im, out);
  const uint8_t* in = im.data;
  const uint32_t n = im.dataLen;
  const char* err = nullptr;
  if (im.enc == 0) {
    if (n < w.total) err = "short image data";
    else
      for (uint32_t i = 0; i < w.total; i++) w.put(in[i]);
  } else {
    uint32_t i = 0;
    while (w.at < w.total && !err) {
      if (i >= n) { err = "short image data"; break; }
      const uint8_t c = in[i++];
      if (c < 128) {  // c + 1 literal bytes
        if (i + c + 1 > n) { err = "short image data"; break; }
        if (w.at + c + 1 > w.total) { err = "image data too long"; break; }
        for (int k = 0; k <= c; k++) w.put(in[i + k]);
        i += c + 1;
      } else if (c > 128) {  // one byte, 257 - c times
        if (i >= n) { err = "short image data"; break; }
        if (w.at + (257 - c) > w.total) { err = "image data too long"; break; }
        for (int k = 0; k < 257 - c; k++) w.put(in[i]);
        i++;
      }  // 128: no-op
    }
  }
  if (!err && w.bad) err = "palette index";
  if (why) *why = err;
  return !err;
}

}  // namespace sfb
