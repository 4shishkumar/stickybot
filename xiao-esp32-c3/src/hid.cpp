#include "hid.h"
#include <HijelHID_BLEKeyboard.h>
#include "config.h"
#include "settings.h"

namespace hid {

static HijelHID_BLEKeyboard* kb = nullptr;
static uint8_t       slot_ = 1;
static String        name_;
static bool          off_ = false;
static volatile bool authEvent = false;

// Random static address per slot: chip MAC + slot, top two bits set
// (required for a random static address).
static void slotAddress(uint8_t n, uint8_t out[6]) {
  uint64_t mac = ESP.getEfuseMac();
  for (int i = 0; i < 6; i++) out[i] = (mac >> (8 * i)) & 0xFF;
  out[0] = out[0] + n * 0x31;
  out[5] |= 0xC0;
}

static void applyAddress() {
  uint8_t a[6];
  slotAddress(slot_, a);
  NimBLEDevice::stopAdvertising();
  NimBLEDevice::setOwnAddr(a);
  NimBLEDevice::setOwnAddrType(BLE_OWN_ADDR_RANDOM);
  if (!off_) NimBLEDevice::startAdvertising();
}

// The library splits a combo into (key, modifiers). For a modifier-only
// combo, send one modifier as the key itself (Option+Cmd -> key Cmd + Option).
static void split(const Combo& c, uint8_t& key, uint8_t& mods) {
  key = c.key;
  mods = c.mods;
  if (key) return;
  static const struct { uint8_t bit, key; } asKey[] = {
    {KEY_MOD_LGUI, KEY_LGUI}, {KEY_MOD_LALT, KEY_LALT},
    {KEY_MOD_LCTRL, KEY_LCTRL}, {KEY_MOD_LSHIFT, KEY_LSHIFT},
  };
  for (const auto& m : asKey) {
    if (mods & m.bit) {
      key = m.key;
      mods &= ~m.bit;
      return;
    }
  }
}

void begin(uint8_t slot) {
  slot_ = slot;
  name_ = settings::name();
  kb = new HijelHID_BLEKeyboard(name_.c_str(), "DIY", 100);
  // Start on a throwaway random address (never the chip's public one), then
  // switch to the slot's own address straight away.
  kb->setRandomAddress(true);
  kb->onPairingComplete([](bool ok) { if (ok) authEvent = true; });
  kb->begin();

  if (settings::needsBondReset()) {  // pairings from before slots existed
    NimBLEDevice::deleteAllBonds();
    for (uint8_t n = 1; n <= settings::SLOTS; n++) settings::clearPeer(n);
    settings::bondResetDone();
  }
  applyAddress();
}

void loop() {
  if (!authEvent) return;
  authEvent = false;
  NimBLEServer* server = NimBLEDevice::getServer();
  if (!server || server->getConnectedCount() == 0) return;
  NimBLEAddress id = server->getPeerInfo(0).getIdAddress();
  String addr = id.toString().c_str();
  String saved;
  uint8_t type;
  if (settings::peer(slot_, saved, type)) {
    if (saved == addr) return;
    // A different Mac took over this slot: drop the old Mac's pairing.
    NimBLEDevice::deleteBond(NimBLEAddress(std::string(saved.c_str()), type));
  }
  settings::savePeer(slot_, addr, id.getType());
  Serial.printf("Slot %u paired with %s\n", slot_, addr.c_str());
}

uint8_t slot() { return slot_; }
const String& name() { return name_; }

Bt state() {
  if (off_) return Bt::OFF;
  return kb->isPaired() ? Bt::CONNECTED : Bt::ADVERTISING;
}

bool connected() { return kb && kb->isPaired(); }

void down(const Combo& c) {
  uint8_t key, mods;
  split(c, key, mods);
  kb->press(key, mods);
}

void up() { kb->releaseAll(); }

void tap(const Combo& c) {
  uint8_t key, mods;
  split(c, key, mods);
  kb->tap(key, mods);
}

void tapEnter() { kb->tap(KEY_RETURN, 0); }

void setBattery(uint8_t percent) { kb->setBatteryLevel(percent); }

void disconnect() {
  if (off_) return;
  off_ = true;
  kb->end();
}

void reconnect() {
  if (!off_) return;
  off_ = false;
  kb->begin();
  applyAddress();
}

bool paired(uint8_t n) {
  String addr;
  uint8_t type;
  return settings::peer(n, addr, type);
}

void forget(uint8_t n) {
  String addr;
  uint8_t type;
  if (settings::peer(n, addr, type)) {
    NimBLEDevice::deleteBond(NimBLEAddress(std::string(addr.c_str()), type));
  }
  settings::clearPeer(n);
}

void forgetAll() {
  NimBLEDevice::deleteAllBonds();
  for (uint8_t n = 1; n <= settings::SLOTS; n++) settings::clearPeer(n);
}

}  // namespace hid
