// Key combos: representation, names, text parsing.
#pragma once
#include <Arduino.h>

// Modifier bits (KEY_MOD_*) plus an optional main key (HID usage code).
// key == 0 means a modifier-only combo, e.g. Option+Cmd.
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

int    keyIndex(uint8_t code);  // index into KEYS, -1 if unknown

// "Option+Cmd", "Ctrl+Space", "(none)"
String comboToString(const Combo& c);

// Accepts e.g. "cmd+opt", "ctrl + space", "Cmd+Shift+4" (case-insensitive).
bool parseCombo(const String& text, Combo& out);
