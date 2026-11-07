// Face packages on the stick's flash (docs/faces/FORMAT.md section 17): LittleFS on the 1.5 MB
// data partition the firmware didn't use before ("spiffs" in default_8MB.csv, the Arduino
// LittleFS default label), mounted at /fs and formatted the first time.
//
//   /fs/faces/<id>.sfb        an installed face (the list of faces is this folder)
//   /fs/faces/.part           a face arriving over Bluetooth (install.cpp)
//   /fs/faces/.part.json      what it is (id, size, sha256, version), so a restart can resume it
//
// At most MAX_FACES faces of at most sfb::MAX_SIZE bytes each. No partition change: the
// firmware update over Bluetooth never touches the partition table.
#pragma once
#include <Arduino.h>
#include "../../logic/sfb.h"

namespace faces {
namespace pkg {
namespace store {

constexpr int      MAX_FACES = 12;
constexpr uint32_t MAX_FILE = sfb::MAX_SIZE;
constexpr uint32_t SPARE = 16 * 1024;   // left free for LittleFS itself (its metadata, wear levelling)

bool     begin();                 // mounts it (formats it if it isn't LittleFS yet) and makes /faces
bool     mounted();
int      list(char ids[][13], int max);       // the installed faces' ids, sorted
uint32_t fileSize(const char* id);            // 0 if it isn't there
uint8_t* read(const char* id, size_t& n);     // the whole file (PSRAM; free() it), nullptr if missing / too big
bool     sha256(const char* id, uint8_t out[32]);
bool     remove(const char* id);
int      wipe();                              // every face and a half-received one; how many faces
uint32_t freeBytes();                         // room for faces (less SPARE)

// The part file (install.cpp)
struct Part {
  char     id[13] = {0}, sha[65] = {0}, version[16] = {0};
  uint32_t size = 0;
};
bool     partFind(Part& p, uint32_t& have);   // what .part.json says and the bytes .part already has
bool     partOpen(const Part& p, bool resume); // resume: append to .part; else start it again
bool     partWrite(const uint8_t* b, size_t n);
bool     partSha256(uint32_t size, uint8_t out[32]);  // of what's in flash (read back)
uint8_t* partRead(size_t& n);                 // the whole part file (PSRAM; free() it)
bool     partCommit(const char* id);          // .part -> <id>.sfb (replaces the old one at once)
void     partDrop();
uint32_t partBytes();                         // its size now (0 none)

}  // namespace store
}  // namespace pkg
}  // namespace faces
