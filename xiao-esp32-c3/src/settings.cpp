#include "settings.h"
#include <Preferences.h>
#include <HijelHID_BLEKeyboard.h>
#include "config.h"

namespace settings {

static Preferences prefs;
static Combo current;

static String getStr(const char* key, const char* def = "") {
  return prefs.isKey(key) ? prefs.getString(key) : String(def);
}

void begin() {
  prefs.begin("macro", false);
  current.mods = prefs.getUChar("mods", KEY_MOD_LALT | KEY_MOD_LGUI);  // default: Option+Cmd
  current.key  = prefs.getUChar("key", 0);
  if (!current.valid()) current.mods = KEY_MOD_LALT | KEY_MOD_LGUI;
}

Combo combo() { return current; }

void saveCombo(const Combo& c) {
  if (!c.valid()) return;
  current = c;
  prefs.putUChar("mods", c.mods);
  prefs.putUChar("key", c.key);
}

String wifiSsid() { return getStr("ssid"); }
String wifiPass() { return getStr("pass"); }

void saveWifi(const String& ssid, const String& pass) {
  prefs.putString("ssid", ssid);
  prefs.putString("pass", pass);
}

String name() { return getStr("name", DEFAULT_NAME); }

bool saveName(const String& n) {
  if (n.isEmpty() || n.length() > 20) return false;
  for (char c : n)
    if (c < 0x20 || c > 0x7E) return false;
  prefs.putString("name", n);
  return true;
}

String password() { return getStr("password", DEFAULT_PASSWORD); }

bool savePassword(const String& p) {
  if (p.length() < 8 || p.length() > 63) return false;
  prefs.putString("password", p);
  return true;
}

uint16_t enterDelayMs() { return prefs.getUShort("enter", 0); }
void saveEnterDelayMs(uint16_t ms) { prefs.putUShort("enter", ms); }

uint8_t slot() {
  uint8_t n = prefs.getUChar("slot", 1);
  return (n >= 1 && n <= SLOTS) ? n : 1;
}

void saveSlot(uint8_t n) {
  if (n >= 1 && n <= SLOTS) prefs.putUChar("slot", n);
}

static String peerKey(uint8_t n, const char* suffix) { return String("peer") + n + suffix; }

bool peer(uint8_t n, String& addr, uint8_t& type) {
  addr = getStr(peerKey(n, "a").c_str());
  type = prefs.getUChar(peerKey(n, "t").c_str(), 0);
  return addr.length() > 0;
}

void savePeer(uint8_t n, const String& addr, uint8_t type) {
  prefs.putString(peerKey(n, "a").c_str(), addr);
  prefs.putUChar(peerKey(n, "t").c_str(), type);
}

void clearPeer(uint8_t n) {
  prefs.remove(peerKey(n, "a").c_str());
  prefs.remove(peerKey(n, "t").c_str());
}

bool needsBondReset() { return !prefs.getBool("slots_v1", false); }
void bondResetDone()  { prefs.putBool("slots_v1", true); }

}  // namespace settings
