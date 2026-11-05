// Firmware update over the Mac link (Bluetooth, no USB or Wi-Fi) — see
// PROTOCOL.md, "Firmware update over Bluetooth". The agent sends the app image
// to the OTA characteristic (6e400006, encrypted link only); this writes it to
// the inactive OTA slot (esp_ota_*), checks its SHA-256 and reboots into it.
// The state machine is in logic/ota_rx.cpp (unit-tested).
//
// Rollback: the bootloader has CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE, so a new
// image boots as "pending verify". We confirm it only after it has run for a
// while (display up, Bluetooth on); any reset before that — a crash, the loop
// watchdog, a power cut — boots the previous firmware again.
#pragma once
#include <Arduino.h>
#include <ArduinoJson.h>

namespace ota {

void bootCheck();                                // in setup(): a new image waiting to be confirmed?
void onPacket(const uint8_t* data, size_t len);  // from the BLE task: [offset u32][bytes]
void control(JsonObjectConst msg);               // {"begin":{...}} | {"end":1} | {"cancel":1} | {"status":1}
void loop();                                     // drain packets, acks, the screen, confirm the image
bool active();                                   // an update is running (the update screen owns the stick)
void cancelByUser();                             // B on the update screen
bool takeNotice(String& title);                  // the update just ended without a restart: "Update cancelled"...
bool pendingVerify();                            // running image not confirmed yet (no standby, no new update)
// main.cpp says why an update can't start now ("dictation", "talk", ...; nullptr = free)
void setBusyCheck(const char* (*why)());

}  // namespace ota
