#include "install.h"
#include <esp_heap_caps.h>
#include <new>
#include "engine.h"
#include "package.h"
#include "store.h"
#include "../../buddy.h"
#include "../../netlog.h"
#include "../../ota.h"
#include "../../ui_draw.h"
#include "../../logic/ota_rx.h"
#include "../../logic/packet_ring.h"

namespace faces {
namespace pkg {
namespace install {

constexpr size_t   RING_BYTES = 24 * 1024;  // > WINDOW + framing, as for firmware updates
constexpr size_t   MAX_PACKET = 512;
constexpr uint32_t TOAST_MS = 3500;         // "Installed Cat Nap" stays this long
constexpr uint32_t STALL_MS = 3000;

using Ring = PacketRing<RING_BYTES>;

// --- The file side: /fs/faces/.part (store.cpp) -------------------------------------------------

struct FileSink : otarx::Sink {
  store::Part next;          // the put being started (open() takes it)
  store::Part part;          // the one arriving
  uint32_t from = 0;
  const char* error = "invalid";
  String why;
  Info info;                 // what commit() found in it

  bool open(uint32_t) override {
    part = next;
    store::Part old;
    uint32_t have = 0;
    // the same face (id, size, sha256) half-received before a restart: carry on from there
    const bool resume = store::partFind(old, have) && !strcmp(old.id, part.id) && old.size == part.size &&
                        !strcmp(old.sha, part.sha) && have > 0 && have < part.size;
    from = resume ? have : 0;
    return store::partOpen(part, resume);
  }
  uint32_t resumeFrom() override { return from; }
  bool write(const uint8_t* p, size_t n) override { return store::partWrite(p, n); }
  bool sha256(uint32_t size, uint8_t out[32]) override { return store::partSha256(size, out); }
  // It arrived whole and the sha256 matches: load it as the engine will (FORMAT.md section 16)
  // and only then make it <id>.sfb.
  bool commit() override {
    size_t n = 0;
    uint8_t* buf = store::partRead(n);
    if (!buf) {
      error = "read";
      why = "can't read it back";
      store::partDrop();
      return false;
    }
    Engine* e = new Engine();
    const LoadError le = e->load(buf, n, why);
    info = e->info();
    delete e;
    free(buf);
    if (le == LoadError::NONE && strcmp(info.id, part.id)) {
      why = String("its id is ") + info.id;
      error = "invalid";
      store::partDrop();
      return false;
    }
    if (le != LoadError::NONE) {
      error = le == LoadError::FORMAT ? "format" : le == LoadError::FIRMWARE ? "firmware" : "invalid";
      store::partDrop();
      return false;
    }
    if (!store::partCommit(part.id)) {
      error = "write";
      why = "can't rename it";
      store::partDrop();
      return false;
    }
    return true;
  }
  void abort() override { store::partDrop(); }
};

static FileSink sink;
static otarx::Receiver rx(sink, otarx::Receiver::Kind::FILE);
static Ring* ring = nullptr;
static portMUX_TYPE mux = portMUX_INITIALIZER_UNLOCKED;
static volatile bool receiving_ = false;
static const char* (*busyWhy)() = nullptr;
static String   name_;                       // what the toast calls it
static uint32_t startedAt = 0, lastDataAt = 0, toastUntil = 0;
static String   toastLine;
static uint16_t toastBg = 0;

void setBusyCheck(const char* (*why)()) { busyWhy = why; }
bool receiving() { return receiving_; }

void onPacket(const uint8_t* data, size_t len) {
  if (!len || len > MAX_PACKET) return;
  portENTER_CRITICAL(&mux);
  if (ring && receiving_) ring->push(data, (uint16_t)len);  // (a full queue: the gap makes the Mac resend)
  portEXIT_CRITICAL(&mux);
}

// acks are quiet; everything else is logged
static void send(const String& body, bool log = true) {
  buddy::sendLine("{\"src\":\"agent\",\"evt\":\"faces\"," + body + "}", log);
}

static String q(const String& s) {
  String o = "\"";
  for (char c : s) {
    if (c == '"' || c == '\\') o += '\\';
    if ((uint8_t)c >= 0x20) o += c;
  }
  return o + "\"";
}

static void refuse(const char* error, const String& why = "") {
  send(String("\"state\":\"refused\",\"error\":\"") + error + "\"" + (why.length() ? ",\"why\":" + q(why) : String()));
  Log.printf("Faces: install refused (%s%s%s)\n", error, why.length() ? ": " : "", why.c_str());
}

static void showToast(const String& line, uint16_t bg) {
  toastLine = line;
  toastBg = bg;
  toastUntil = millis() + TOAST_MS;
}

static void reply(const otarx::Reply& r, const char* by = nullptr) {
  using K = otarx::Reply::Kind;
  const String off = String(",\"offset\":") + r.offset;
  switch (r.kind) {
    case K::NONE: return;
    case K::READY:
      send(String("\"state\":\"ready\",\"id\":") + q(sink.part.id) + off + ",\"size\":" + rx.size() +
           ",\"window\":" + otarx::WINDOW + ",\"chunk\":" + (int)(buddy::audioPayloadMax() - otarx::HEADER) +
           ",\"resumed\":" + (r.resumed ? "true" : "false"));
      return;
    case K::REFUSED:
      refuse(otarx::errorName(r.error));
      return;
    case K::ACK: send(String("\"state\":\"ack\"") + off, false); return;
    case K::GAP: send(String("\"state\":\"ack\"") + off + ",\"gap\":true"); return;
    case K::DONE:
      send(String("\"state\":\"done\",\"id\":") + q(sink.part.id) + ",\"version\":" + q(sink.info.version));
      return;
    case K::ERROR: {
      // a commit that failed is the face itself (invalid, format, firmware): the sink knows why
      const bool face = r.error == otarx::Error::IMAGE;
      const String error = face ? sink.error : otarx::errorName(r.error);
      const String why = face ? sink.why : String();
      send(String("\"state\":\"error\",\"error\":\"") + error + "\"" + (why.length() ? ",\"why\":" + q(why) : String()) +
           off);
      return;
    }
    case K::CANCELLED:
      send(String("\"state\":\"cancelled\",\"by\":\"") + (by ? by : "agent") + "\"" + off);
      return;
  }
}

// After every reply: the end of a session (done, failed) updates the list and the toast.
static void after(const otarx::Reply& r) {
  using K = otarx::Reply::Kind;
  receiving_ = rx.receiving();
  if (r.kind == K::DONE) {
    Log.printf("Faces: installed %s %s (%lu bytes, %lu ms)\n", sink.part.id, sink.info.version,
               (unsigned long)rx.size(), (unsigned long)(millis() - startedAt));
    pkg::installed(sink.part.id);
    showToast(String("Installed ") + (sink.info.name[0] ? sink.info.name : sink.part.id), ui::blend(ui::GROUND, ui::OK, 0.6f));
  } else if (r.kind == K::ERROR && r.error != otarx::Error::INCOMPLETE) {
    const bool face = r.error == otarx::Error::IMAGE;
    Log.printf("Faces: install of %s failed (%s%s%s)\n", sink.part.id, face ? sink.error : otarx::errorName(r.error),
               face ? ": " : "", face ? sink.why.c_str() : "");
    showToast(String("Couldn't install ") + name_, ui::blend(ui::GROUND, ui::ERR, 0.7f));
  }
}

static void drain() {
  static uint8_t pkt[MAX_PACKET];
  for (int i = 0; i < 64 && rx.receiving(); i++) {
    uint16_t n = 0;
    portENTER_CRITICAL(&mux);
    const bool got = ring && ring->pop(pkt, sizeof pkt, n);
    portEXIT_CRITICAL(&mux);
    if (!got) break;
    lastDataAt = millis();
    const otarx::Reply r = rx.packet(pkt, n, millis());
    reply(r);
    after(r);
  }
}

static bool idOk(const char* id) {
  const size_t n = strlen(id);
  if (n < 3 || n > 12 || id[0] < 'a' || id[0] > 'z') return false;
  for (size_t i = 0; i < n; i++)
    if (!((id[i] >= 'a' && id[i] <= 'z') || (id[i] >= '0' && id[i] <= '9'))) return false;
  return true;
}

static void put(JsonObjectConst p) {
  const char* id = p["id"] | "";
  const uint32_t size = p["size"] | 0UL;
  if (!idOk(id)) return refuse("id", "3-12 of a-z, 0-9");
  static const char* const RESERVED[] = {"all", "none", "demo", "default", "off", "face", "faces", "stickybot"};
  const int known = faces::find(id);
  bool reserved = known >= 0 && !pkg::isPackage(faces::at(known));  // a built-in face's id
  for (const char* r : RESERVED) reserved = reserved || !strcmp(r, id);
  if (reserved) return refuse("id", "a built-in face's (or a reserved) id");
  if (ota::active()) return refuse("ota", "a firmware update is running");
  const char* busy = rx.receiving() || !busyWhy ? nullptr : busyWhy();  // (a face arriving: this resumes or replaces it)
  if (busy) return refuse("busy", busy);
  if (size == 0 || size > store::MAX_FILE) return refuse("size", "1 byte to 64 KB");
  if (!store::mounted()) return refuse("full", "no storage");
  char ids[store::MAX_FACES + 1][13];
  const int n = store::list(ids, store::MAX_FACES + 1);
  const uint32_t old = store::fileSize(id);
  if (!old && n >= store::MAX_FACES) return refuse("full", String(store::MAX_FACES) + " faces installed");
  if (store::freeBytes() + old + store::partBytes() < size) return refuse("full", "not enough space");

  otarx::Begin b;
  b.size = size;
  b.sha256 = p["sha256"] | "";
  b.version = p["version"] | "";
  snprintf(sink.next.id, sizeof sink.next.id, "%s", id);
  snprintf(sink.next.sha, sizeof sink.next.sha, "%s", b.sha256);
  snprintf(sink.next.version, sizeof sink.next.version, "%s", b.version);
  sink.next.size = size;
  otarx::Conditions c;
  c.partitionSize = store::MAX_FILE;
  if (!ring) {  // once, in PSRAM if there is some
    void* mem = heap_caps_malloc(sizeof(Ring), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (!mem) mem = malloc(sizeof(Ring));
    if (mem) ring = new (mem) Ring();
  }
  if (!ring) return refuse("full", "out of memory");
  const bool wasReceiving = rx.receiving();
  const otarx::Reply r = rx.begin(b, c, millis());
  if (r.kind == otarx::Reply::Kind::READY) {
    portENTER_CRITICAL(&mux);
    ring->clear();
    portEXIT_CRITICAL(&mux);
    receiving_ = true;
    buddy::fastData();
    lastDataAt = startedAt = millis();
    const char* name = p["name"] | "";  // optional: what the toast calls it
    name_ = strlen(name) && strlen(name) <= 24 ? String(name) : String(id);
    toastUntil = 0;
    ui::wake(15000);
    Log.printf("Faces: %s %s %s (%lu bytes) from %lu%s\n", r.resumed ? "resuming" : "receiving", id, b.version,
               (unsigned long)size, (unsigned long)r.offset, wasReceiving && !r.resumed ? " (the last one dropped)" : "");
  } else if (r.kind == otarx::Reply::Kind::ERROR) {  // the part file didn't open
    send(String("\"state\":\"error\",\"error\":\"") + otarx::errorName(r.error) + "\",\"offset\":0");
    receiving_ = rx.receiving();
    return;
  }
  reply(r);
}

void control(JsonObjectConst msg) {
  if (msg["put"].is<JsonObjectConst>()) {
    put(msg["put"]);
    return;
  }
  if (msg["end"].is<int>() || msg["end"].is<bool>()) {
    drain();  // anything still queued (the agent waits for the last ack, so normally nothing)
    const otarx::Reply r = rx.end(millis());
    reply(r);
    after(r);
    return;
  }
  if (msg["cancel"].is<int>() || msg["cancel"].is<bool>()) {
    otarx::Reply r = rx.cancel();
    if (r.kind == otarx::Reply::Kind::CANCELLED) {
      Log.printf("Faces: install of %s cancelled by the agent\n", sink.part.id);
      toastUntil = 0;
    } else {
      r.kind = otarx::Reply::Kind::CANCELLED;  // nothing running: say so anyway, the agent is waiting
    }
    receiving_ = rx.receiving();
    reply(r, "agent");
    return;
  }
  if (msg["rm"].is<const char*>()) {
    const String id = msg["rm"].as<const char*>();
    if (rx.receiving() && (id == "all" || id == sink.part.id)) {  // what's arriving goes too
      rx.cancel();
      receiving_ = false;
    }
    const int n = pkg::remove(id);
    if (n || id == "all") send("\"state\":\"removed\",\"id\":" + q(id) + ",\"n\":" + n);
    else send("\"state\":\"error\",\"error\":\"notfound\",\"id\":" + q(id));
    return;
  }
}

void cancelByUser() {
  otarx::Reply r = rx.cancel();
  receiving_ = rx.receiving();
  if (r.kind != otarx::Reply::Kind::CANCELLED) return;
  Log.printf("Faces: install of %s cancelled on the stick\n", sink.part.id);
  reply(r, "stick");
  showToast("Install cancelled", ui::blend(ui::GROUND, ui::ASK, 0.6f));
}

void loop() {
  if (!rx.receiving()) {
    receiving_ = false;
    return;
  }
  drain();
  const otarx::Reply r = rx.poll(millis());
  reply(r);
  after(r);
}

bool toast(String& line, uint16_t& bg) {
  if (rx.receiving()) {
    const bool stall = millis() - lastDataAt > STALL_MS;
    line = "Installing " + name_ + (stall ? "  waiting" : "  " + String(rx.percent()) + " %");
    bg = ui::blend(ui::GROUND, ui::AGENT, 0.45f);
    return true;
  }
  if (toastUntil && (int32_t)(millis() - toastUntil) < 0) {
    line = toastLine;
    bg = toastBg;
    return true;
  }
  toastUntil = 0;
  return false;
}

}  // namespace install
}  // namespace pkg
}  // namespace faces
