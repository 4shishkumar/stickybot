#include "combo_logic.h"
#include <cctype>

const KeyName KEYS[] = {
  {"(none)", 0},  // none
  {"A", 0x04},  // KEY_A
  {"B", 0x05},  // KEY_B
  {"C", 0x06},  // KEY_C
  {"D", 0x07},  // KEY_D
  {"E", 0x08},  // KEY_E
  {"F", 0x09},  // KEY_F
  {"G", 0x0A},  // KEY_G
  {"H", 0x0B},  // KEY_H
  {"I", 0x0C},  // KEY_I
  {"J", 0x0D},  // KEY_J
  {"K", 0x0E},  // KEY_K
  {"L", 0x0F},  // KEY_L
  {"M", 0x10},  // KEY_M
  {"N", 0x11},  // KEY_N
  {"O", 0x12},  // KEY_O
  {"P", 0x13},  // KEY_P
  {"Q", 0x14},  // KEY_Q
  {"R", 0x15},  // KEY_R
  {"S", 0x16},  // KEY_S
  {"T", 0x17},  // KEY_T
  {"U", 0x18},  // KEY_U
  {"V", 0x19},  // KEY_V
  {"W", 0x1A},  // KEY_W
  {"X", 0x1B},  // KEY_X
  {"Y", 0x1C},  // KEY_Y
  {"Z", 0x1D},  // KEY_Z
  {"1", 0x1E},  // KEY_1
  {"2", 0x1F},  // KEY_2
  {"3", 0x20},  // KEY_3
  {"4", 0x21},  // KEY_4
  {"5", 0x22},  // KEY_5
  {"6", 0x23},  // KEY_6
  {"7", 0x24},  // KEY_7
  {"8", 0x25},  // KEY_8
  {"9", 0x26},  // KEY_9
  {"0", 0x27},  // KEY_0
  {"Space", 0x2C},  // KEY_SPACE
  {"Return", 0x28},  // KEY_RETURN
  {"Esc", 0x29},  // KEY_ESCAPE
  {"Tab", 0x2B},  // KEY_TAB
  {"Backspace", 0x2A},  // KEY_BACKSPACE
  {"Delete", 0x4C},  // KEY_DELETE
  {"Up", 0x52},  // KEY_UP
  {"Down", 0x51},  // KEY_DOWN
  {"Left", 0x50},  // KEY_LEFT
  {"Right", 0x4F},  // KEY_RIGHT
  {"Home", 0x4A},  // KEY_HOME
  {"End", 0x4D},  // KEY_END
  {"PageUp", 0x4B},  // KEY_PAGE_UP
  {"PageDown", 0x4E},  // KEY_PAGE_DOWN
  {"-", 0x2D},  // KEY_MINUS
  {"=", 0x2E},  // KEY_EQUAL
  {"[", 0x2F},  // KEY_LEFTBRACE
  {"]", 0x30},  // KEY_RIGHTBRACE
  {"\\", 0x31},  // KEY_BACKSLASH
  {";", 0x33},  // KEY_SEMICOLON
  {"'", 0x34},  // KEY_APOSTROPHE
  {"`", 0x35},  // KEY_GRAVE
  {",", 0x36},  // KEY_COMMA
  {".", 0x37},  // KEY_DOT
  {"/", 0x38},  // KEY_SLASH
  {"F1", 0x3A},  // KEY_F1
  {"F2", 0x3B},  // KEY_F2
  {"F3", 0x3C},  // KEY_F3
  {"F4", 0x3D},  // KEY_F4
  {"F5", 0x3E},  // KEY_F5
  {"F6", 0x3F},  // KEY_F6
  {"F7", 0x40},  // KEY_F7
  {"F8", 0x41},  // KEY_F8
  {"F9", 0x42},  // KEY_F9
  {"F10", 0x43},  // KEY_F10
  {"F11", 0x44},  // KEY_F11
  {"F12", 0x45},  // KEY_F12
  {"F13", 0x68},  // KEY_F13
  {"F14", 0x69},  // KEY_F14
  {"F15", 0x6A},  // KEY_F15
  {"F16", 0x6B},  // KEY_F16
  {"F17", 0x6C},  // KEY_F17
  {"F18", 0x6D},  // KEY_F18
  {"F19", 0x6E},  // KEY_F19
  {"F20", 0x6F},  // KEY_F20
};
const size_t KEY_COUNT = sizeof(KEYS) / sizeof(KEYS[0]);

const ModName MODS[4] = {
  {"Ctrl", combo::MOD_CTRL}, {"Option", combo::MOD_ALT}, {"Shift", combo::MOD_SHIFT}, {"Cmd", combo::MOD_GUI},
};

int keyIndex(uint8_t code) {
  for (size_t i = 0; i < KEY_COUNT; i++)
    if (KEYS[i].code == code) return (int)i;
  return -1;
}

namespace combo {

static std::string lower(std::string s) {
  for (auto& c : s) c = (char)std::tolower((unsigned char)c);
  return s;
}

static std::string trim(const std::string& s) {
  size_t a = s.find_first_not_of(" \t\r\n"), b = s.find_last_not_of(" \t\r\n");
  return a == std::string::npos ? "" : s.substr(a, b - a + 1);
}

std::string toString(const Combo& c) {
  std::string s;
  for (const auto& m : MODS) {
    if (!(c.mods & m.bit)) continue;
    if (!s.empty()) s += '+';
    s += m.name;
  }
  if (c.key) {
    int i = keyIndex(c.key);
    if (!s.empty()) s += '+';
    s += i >= 0 ? KEYS[i].name : "?";
  }
  return s.empty() ? "(none)" : s;
}

static uint8_t modFromToken(const std::string& t) {
  if (t == "cmd" || t == "command" || t == "gui" || t == "meta") return MOD_GUI;
  if (t == "opt" || t == "option" || t == "alt")                  return MOD_ALT;
  if (t == "ctrl" || t == "control")                              return MOD_CTRL;
  if (t == "shift")                                               return MOD_SHIFT;
  return 0;
}

static int keyFromToken(const std::string& t) {
  if (t == "enter")  return HID_RETURN;
  if (t == "escape") return HID_ESCAPE;
  if (t == "del")    return HID_DELETE;
  if (t == "pgup")   return HID_PAGE_UP;
  if (t == "pgdn")   return HID_PAGE_DOWN;
  for (size_t i = 1; i < KEY_COUNT; i++)
    if (t == lower(KEYS[i].name)) return KEYS[i].code;
  return -1;
}

bool parse(const std::string& text, Combo& out) {
  Combo c;
  size_t start = 0;
  while (start <= text.size()) {
    size_t plus = text.find('+', start);
    if (plus == std::string::npos) plus = text.size();
    std::string t = lower(trim(text.substr(start, plus - start)));
    if (t.empty()) return false;
    if (uint8_t m = modFromToken(t)) {
      c.mods |= m;
    } else {
      int k = keyFromToken(t);
      if (k < 0 || c.key) return false;  // unknown key, or two main keys
      c.key = (uint8_t)k;
    }
    start = plus + 1;
  }
  if (!c.valid()) return false;
  out = c;
  return true;
}

}  // namespace combo
