// Installing a face package over the Mac link (docs/faces/FORMAT.md section 18, PROTOCOL.md
// "Face packages over Bluetooth"): the firmware update's receiver (logic/ota_rx.cpp) with a
// file sink instead of the flash one, on the same data characteristic (…0006…). One bulk
// transfer at a time: while a face arrives it owns …0006… (a firmware update is refused
// "busy": "faces"), and a face is refused "ota" while a firmware update runs.
//
// The agent sends {"src":"agent","faces":{"put":{"id","version","size","sha256"}}}, the bytes,
// then {"faces":{"end":1}}; the stick writes /fs/faces/.part, reads it back for the sha256,
// loads it as the engine would (anything invalid: refused, never installed), and renames it to
// <id>.sfb. Also {"faces":{"cancel":1}} and {"faces":{"rm":"<id>"|"all"}}. Answers are
// {"src":"agent","evt":"faces","state":…}.
#pragma once
#include <Arduino.h>
#include <ArduinoJson.h>

namespace faces {
namespace pkg {
namespace install {

void onPacket(const uint8_t* data, size_t len);  // from the BLE task: [offset u32][bytes]
bool receiving();                                // a face is arriving (it owns …0006…)
void control(JsonObjectConst msg);               // {"put":{…}} | {"end":1} | {"cancel":1} | {"rm":…}
void loop();                                     // drain packets, acks, the idle timeout
void cancelByUser();                             // B while the toast shows
// main.cpp says why a transfer can't start now ("dictation", "menu", ...; nullptr = free)
void setBusyCheck(const char* (*why)());
// What the toast over Home says now ("Installing Cat Nap  60 %", "Installed Cat Nap", ...);
// false when there's nothing to show.
bool toast(String& line, uint16_t& bg);

}  // namespace install
}  // namespace pkg
}  // namespace faces
