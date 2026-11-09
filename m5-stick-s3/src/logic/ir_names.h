// The IR remote's names and limits (firmware 2.5.0, src/ir/): device ids, button names, the
// "button N" default, and the "@ir send dev=tv btn=volume up" command line. Pure C++.
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace irnames {

constexpr int    MAX_DEVICES = 16;
constexpr int    MAX_BUTTONS = 48;          // per device
constexpr size_t DEVICE_LEN = 16;           // "living-room-tv"
constexpr size_t BUTTON_LEN = 24;           // "cool 24 fan auto"
constexpr size_t MAX_FILE = 48 * 1024;      // one device's JSON file

// "Living Room TV" -> "living-room-tv": lower case, spaces and _ become -, only a-z 0-9 -,
// 1..16 characters starting with a letter or digit. false if nothing usable is left.
bool deviceId(const char* in, char out[DEVICE_LEN + 1]);
// "  Volume  Up " -> "volume up": lower case, single spaces, a-z 0-9 space + - _ . # only,
// 1..24 characters.
bool buttonName(const char* in, char out[BUTTON_LEN + 1]);
// The lowest n >= 1 for which "<prefix> n" isn't in names (prefix "button" -> "button 3")
int nextNumber(const char* const* names, int count, const char* prefix);

// "send dev=tv btn=volume up repeat=3" -> "op=send&dev=tv&btn=volume%20up&repeat=3": the first
// word without = is the op; words without = after a value continue that value (spaces kept);
// "&" separated input is passed through. Returns the length, 0 if it doesn't fit.
size_t normalizeQuery(const char* in, char* out, size_t max);

}  // namespace irnames
