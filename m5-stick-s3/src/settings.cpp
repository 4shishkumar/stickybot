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
  prefs.begin("stick", false);  // same namespace as M1, keeps the saved combo
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

bool flipped()            { return prefs.getBool("flip", false); }
void saveFlipped(bool f)   { prefs.putBool("flip", f); }
uint8_t standbyMin()       { return prefs.getUChar("standby", 10); }
void saveStandbyMin(uint8_t m) { prefs.putUChar("standby", m); }

bool soundOn()                { return prefs.getBool("snd", true); }
void saveSoundOn(bool on)      { prefs.putBool("snd", on); }
uint8_t soundVolume()          { return prefs.getUChar("sndvol", 140); }
void saveSoundVolume(uint8_t v) { prefs.putUChar("sndvol", v); }

uint8_t brightnessPct() {
  uint8_t p = prefs.getUChar("bright", 50);
  return (p >= 10 && p <= 100) ? p : 50;
}
void saveBrightnessPct(uint8_t p) { prefs.putUChar("bright", p); }


bool liftWakes()      { return prefs.getBool("m_lift", true); }
bool faceDownSleeps() { return prefs.getBool("m_face", true); }
bool shakeEscapes()   { return prefs.getBool("m_shake", true); }
void saveMotion(bool lift, bool faceDown, bool shake) {
  prefs.putBool("m_lift", lift);
  prefs.putBool("m_face", faceDown);
  prefs.putBool("m_shake", shake);
}

bool autoRotate()           { return prefs.getBool("autorot", true); }
void saveAutoRotate(bool on) { prefs.putBool("autorot", on); }

String macPassword() { return getStr("macpass"); }
void saveMacPassword(const String& p) {
  if (p.isEmpty()) prefs.remove("macpass");
  else prefs.putString("macpass", p);
}

bool devMode()           { return prefs.getBool("devmode", true); }
void saveDevMode(bool on)  { prefs.putBool("devmode", on); }

bool wirelessMic()            { return prefs.getBool("wmic", true); }
void saveWirelessMic(bool on)  { prefs.putBool("wmic", on); }

bool needsBondReset() { return !prefs.getBool("slots_v1", false); }
void bondResetDone()  { prefs.putBool("slots_v1", true); }

}  // namespace settings
