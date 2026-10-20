// Bluetooth keyboard with 3 Mac slots. Each slot has its own Bluetooth
// identity, so each Mac only knows "its" slot.
#pragma once
#include "combo.h"

namespace hid {

enum class Bt { CONNECTED, ADVERTISING, OFF };

void   begin(uint8_t slot);   // slot 1..3; Bluetooth name from settings (same for every slot)
void   loop();                // records which Mac paired with the slot
uint8_t slot();
const String& name();
Bt     state();
bool   connected();           // paired + encrypted, ready to send

void down(const Combo& c);    // hold combo
void up();                    // release everything
void tap(const Combo& c);
void tapEnter();
void setBattery(uint8_t percent);

void disconnect();            // stay paired, stop Bluetooth until reconnect()
void reconnect();
bool paired(uint8_t slot);
void forget(uint8_t slot);    // delete that slot's pairing
void forgetAll();

}  // namespace hid
