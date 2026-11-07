#include "store.h"
#include <ArduinoJson.h>
#include <LittleFS.h>
#include <mbedtls/sha256.h>
#include "engine.h"
#include "../../netlog.h"

namespace faces {
namespace pkg {
namespace store {

static const char* DIR_ = "/faces";
static const char* PART = "/faces/.part";
static const char* PART_JSON = "/faces/.part.json";
static bool mounted_ = false;
static File part_;  // open while a face arrives

static String path(const char* id) { return String(DIR_) + "/" + id + ".sfb"; }

static bool idOk(const char* id) {
  const size_t n = strlen(id);
  if (n < 3 || n > 12 || id[0] < 'a' || id[0] > 'z') return false;
  for (size_t i = 0; i < n; i++)
    if (!((id[i] >= 'a' && id[i] <= 'z') || (id[i] >= '0' && id[i] <= '9'))) return false;
  return true;
}

bool begin() {
  if (mounted_) return true;
  const uint32_t t0 = millis();
  // formatOnFail: the first time (or a partition that was never LittleFS) takes ~1-2 s, once
  mounted_ = LittleFS.begin(true, "/fs", 4, "spiffs");
  if (!mounted_) {
    Log.println("Faces: storage didn't mount (no package faces)");
    return false;
  }
  if (!LittleFS.exists(DIR_)) LittleFS.mkdir(DIR_);
  Log.printf("Faces: storage %u KB, %u KB used (%lu ms)\n", (unsigned)(LittleFS.totalBytes() / 1024),
             (unsigned)(LittleFS.usedBytes() / 1024), (unsigned long)(millis() - t0));
  return true;
}

bool mounted() { return mounted_; }

int list(char ids[][13], int max) {
  if (!mounted_) return 0;
  File dir = LittleFS.open(DIR_);
  if (!dir || !dir.isDirectory()) return 0;
  int n = 0;
  for (File f = dir.openNextFile(); f && n < max; f = dir.openNextFile()) {
    const char* name = f.name();  // "catnap.sfb"
    const size_t len = strlen(name);
    if (f.isDirectory() || len < 5 || len > 16 || strcmp(name + len - 4, ".sfb")) continue;
    char id[13];
    memcpy(id, name, len - 4);
    id[len - 4] = 0;
    if (!idOk(id)) continue;
    int at = n++;  // sorted insert: the pickers show them in a steady order
    while (at > 0 && strcmp(ids[at - 1], id) > 0) {
      memcpy(ids[at], ids[at - 1], 13);
      at--;
    }
    memcpy(ids[at], id, 13);
  }
  return n;
}

uint32_t fileSize(const char* id) {
  if (!mounted_ || !idOk(id)) return 0;
  File f = LittleFS.open(path(id), "r");
  return f ? f.size() : 0;
}

static uint8_t* readAll(const char* p, size_t& n) {
  n = 0;
  File f = LittleFS.open(p, "r");
  if (!f || f.size() == 0 || f.size() > MAX_FILE) return nullptr;
  const size_t size = f.size();
  uint8_t* buf = (uint8_t*)bigAlloc(size);
  if (!buf) return nullptr;
  if (f.read(buf, size) != size) {
    free(buf);
    return nullptr;
  }
  n = size;
  return buf;
}

uint8_t* read(const char* id, size_t& n) {
  n = 0;
  if (!mounted_ || !idOk(id)) return nullptr;
  return readAll(path(id).c_str(), n);
}

static bool hashFile(const char* p, uint32_t size, uint8_t out[32]) {
  File f = LittleFS.open(p, "r");
  if (!f || f.size() < size) return false;
  static uint8_t buf[1024];
  mbedtls_sha256_context ctx;
  mbedtls_sha256_init(&ctx);
  mbedtls_sha256_starts(&ctx, 0);
  bool ok = true;
  for (uint32_t off = 0; off < size && ok;) {
    const size_t want = min((uint32_t)sizeof buf, size - off);
    const size_t got = f.read(buf, want);
    ok = got == want;
    if (ok) mbedtls_sha256_update(&ctx, buf, got);
    off += got;
  }
  if (ok) mbedtls_sha256_finish(&ctx, out);
  mbedtls_sha256_free(&ctx);
  return ok;
}

bool sha256(const char* id, uint8_t out[32]) {
  if (!mounted_ || !idOk(id)) return false;
  const uint32_t n = fileSize(id);
  return n && hashFile(path(id).c_str(), n, out);
}

bool remove(const char* id) {
  if (!mounted_ || !idOk(id)) return false;
  return LittleFS.remove(path(id));
}

int wipe() {
  if (!mounted_) return 0;
  partDrop();
  char ids[MAX_FACES * 2][13];
  const int n = list(ids, MAX_FACES * 2);
  int gone = 0;
  for (int i = 0; i < n; i++) gone += remove(ids[i]) ? 1 : 0;
  return gone;
}

uint32_t freeBytes() {
  if (!mounted_) return 0;
  const size_t total = LittleFS.totalBytes(), used = LittleFS.usedBytes();
  return used + SPARE >= total ? 0 : total - used - SPARE;
}

// --- The part file ---------------------------------------------------------------------------

bool partFind(Part& p, uint32_t& have) {
  have = 0;
  p = Part{};
  if (!mounted_ || !LittleFS.exists(PART_JSON) || !LittleFS.exists(PART)) return false;
  File j = LittleFS.open(PART_JSON, "r");
  JsonDocument doc;
  if (!j || deserializeJson(doc, j)) return false;
  snprintf(p.id, sizeof p.id, "%s", doc["id"] | "");
  snprintf(p.sha, sizeof p.sha, "%s", doc["sha256"] | "");
  snprintf(p.version, sizeof p.version, "%s", doc["version"] | "");
  p.size = doc["size"] | 0UL;
  File f = LittleFS.open(PART, "r");
  have = f ? f.size() : 0;
  return p.size > 0;
}

bool partOpen(const Part& p, bool resume) {
  if (!mounted_) return false;
  if (part_) part_.close();
  if (!resume) {
    LittleFS.remove(PART);
    File j = LittleFS.open(PART_JSON, "w");
    if (!j) return false;
    JsonDocument doc;
    doc["id"] = p.id;
    doc["size"] = p.size;
    doc["sha256"] = p.sha;
    doc["version"] = p.version;
    serializeJson(doc, j);
  }
  part_ = LittleFS.open(PART, resume ? "a" : "w");
  return (bool)part_;
}

bool partWrite(const uint8_t* b, size_t n) { return part_ && part_.write(b, n) == n; }

uint32_t partBytes() {
  if (part_) return part_.size();
  if (!mounted_ || !LittleFS.exists(PART)) return 0;
  File f = LittleFS.open(PART, "r");
  return f ? f.size() : 0;
}

bool partSha256(uint32_t size, uint8_t out[32]) {
  if (part_) part_.close();  // flushed: read back what is really on flash
  return hashFile(PART, size, out);
}

uint8_t* partRead(size_t& n) {
  if (part_) part_.close();
  return readAll(PART, n);
}

bool partCommit(const char* id) {
  if (part_) part_.close();
  if (!idOk(id)) return false;
  const bool ok = LittleFS.rename(PART, path(id));  // LittleFS: replaces the old file atomically
  LittleFS.remove(PART_JSON);
  return ok;
}

void partDrop() {
  if (part_) part_.close();
  if (!mounted_) return;
  LittleFS.remove(PART);
  LittleFS.remove(PART_JSON);
}

}  // namespace store
}  // namespace pkg
}  // namespace faces
