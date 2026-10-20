// Key combos: representation, names, text parsing (Arduino String wrappers
// around logic/combo_logic.h, which the native unit tests cover).
#pragma once
#include <Arduino.h>
#include "logic/combo_logic.h"

// "Option+Cmd", "Ctrl+Space", "(none)"
String comboToString(const Combo& c);

// Accepts e.g. "cmd+opt", "ctrl + space", "Cmd+Shift+4" (case-insensitive).
bool parseCombo(const String& text, Combo& out);
