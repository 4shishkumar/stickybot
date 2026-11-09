#include "store.h"
#include <LittleFS.h>
#include "../faces/pkg/store.h"
#include "../netlog.h"

namespace ir {
namespace store {

static const char* DIR_ = "/ir";
constexpr uint32_t SPARE = 32 * 1024;  // left free (LittleFS itself, the faces)

static String path(const char* id) { return String(DIR_) + "/" + id + ".json"; }

static bool idOk(const char* id) {
  char clean[irnames::DEVICE_LEN + 1];
  return id && irnames::deviceId(id, clean) && !strcmp(clean, id);
}

bool begin() {
  if (!faces::pkg::store::begin()) return false;  // mounts LittleFS (idempotent)
  if (!LittleFS.exists(DIR_)) LittleFS.mkdir(DIR_);
  return true;
}

int list(char ids[][irnames::DEVICE_LEN + 1], int max) {
  if (!faces::pkg::store::mounted()) return 0;
  File dir = LittleFS.open(DIR_);
  if (!dir || !dir.isDirectory()) return 0;
  int n = 0;
  for (File f = dir.openNextFile(); f && n < max; f = dir.openNextFile()) {
    const char* name = f.name();
    const size_t len = strlen(name);
    if (f.isDirectory() || len < 6 || len > irnames::DEVICE_LEN + 5 || strcmp(name + len - 5, ".json")) continue;
    char id[irnames::DEVICE_LEN + 1];
    memcpy(id, name, len - 5);
    id[len - 5] = 0;
    if (!idOk(id)) continue;
    int at = n++;
    while (at > 0 && strcmp(ids[at - 1], id) > 0) {
      memcpy(ids[at], ids[at - 1], sizeof ids[0]);
      at--;
    }
    memcpy(ids[at], id, sizeof ids[0]);
  }
  return n;
}

bool exists(const char* id) { return idOk(id) && faces::pkg::store::mounted() && LittleFS.exists(path(id)); }

bool load(const char* id, irdoc::Device& d) {
  if (!exists(id)) return false;
  File f = LittleFS.open(path(id), "r");
  if (!f || f.size() > irnames::MAX_FILE) return false;
  std::string text(f.size(), '\0');
  if (f.read((uint8_t*)&text[0], text.size()) != text.size()) return false;
  const char* why = nullptr;
  if (!irdoc::read(text.data(), text.size(), d, why) || d.id != id) {
    Log.printf("IR: /ir/%s.json unreadable (%s)\n", id, why ? why : "dev");
    return false;
  }
  return true;
}

uint32_t freeKB() {
  if (!faces::pkg::store::mounted()) return 0;
  const size_t total = LittleFS.totalBytes(), used = LittleFS.usedBytes();
  return used + SPARE >= total ? 0 : (total - used - SPARE) / 1024;
}

bool save(const irdoc::Device& d, const char*& why) {
  why = "write";
  if (!idOk(d.id.c_str()) || !begin()) return false;
  const std::string text = irdoc::write(d);
  const bool fresh = !exists(d.id.c_str());
  if (text.size() > irnames::MAX_FILE) { why = "full"; return false; }
  if (fresh) {
    char ids[irnames::MAX_DEVICES + 1][irnames::DEVICE_LEN + 1];
    if (list(ids, irnames::MAX_DEVICES + 1) >= irnames::MAX_DEVICES) { why = "full"; return false; }
  }
  if (freeKB() * 1024 < text.size() + 4096) { why = "full"; return false; }
  // written next to it, then renamed over it: a power cut never leaves half a file
  const String tmp = String(DIR_) + "/.tmp";
  File f = LittleFS.open(tmp, "w");
  if (!f) return false;
  const bool ok = f.write((const uint8_t*)text.data(), text.size()) == text.size();
  f.close();
  if (!ok || !LittleFS.rename(tmp, path(d.id.c_str()))) {
    LittleFS.remove(tmp);
    return false;
  }
  return true;
}

bool remove(const char* id) { return exists(id) && LittleFS.remove(path(id)); }

bool rename(const char* from, const char* to, const char*& why) {
  irdoc::Device d;
  why = "notfound";
  if (!load(from, d)) return false;
  why = "name";
  if (!idOk(to)) return false;
  why = "exists";
  if (strcmp(from, to) && exists(to)) return false;
  d.id = to;
  why = "write";
  const std::string text = irdoc::write(d);
  File f = LittleFS.open(path(to), "w");
  if (!f || f.write((const uint8_t*)text.data(), text.size()) != text.size()) return false;
  f.close();
  if (strcmp(from, to)) LittleFS.remove(path(from));
  return true;
}

}  // namespace store
}  // namespace ir
