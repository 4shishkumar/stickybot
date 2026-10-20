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

bool   needsBondReset();              // first boot after upgrading to slots
void   bondResetDone();

}  // namespace settings
