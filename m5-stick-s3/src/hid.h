// Bluetooth keyboard with 3 Mac slots. Each slot has its own Bluetooth
// identity, so each Mac only knows "its" slot.
#pragma once
#include "combo.h"

namespace hid {

enum class Bt { CONNECTED, ADVERTISING, OFF };

void   begin(uint8_t slot);   // slot 1..3; Bluetooth name from settings (same for every slot)
void   loop();                // records pairings, times out unanswered codes
uint8_t slot();
const String& name();
Bt     state();
bool   connected();           // paired + encrypted, ready to send

void down(const Combo& c);    // hold combo
void up();                    // release everything
void tap(const Combo& c);
void tapEnter();
void tapEscape();
void typeText(const String& s);  // US layout, ASCII
void setBattery(uint8_t percent);

// Fn / Globe: macOS only accepts it from an Apple keyboard, so while the saved
// combo has Fn the stick identifies as one (chosen at boot). needsRestart():
// saving this combo changes that, so restart (and re-pair on the Mac).
bool appleId();
bool needsRestart(const Combo& c);

void disconnect();            // stay paired, stop Bluetooth until reconnect()
void reconnect();
bool paired(uint8_t slot);
void forget(uint8_t slot);    // delete that slot's pairing
void forgetAll();

// Pairing: the Mac and the stick show the same 6-digit code. The pairing only
// completes when the user accepts on the stick (button A), so nobody can pair
// a laptop without physical access to the stick.
uint32_t pairingCode();       // code waiting for confirmation, 0 = none
void     confirmPairing(bool accept);

}  // namespace hid
