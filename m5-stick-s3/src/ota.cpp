#include "ota.h"
#include <M5Unified.h>
#include <esp_ota_ops.h>
#include <esp_partition.h>
#include <mbedtls/sha256.h>
#include <new>
#include "config.h"
#include "netlog.h"
#include "buddy.h"
#include "battery.h"
#include "hid.h"
#include "ui.h"
#include "logic/ota_rx.h"
#include "logic/packet_ring.h"

// Arduino's initArduino() confirms a pending image right away unless this says
// "later" (weak in esp32-hal-misc.c). We confirm it in loop() once it has run.
extern "C" bool verifyRollbackLater() { return true; }

namespace ota {

constexpr uint32_t CONFIRM_AFTER_MS = 25000;   // a new image confirms itself after running this long
constexpr uint32_t STALL_MS         = 3000;    // no data this long: "waiting for the Mac"
constexpr uint32_t RESTART_DELAY_MS = 1500;    // after "done": the reply goes out, then reboot
constexpr size_t   RING_BYTES       = 24 * 1024;  // > WINDOW + framing
constexpr size_t   MAX_PACKET       = 512;

using Ring = PacketRing<RING_BYTES>;

// --- the flash side ------------------------------------------------------------

struct FlashSink : otarx::Sink {
  const esp_partition_t* part = nullptr;
  esp_ota_handle_t handle = 0;
  bool open_ = false;

  bool open(uint32_t size) override {
    part = esp_ota_get_next_update_partition(nullptr);
    if (!part) return false;
    // Sequential writes: each sector is erased as it's reached, so begin returns at once
    esp_err_t e = esp_ota_begin(part, OTA_WITH_SEQUENTIAL_WRITES, &handle);
    open_ = e == ESP_OK;
    Log.printf("OTA: writing %lu bytes to %s at 0x%lx%s\n", (unsigned long)size, part->label,
               (unsigned long)part->address, open_ ? "" : (String(" — esp_ota_begin failed: ") + esp_err_to_name(e)).c_str());
    return open_;
  }
  bool write(const uint8_t* p, size_t n) override {
    esp_err_t e = esp_ota_write(handle, p, n);
    if (e != ESP_OK) Log.printf("OTA: esp_ota_write: %s\n", esp_err_to_name(e));
    return e == ESP_OK;
  }
  // SHA-256 of what is actually in flash (read back), not of what we meant to write
  bool sha256(uint32_t size, uint8_t out[32]) override {
    static uint8_t buf[4096];
    mbedtls_sha256_context ctx;
    mbedtls_sha256_init(&ctx);
    mbedtls_sha256_starts(&ctx, 0);
    bool ok = true;
    for (uint32_t off = 0; off < size && ok; off += sizeof(buf)) {
      size_t n = min((uint32_t)sizeof(buf), size - off);
      ok = esp_partition_read(part, off, buf, n) == ESP_OK;
      if (ok) mbedtls_sha256_update(&ctx, buf, n);
    }
    if (ok) mbedtls_sha256_finish(&ctx, out);
    mbedtls_sha256_free(&ctx);
    return ok;
  }
  // Signed firmware (not yet): verify the image's signature here, before it can become the boot slot.
  bool commit() override {
    open_ = false;
    esp_err_t e = esp_ota_end(handle);  // checks the image (header, segments, its own checksum + hash)
    if (e == ESP_OK) e = esp_ota_set_boot_partition(part);
    if (e != ESP_OK) Log.printf("OTA: image refused: %s\n", esp_err_to_name(e));
    return e == ESP_OK;
  }
  void abort() override {
    if (open_) esp_ota_abort(handle);
    open_ = false;
  }
};

static FlashSink    sink;
static otarx::Receiver rx(sink);
static Ring*        ring = nullptr;
static portMUX_TYPE mux = portMUX_INITIALIZER_UNLOCKED;
static volatile uint32_t dropped = 0;  // packets that didn't fit in the queue (resent after a GAP)
static const char* (*busyWhy)() = nullptr;
static bool     pending_ = false;
static uint32_t restartAt = 0, lastDataAt = 0, startedAt = 0, drawnAt = 0;
static int      drawnPct = -1;
static bool     drawnStall = false;
static String   doneLine;  // a message shown after the update ends (cancelled / failed), until a press

void setBusyCheck(const char* (*why)()) { busyWhy = why; }
bool pendingVerify() { return pending_; }
bool active() { return rx.receiving() || restartAt; }

void bootCheck() {
  const esp_partition_t* running = esp_ota_get_running_partition();
  esp_ota_img_states_t st;
  if (running && esp_ota_get_state_partition(running, &st) == ESP_OK && st == ESP_OTA_IMG_PENDING_VERIFY) {
    pending_ = true;
    enableLoopWDT();  // a hang before the confirmation resets (and so rolls back) instead of sitting there
    Log.printf("OTA: running new firmware %s from %s — confirming in %lu s\n", FW_VERSION, running->label,
               (unsigned long)(CONFIRM_AFTER_MS / 1000));
  }
}

void onPacket(const uint8_t* data, size_t len) {
  if (!len || len > MAX_PACKET) return;
  portENTER_CRITICAL(&mux);
  bool ok = ring && rx.receiving() && ring->push(data, (uint16_t)len);
  portEXIT_CRITICAL(&mux);
  if (!ok) dropped = dropped + 1;
}

static void send(const String& body) { buddy::sendLine("{\"src\":\"agent\",\"evt\":\"ota\"," + body + "}"); }

static void reply(const otarx::Reply& r, const char* by = nullptr, const char* why = nullptr) {
  using K = otarx::Reply::Kind;
  String off = String(",\"offset\":") + r.offset;
  switch (r.kind) {
    case K::NONE: return;
    case K::READY:
      send(String("\"state\":\"ready\"") + off + ",\"size\":" + rx.size() + ",\"resumed\":" +
           (r.resumed ? "true" : "false") + ",\"window\":" + otarx::WINDOW +
           ",\"chunk\":" + (int)(buddy::audioPayloadMax() - otarx::HEADER));
      return;
    case K::REFUSED:
      send(String("\"state\":\"refused\",\"error\":\"") + otarx::errorName(r.error) + "\"" +
           (why ? String(",\"why\":\"") + why + "\"" : String()));
      return;
    case K::ACK:  send(String("\"state\":\"ack\"") + off); return;
    case K::GAP:  send(String("\"state\":\"ack\"") + off + ",\"gap\":true"); return;
    case K::DONE: send(String("\"state\":\"done\"") + off); return;
    case K::ERROR:
      send(String("\"state\":\"error\",\"error\":\"") + otarx::errorName(r.error) + "\"" + off);
      return;
    case K::CANCELLED:
      send(String("\"state\":\"cancelled\",\"by\":\"") + (by ? by : "agent") + "\"" + off);
      return;
  }
}

static void ended(const otarx::Reply& r) {
  using K = otarx::Reply::Kind;
  if (r.kind == K::ERROR && r.error != otarx::Error::INCOMPLETE) {
    Log.printf("OTA: failed (%s) at %lu of %lu bytes\n", otarx::errorName(r.error), (unsigned long)r.offset,
               (unsigned long)rx.size());
    doneLine = String("Update failed (") + otarx::errorName(r.error) + ")";
  }
}

// Writes what has arrived (in order); flash erase + write happen here, not in the BLE task.
static void drain() {
  static uint8_t pkt[MAX_PACKET];
  for (int i = 0; i < 64 && rx.receiving(); i++) {
    uint16_t n = 0;
    portENTER_CRITICAL(&mux);
    bool got = ring && ring->pop(pkt, sizeof(pkt), n);
    portEXIT_CRITICAL(&mux);
    if (!got) break;
    lastDataAt = millis();
    otarx::Reply r = rx.packet(pkt, n, millis());
    reply(r);
    ended(r);
  }
}

void control(JsonObjectConst msg) {
  if (msg["begin"].is<JsonObjectConst>()) {
    JsonObjectConst b = msg["begin"];
    otarx::Begin req;
    req.size = b["size"] | 0UL;
    req.sha256 = b["sha256"] | "";
    req.version = b["version"] | "";
    req.compressed = b["compressed"] | false;
    req.force = b["force"] | false;
    otarx::Conditions c;
    c.busy = busyWhy ? busyWhy() : nullptr;
    c.batteryPct = battery::level();
    c.usbPower = battery::usbPower();
    c.pendingVerify = pending_;
    const esp_partition_t* next = esp_ota_get_next_update_partition(nullptr);
    c.partitionSize = next ? next->size : 0;
    c.running = FW_VERSION;
    if (!ring) {  // once, in PSRAM if there is some
      void* mem = heap_caps_malloc(sizeof(Ring), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
      if (!mem) mem = malloc(sizeof(Ring));
      if (mem) ring = new (mem) Ring();
    }
    otarx::Reply r = ring ? rx.begin(req, c, millis()) : otarx::Reply{};
    if (!ring) {
      r.kind = otarx::Reply::Kind::ERROR;
      r.error = otarx::Error::OPEN;
    }
    if (r.kind == otarx::Reply::Kind::READY) {
      portENTER_CRITICAL(&mux);
      ring->clear();
      portEXIT_CRITICAL(&mux);
      dropped = 0;
      buddy::fastData();
      lastDataAt = startedAt = millis();
      doneLine = "";
      drawnPct = -1;
      ui::wake(60000);
      Log.printf("OTA: %s %s (%lu bytes) from %lu, %s\n", r.resumed ? "resuming" : "receiving", rx.version(),
                 (unsigned long)rx.size(), (unsigned long)r.offset, buddy::linkInfo().c_str());
    } else if (r.kind == otarx::Reply::Kind::REFUSED) {
      Log.printf("OTA: refused %s (%s%s%s)\n", req.version, otarx::errorName(r.error), c.busy ? ": " : "",
                 c.busy ? c.busy : "");
    }
    reply(r, nullptr, r.error == otarx::Error::BUSY ? c.busy : nullptr);
    return;
  }
  if (msg["end"].is<int>() || msg["end"].is<bool>()) {
    drain();  // anything still queued (the agent waits for the last ack, so normally nothing)
    uint32_t t0 = millis();
    otarx::Reply r = rx.end(millis());
    if (r.kind == otarx::Reply::Kind::DONE && !restartAt) {
      Log.printf("OTA: %s verified (sha256 read back in %lu ms) and set to boot, %lu s in all — restarting\n",
                 rx.version(), (unsigned long)(millis() - t0), (unsigned long)((millis() - startedAt) / 1000));
      restartAt = millis() + RESTART_DELAY_MS;
      ui::wake(10000);
      ui::drawMessage("Restarting", (String("Firmware ") + rx.version()).c_str(), 0x03E0);
    }
    reply(r);
    ended(r);
    return;
  }
  if (msg["cancel"].is<int>() || msg["cancel"].is<bool>()) {
    otarx::Reply r = rx.cancel();
    if (r.kind == otarx::Reply::Kind::CANCELLED) {
      Log.println("OTA: cancelled by the agent");
      doneLine = "Update cancelled on the Mac";
    } else {
      r.kind = otarx::Reply::Kind::CANCELLED;  // nothing running: say so anyway, the agent is waiting
    }
    reply(r, "agent");
    return;
  }
  if (msg["status"].is<int>() || msg["status"].is<bool>()) {
    send(String("\"state\":\"status\",\"receiving\":") + (rx.receiving() ? "true" : "false") +
         ",\"offset\":" + rx.next() + ",\"size\":" + rx.size() + ",\"pending\":" + (pending_ ? "true" : "false"));
  }
}

bool takeNotice(String& title) {
  if (!doneLine.length() || rx.receiving()) return false;
  title = doneLine;
  doneLine = "";
  return true;
}

void cancelByUser() {
  otarx::Reply r = rx.cancel();
  if (r.kind != otarx::Reply::Kind::CANCELLED) return;
  Log.println("OTA: cancelled on the stick");
  reply(r, "stick");
  doneLine = "Update cancelled";
}

static void draw() {
  bool stall = millis() - lastDataAt > STALL_MS;
  int pct = rx.percent();
  if (pct == drawnPct && stall == drawnStall && millis() - drawnAt < 1000) return;
  drawnPct = pct;
  drawnStall = stall;
  drawnAt = millis();
  ui::drawUpdate(pct, rx.version(), stall ? "Waiting for the Mac..." : "Keep the stick near the Mac");
}

void loop() {
  if (pending_ && millis() > CONFIRM_AFTER_MS && hid::state() != hid::Bt::OFF) {
    esp_err_t e = esp_ota_mark_app_valid_cancel_rollback();
    pending_ = false;
    disableLoopWDT();
    Log.printf("OTA: firmware %s confirmed (%s)\n", FW_VERSION, esp_err_to_name(e));
    if (buddy::agentLinked()) send(String("\"state\":\"confirmed\",\"fw\":\"") + FW_VERSION + "\"");
  }
  if (restartAt) {
    ui::keepAwake();
    if (millis() >= restartAt) {
      Serial.flush();
      ESP.restart();
    }
    return;
  }
  if (!rx.receiving()) return;
  drain();
  otarx::Reply r = rx.poll(millis());
  reply(r);
  ended(r);
  if (rx.receiving()) {
    ui::keepAwake();
    draw();
  }
}

}  // namespace ota
