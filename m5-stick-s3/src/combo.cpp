#include "combo.h"
#include <HijelHID_BLEKeyboard.h>

#define K(x) {#x, KEY_##x}

const KeyName KEYS[] = {
  {"(none)", 0},
  K(A), K(B), K(C), K(D), K(E), K(F), K(G), K(H), K(I), K(J), K(K), K(L), K(M),
  K(N), K(O), K(P), K(Q), K(R), K(S), K(T), K(U), K(V), K(W), K(X), K(Y), K(Z),
  K(1), K(2), K(3), K(4), K(5), K(6), K(7), K(8), K(9), K(0),
  {"Space", KEY_SPACE}, {"Return", KEY_RETURN}, {"Esc", KEY_ESCAPE}, {"Tab", KEY_TAB},
  {"Backspace", KEY_BACKSPACE}, {"Delete", KEY_DELETE},
  {"Up", KEY_UP}, {"Down", KEY_DOWN}, {"Left", KEY_LEFT}, {"Right", KEY_RIGHT},
  {"Home", KEY_HOME}, {"End", KEY_END}, {"PageUp", KEY_PAGE_UP}, {"PageDown", KEY_PAGE_DOWN},
  {"-", KEY_MINUS}, {"=", KEY_EQUAL}, {"[", KEY_LEFTBRACE}, {"]", KEY_RIGHTBRACE},
  {"\\", KEY_BACKSLASH}, {";", KEY_SEMICOLON}, {"'", KEY_APOSTROPHE}, {"`", KEY_GRAVE},
  {",", KEY_COMMA}, {".", KEY_DOT}, {"/", KEY_SLASH},
  K(F1), K(F2), K(F3), K(F4), K(F5), K(F6), K(F7), K(F8), K(F9), K(F10),
  K(F11), K(F12), K(F13), K(F14), K(F15), K(F16), K(F17), K(F18), K(F19), K(F20),
};
const size_t KEY_COUNT = sizeof(KEYS) / sizeof(KEYS[0]);

#undef K

const ModName MODS[4] = {
  {"Ctrl", KEY_MOD_LCTRL}, {"Option", KEY_MOD_LALT}, {"Shift", KEY_MOD_LSHIFT}, {"Cmd", KEY_MOD_LGUI},
};

int keyIndex(uint8_t code) {
  for (size_t i = 0; i < KEY_COUNT; i++)
    if (KEYS[i].code == code) return i;
  return -1;
}

String comboToString(const Combo& c) {
  String s;
  for (const auto& m : MODS) {
    if (!(c.mods & m.bit)) continue;
    if (s.length()) s += '+';
    s += m.name;
  }
  if (c.key) {
    int i = keyIndex(c.key);
    if (s.length()) s += '+';
    s += i >= 0 ? KEYS[i].name : "?";
  }
  return s.length() ? s : "(none)";
}

static uint8_t modFromToken(const String& t) {
  if (t == "cmd" || t == "command" || t == "gui" || t == "meta") return KEY_MOD_LGUI;
  if (t == "opt" || t == "option" || t == "alt")                  return KEY_MOD_LALT;
  if (t == "ctrl" || t == "control")                              return KEY_MOD_LCTRL;
  if (t == "shift")                                               return KEY_MOD_LSHIFT;
  return 0;
}

static int keyFromToken(const String& t) {
  if (t == "enter")  return KEY_RETURN;
  if (t == "escape") return KEY_ESCAPE;
  if (t == "del")    return KEY_DELETE;
  if (t == "pgup")   return KEY_PAGE_UP;
  if (t == "pgdn")   return KEY_PAGE_DOWN;
  for (size_t i = 1; i < KEY_COUNT; i++)
    if (t.equalsIgnoreCase(KEYS[i].name)) return KEYS[i].code;
  return -1;
}

bool parseCombo(const String& text, Combo& out) {
  Combo c;
  int start = 0;
  while (start <= (int)text.length()) {
    int plus = text.indexOf('+', start);
    if (plus < 0) plus = text.length();
    String t = text.substring(start, plus);
    t.trim();
    t.toLowerCase();
    if (t.isEmpty()) return false;

    if (uint8_t m = modFromToken(t)) {
      c.mods |= m;
    } else {
      int k = keyFromToken(t);
      if (k < 0 || c.key) return false;  // unknown key, or two main keys
      c.key = k;
    }
    start = plus + 1;
  }
  if (!c.valid()) return false;
  out = c;
  return true;
}
