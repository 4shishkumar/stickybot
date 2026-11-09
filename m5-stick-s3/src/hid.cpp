#include "hid.h"
#include "netlog.h"
#include <HijelHID_BLEKeyboard.h>
#include "config.h"
#include "settings.h"
#include "buddy.h"
#include <Preferences.h>

namespace hid {

static HijelHID_BLEKeyboard* kb = nullptr;
static uint8_t       slot_ = 1;
static String        name_;
static bool          off_ = false;
static volatile bool authEvent = false;
static bool          appleId_ = false;  // identifying as an Apple keyboard (for fn)

constexpr uint16_t APPLE_VID = 0x05AC, APPLE_PID = 0x0267;  // Apple, Magic Keyboard

constexpr uint32_t CONFIRM_TIMEOUT_MS = 30000;
constexpr uint32_t TAP_FN_MS = 30;  // a Fn-only tap: how long it's held
// The HID report map's version: raised whenever it changes (2 = 2.5.0, the mouse). On the
// first boot with a new one the stick tells bonded Macs to read its services again
// (Service Changed), see begin().
constexpr uint8_t HID_MAP_VERSION = 2;
static bool          mapChanged_ = false;
static volatile bool mouseSubscribed = false;

class MouseSubscribe : public NimBLECharacteristicCallbacks {
  void onSubscribe(NimBLECharacteristic*, NimBLEConnInfo&, uint16_t sub) override { mouseSubscribed = sub != 0; }
};

static NimBLEConnInfo*   pendingConn = nullptr;
static volatile uint32_t pendingCode = 0;
static uint32_t          pendingSince = 0;

// The library auto-accepts the numeric-comparison code; hold it until the
// user presses A (accept) or B (reject) on the stick instead.
class ConfirmCallbacks : public HijelHID_Internal::KBServerCallbacks {
 public:
  using KBServerCallbacks::KBServerCallbacks;
  void onConfirmPassKey(NimBLEConnInfo& info, uint32_t code) override {
    delete pendingConn;
    pendingConn = new NimBLEConnInfo(info);
    pendingSince = millis();
    pendingCode = code;
  }
  void onDisconnect(NimBLEServer* s, NimBLEConnInfo& info, int reason) override {
    pendingCode = 0;
    mouseSubscribed = false;
    KBServerCallbacks::onDisconnect(s, info, reason);
  }
};

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
// Fn isn't an HID modifier: it goes in its own report bit (setFn), so it's
// left out here; a Fn-only combo has key 0 and nothing else to press.
static void split(const Combo& c, uint8_t& key, uint8_t& mods) {
  key = c.key;
  mods = c.mods & ~combo::MOD_FN;
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

bool appleId() { return appleId_; }

bool needsRestart(const Combo& c) { return ((c.mods & combo::MOD_FN) != 0) != appleId_; }

void begin(uint8_t slot) {
  slot_ = slot;
  name_ = settings::name();
  kb = new HijelHID_BLEKeyboard(name_.c_str(), "DIY", 100);
  // macOS only takes the fn / Globe key from an Apple keyboard, so a combo with
  // Fn makes the stick say it's one (Magic Keyboard IDs, USB-IF vendor source).
  // The Mac caches this per pairing: re-pair after it changes.
  appleId_ = settings::combo().mods & combo::MOD_FN;
  if (appleId_) kb->setPnpId(0x02, APPLE_VID, APPLE_PID, 0x0100);
  // Start on a throwaway random address (never the chip's public one), then
  // switch to the slot's own address straight away.
  kb->setRandomAddress(true);
  kb->setSecurityMode(HIDSecurity::Passkey);
  kb->onPairingComplete([](bool ok) { if (ok) authEvent = true; });
  kb->begin();
  static ConfirmCallbacks confirm(kb);
  NimBLEDevice::getServer()->setCallbacks(&confirm, false);

  if (settings::needsBondReset()) {  // pairings from before slots existed
    NimBLEDevice::deleteAllBonds();
    for (uint8_t n = 1; n <= settings::SLOTS; n++) settings::clearPeer(n);
    settings::bondResetDone();
  }
  buddy::attach();  // link for the Mac agent + Claude Code hook
  static MouseSubscribe mouseSub;
  if (kb->mouseInput()) kb->mouseInput()->setCallbacks(&mouseSub);
  // A new report map (2.5.0 added the mouse): every boot already re-registers the services
  // after buddy::attach(), which queues a Service Changed indication for each bonded Mac
  // (sent when it reconnects); say it once more on purpose the first time, so a Mac
  // re-reads the services and the new map instead of using its cached copy.
  {
    Preferences p;
    if (p.begin("hid", false)) {
      if (p.getUChar("map", 0) != HID_MAP_VERSION) {
        mapChanged_ = true;
        NimBLEDevice::getServer()->sendServiceChangedIndication();
        p.putUChar("map", HID_MAP_VERSION);
        Log.printf("HID report map v%u (mouse added): Service Changed queued for the paired Macs\n", HID_MAP_VERSION);
      }
      p.end();
    }
  }
  applyAddress();
}

uint32_t pairingCode() { return pendingCode; }

void confirmPairing(bool accept) {
  if (!pendingCode || !pendingConn) return;
  NimBLEDevice::injectConfirmPasskey(*pendingConn, accept);
  Log.printf("Pairing %s\n", accept ? "accepted" : "rejected");
  pendingCode = 0;
}

void loop() {
  if (pendingCode && millis() - pendingSince > CONFIRM_TIMEOUT_MS) confirmPairing(false);
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
  Log.printf("Slot %u paired with %s\n", slot_, addr.c_str());
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
  if (c.mods & combo::MOD_FN) kb->setFn(true);
  if (key) kb->press(key, mods);
}

void up() { kb->releaseAll(); }

void tap(const Combo& c) {
  uint8_t key, mods;
  split(c, key, mods);
  if (!(c.mods & combo::MOD_FN)) return kb->tap(key, mods);
  kb->setFn(true);
  if (key) kb->tap(key, mods);
  else delay(TAP_FN_MS);
  kb->setFn(false);
}

void tapEnter() { kb->tap(KEY_RETURN, 0); }
void tapEscape() { kb->tap(KEY_ESCAPE, 0); }
void typeText(const String& s) { kb->print(s.c_str()); }

void setBattery(uint8_t percent) { kb->setBatteryLevel(percent); }

static int8_t clamp8(int v) { return v > 127 ? 127 : v < -127 ? -127 : v; }

bool mouseMove(int dx, int dy, int wheel, int pan) {
  if (!kb || !kb->isPaired()) return false;
  return kb->mouseMove(clamp8(dx), clamp8(dy), clamp8(wheel), clamp8(pan));
}

void mouseButtons(uint8_t mask) {
  if (kb && kb->isPaired()) kb->mouseButtons(mask);
}

void media(uint16_t usage) {
  if (kb && kb->isPaired()) kb->tap(usage);
}

bool mouseKnown() { return mouseSubscribed; }
bool mapChanged() { return mapChanged_; }

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
