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

// Mouse + media keys (firmware 2.5.0: the air mouse, presenter and media remote). The
// stick is a keyboard + consumer control + mouse; the mouse part was added in 2.5.0.
bool mouseMove(int dx, int dy, int wheel = 0, int pan = 0);  // false: not sent (busy / not connected)
void mouseButtons(uint8_t mask);  // held buttons: 1 left, 2 right, 4 middle (0 = none)
void media(uint16_t usage);       // tap a consumer key (MEDIA_* in BLEHIDMediaKeys.h)
// The Mac subscribed to the mouse report this connection: it knows the stick is a mouse
// too. Still false after the Mac connected = it kept the old (keyboard-only) report map
// from before 2.5.0: forget the stick on the Mac and pair again (once).
bool mouseKnown();
bool mapChanged();                // the report map changed at this boot (Service Changed sent)

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
