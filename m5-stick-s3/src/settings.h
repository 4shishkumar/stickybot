// Persistent settings in NVS flash. Survives power-off and re-flashing
// (`pio run -t upload`); wiped by `pio run -t erase`.
#pragma once
#include "combo.h"

namespace settings {

constexpr uint8_t SLOTS = 3;

void   begin();

Combo  combo();
void   saveCombo(const Combo& c);

String wifiSsid();
String wifiPass();
void   saveWifi(const String& ssid, const String& pass);

String name();                        // Bluetooth name
bool   saveName(const String& n);     // 1-20 printable characters

String password();                    // hotspot + OTA
bool   savePassword(const String& p); // 8-63 chars

// After a hold (e.g. dictation) is released, tap Enter after a delay.
// 0 = off. Short taps never trigger it.
uint16_t enterDelayMs();
void     saveEnterDelayMs(uint16_t ms);

uint8_t slot();                       // active Mac slot, 1..SLOTS
void    saveSlot(uint8_t n);

// The Mac bonded to a slot (its Bluetooth identity address).
bool   peer(uint8_t n, String& addr, uint8_t& type);
void   savePeer(uint8_t n, const String& addr, uint8_t type);
void   clearPeer(uint8_t n);

bool   flipped();                     // screen rotated 180°
void   saveFlipped(bool f);
uint8_t standbyMin();                 // minutes idle on battery before standby, 0 = never
void    saveStandbyMin(uint8_t m);

bool    soundOn();                    // sound cues on the speaker
void    saveSoundOn(bool on);
uint8_t soundVolume();                // 0-255
void    saveSoundVolume(uint8_t v);

uint8_t brightnessPct();              // screen brightness 25/50/75/100
void    saveBrightnessPct(uint8_t p);

// Motion gestures (each can be turned off)
bool   liftWakes();      // pick up -> wake screen
bool   faceDownSleeps(); // face-down -> standby
bool   shakeEscapes();   // shake -> Escape / deny Claude prompt
void   saveMotion(bool lift, bool faceDown, bool shake);

bool   autoRotate();     // follow the motion sensor instead of the manual flip
void   saveAutoRotate(bool on);
bool   landscape();      // with auto-rotate: the reading screens turn landscape when held sideways
void   saveLandscape(bool on);

String face();                    // the Home screen's face id (faces/faces.h), default "classic"
void   saveFace(const String& id);

// Mac password for "unlock" (double-press B). Write-only from outside.
String macPassword();
void   saveMacPassword(const String& p);  // empty = clear

// Developer mode: on USB power, Wi-Fi stays on (home network) for OTA + logs.
bool   devMode();
void   saveDevMode(bool on);

// Wireless Wispr mic: an A hold away from USB streams the mic to the Mac agent
// (only when the agent has it turned on too).
bool   wirelessMic();
void   saveWirelessMic(bool on);

bool   needsBondReset();             // first boot after upgrading to slots
void   bondResetDone();

}  // namespace settings
