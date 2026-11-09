// Infrared remote codes (firmware 2.5.0, src/ir/): the common protocols' encoders and
// decoders, raw captures (cleaning, comparing, the compact text form) and the code strings the
// stick stores and the Mac sends. Pure C++ (native-testable, no Arduino).
//
// Timings are microseconds, marks (carrier on) and spaces (off) alternating, starting with a
// mark and ending with one: t[0] mark, t[1] space, ... t[n-1] mark (n odd). A receiver module
// (TSOP-style) demodulates the carrier, so a capture has no carrier frequency in it: raw codes
// carry the one they are sent with (38 kHz unless told otherwise).
//
// Protocols, with the field layout used here (addr / cmd as the usual code tables write them):
//   nec        9000/4500 header, 32 bits LSB first: addr, ~addr, cmd, ~cmd. An address whose
//              second byte isn't its inverse is NEC "extended": addr = 16 bits (lo | hi << 8);
//              a command without its inverse likewise (16 bits). Repeat: 9000/2250/560.
//   samsung    4500/4500 header, 32 bits LSB first: addr, addr, cmd, ~cmd (Samsung32).
//   lg         8500/4250 header, 28 bits MSB first: addr 8, cmd 16, checksum 4 (the sum of the
//              command's nibbles). LG air conditioners; LG TVs speak NEC.
//   sony12/15/20  2400 header mark, 600 spaces, marks 1200 = 1 / 600 = 0, LSB first: cmd 7,
//              addr 5 / 8 / 13 bits. Sent three times at least (45 ms period).
//   rc5        Manchester, 889 us half-bits, 14 bits: 1, field (= NOT cmd bit 6), toggle,
//              addr 5, cmd 6 (cmd 0-127 with the field bit).
//   rc6        mode 0: 2666/889 leader, 444 us half-bits, start 1, mode 000, toggle (double
//              width), addr 8, cmd 8.
//   kaseikyo   3456/1728 header, 48 bits LSB first: vendor 16, vendor parity 4, addr 12, cmd 8,
//              parity 8 (XOR of the three bytes before it). Panasonic = vendor 0x2002.
//
// Sources: SB-Projects' IR pages (https://www.sbprojects.net/knowledge/ir/ : NEC, SIRC, RC-5,
// RC-6), and the Kaseikyo / LG layouts as Arduino-IRremote documents them (ir_Kaseikyo.hpp,
// ir_LG.hpp, https://github.com/Arduino-IRremote/Arduino-IRremote).
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace ircodec {

enum class Proto : uint8_t { RAW, NEC, SAMSUNG, LG, SONY, RC5, RC6, KASEIKYO };

struct Code {
  Proto    proto = Proto::RAW;
  uint8_t  bits = 0;     // sony: 12 / 15 / 20 (the others have one size)
  uint32_t addr = 0;
  uint32_t cmd = 0;
  uint16_t vendor = 0;   // kaseikyo only
  bool operator==(const Code& o) const {
    return proto == o.proto && bits == o.bits && addr == o.addr && cmd == o.cmd && vendor == o.vendor;
  }
  bool operator!=(const Code& o) const { return !(*this == o); }
};

constexpr uint16_t PANASONIC_VENDOR = 0x2002;
constexpr size_t   MAX_RAW = 800;          // durations in a raw code (air conditioners: ~600)
constexpr size_t   MAX_FRAME = 120;        // durations a protocol frame can have (rc6: ~70)
constexpr uint32_t FRAME_GAP_US = 8000;    // a space longer than this ends a frame
constexpr uint16_t DEFAULT_KHZ = 38;

const char* protoName(const Code& c);      // "nec", "sony12", "panasonic", "raw", ...
uint32_t    carrierHz(Proto p);            // 38000 / 40000 (sony) / 36000 (rc5, rc6) / 37000 (kaseikyo)
uint32_t    periodUs(Proto p);             // from one frame's start to the next when repeating
bool        hasRepeatFrame(Proto p);       // nec / lg: holding sends the short repeat frame
uint8_t     minFrames(Proto p);            // a press sends at least this many frames (sony: 3)

// --- Encoding: one frame's timings into out (max entries); 0 if it doesn't fit / bad code --------
size_t encode(const Code& c, uint16_t* out, size_t max, bool toggle = false);
size_t encodeRepeat(const Code& c, uint16_t* out, size_t max, bool toggle = false);  // what holding sends

// --- Decoding ------------------------------------------------------------------------------
// One frame (no long gaps inside) -> a code. false: not one of the protocols (keep it raw).
bool decode(const uint16_t* t, size_t n, Code& out);
bool isNecRepeat(const uint16_t* t, size_t n);   // the 9000/2250/560 "still held" frame
// The first frame of a capture: its length (up to the first space > FRAME_GAP_US, or n)
size_t firstFrame(const uint16_t* t, size_t n);
// A whole capture (frames joined by their gaps) -> a protocol code from its first frame
bool decodeCapture(const uint16_t* t, size_t n, Code& out);

// --- Raw captures --------------------------------------------------------------------------
// Tidies a raw capture in place: drops a trailing space, NEC repeat frames and frames that only
// repeat the one before (a held button); gaps are capped at 65535. Returns the new length (odd,
// ends with a mark), 0 if nothing usable is left.
size_t cleanRaw(uint16_t* t, size_t n);
// Two captures of the same button: same length, every duration within 25 % + 150 us.
bool sameRaw(const uint16_t* a, size_t na, const uint16_t* b, size_t nb);
bool near(uint32_t measured, uint32_t expected);  // the decoders' tolerance (30 % + 120 us)

// --- Code strings (stored on the stick, sent by the Mac) -------------------------------------
//   nec:<addr hex>:<cmd hex>       samsung:7:2      lg:88:c05      sony12:1:15     rc5:0:c
//   rc6:0:c    panasonic:<addr>:<cmd>   kaseikyo:<vendor>:<addr>:<cmd>
//   raw:<kHz>:<d1>,<d2>,...                       (durations in us, decimal)
//   raw:<kHz>:<v0>,<v1>,...:<index string>        (compact: each character picks a value, 0-9a-z)
struct Parsed {
  Code     code;
  uint16_t khz = DEFAULT_KHZ;     // raw only
  uint16_t n = 0;                 // raw durations in raw[]
};
// raw: room for MAX_RAW durations (only written for raw codes). false: not a valid code.
bool parse(const char* s, Parsed& p, uint16_t* raw);
// -> characters written (without the 0), 0 if it doesn't fit. Raw codes use the compact form
// when it is shorter (durations clustered within 12 % - each one stays within the decoders'
// and the TVs' tolerance).
size_t format(const Code& c, char* out, size_t max);
size_t formatRaw(const uint16_t* t, size_t n, uint16_t khz, char* out, size_t max);

}  // namespace ircodec
