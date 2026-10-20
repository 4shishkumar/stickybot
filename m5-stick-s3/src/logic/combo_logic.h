// Key combos: the key table, parsing ("cmd+opt", "Ctrl + Space") and display
// ("Option+Cmd"). Pure C++ (native-testable); combo.h wraps it for Arduino String.
// Codes are USB HID usage IDs (same as HijelHID's KEY_* — checked in combo.cpp).
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <string>

// Modifier bits plus an optional main key; key == 0 is a modifier-only combo.
struct Combo {
  uint8_t mods = 0;
  uint8_t key  = 0;
  bool valid() const { return mods || key; }
};

struct KeyName { const char* name; uint8_t code; };
extern const KeyName KEYS[];   // KEYS[0] is "(none)", code 0
extern const size_t  KEY_COUNT;

struct ModName { const char* name; uint8_t bit; };
extern const ModName MODS[4];  // Mac order: Ctrl, Option, Shift, Cmd

namespace combo {
constexpr uint8_t MOD_CTRL = 0x01, MOD_SHIFT = 0x02, MOD_ALT = 0x04, MOD_GUI = 0x08;  // left-hand modifiers
constexpr uint8_t HID_RETURN = 0x28, HID_ESCAPE = 0x29, HID_DELETE = 0x4C, HID_PAGE_UP = 0x4B, HID_PAGE_DOWN = 0x4E;

std::string toString(const Combo& c);                    // "(none)" when empty
bool        parse(const std::string& text, Combo& out);  // case-insensitive; false if invalid
}  // namespace combo

int keyIndex(uint8_t code);  // index into KEYS, -1 if unknown
