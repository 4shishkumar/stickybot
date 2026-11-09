// The learned remotes in the stick's flash: /ir/<device>.json in the LittleFS partition the face
// packages use (mounted by faces::pkg::store), one file per device (logic/ir_doc.h). At most
// irnames::MAX_DEVICES devices of irnames::MAX_BUTTONS buttons; a save needs room to spare.
#pragma once
#include <Arduino.h>
#include "../logic/ir_doc.h"

namespace ir {
namespace store {

bool begin();
int  list(char ids[][irnames::DEVICE_LEN + 1], int max);    // sorted
bool exists(const char* id);
bool load(const char* id, irdoc::Device& d);
bool save(const irdoc::Device& d, const char*& why);        // why: "full" (no room / too many), "write"
bool remove(const char* id);
bool rename(const char* from, const char* to, const char*& why);
uint32_t freeKB();

}  // namespace store
}  // namespace ir
