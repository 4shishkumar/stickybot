#include "ir.h"
#include <esp_heap_caps.h>
#include <new>
#include "pins.h"
#include "rmt.h"
#include "store.h"
#include "../buddy.h"
#include "../netlog.h"
#include "../logic/ir_codec.h"
#include "../logic/ir_learn.h"
#include "../logic/ir_names.h"

namespace ir {

using ircodec::Parsed;
using ircodec::Proto;

constexpr uint32_t LEARN_STEP_MS = 30000;  // per press
constexpr int      MAX_REPEAT = 50;
constexpr int      PAGE = 12;               // buttons per @ir list page

static void event(JsonDocument& d) {
  d["src"] = "agent";
  d["evt"] = "ir";
  String out;
  serializeJson(d, out);
  buddy::sendLine(out);
}

static String jsonStr(const String& s) {
  String o = "\"";
  for (char c : s) {
    if (c == '"' || c == '\\') o += '\\';
    if ((uint8_t)c >= 0x20) o += c;
  }
  return o + "\"";
}

static protocol::Result ok(const String& extra = "") {
  return {200, "{\"ok\":true" + (extra.length() ? "," + extra : String()) + "}"};
}
static protocol::Result fail(const String& code, const String& msg, int status = 400) {
  return {status, "{\"ok\":false,\"error\":" + jsonStr(msg) + ",\"code\":" + jsonStr(code) + "}"};
}

// --- Sending ------------------------------------------------------------------------------------

static struct {
  bool     active = false, held = false, toggle = false;
  Parsed   p;
  int      sent = 0, total = 0;
  uint32_t periodUs = 0, nextAt = 0;  // micros()
} tx;
// The big buffers live in PSRAM (made in begin()): nothing reads them from an interrupt
static uint16_t* txRaw = nullptr;
static uint16_t* frame = nullptr;
constexpr size_t CODE_MAX = ircodec::MAX_RAW * 2 + 64;
static char* codeBuf = nullptr;

static void* bigBuf(size_t n) {
  void* p = heap_caps_malloc(n, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
  return p ? p : malloc(n);
}

static uint32_t rawPeriodUs(const uint16_t* t, size_t n) {
  uint32_t sum = 0;
  for (size_t i = 0; i < n; i++) sum += t[i];
  return sum + 45000;  // a short pause after a raw code before it repeats
}

static void startSend(const Parsed& p, const uint16_t* raw, int repeat, bool held) {
  if (p.code.proto == Proto::RAW && raw != txRaw) memcpy(txRaw, raw, p.n * sizeof(uint16_t));
  tx.p = p;
  tx.toggle = !tx.toggle;  // rc5 / rc6: a new press
  tx.sent = 0;
  tx.held = held;
  tx.total = max<int>(ircodec::minFrames(p.code.proto), 1 + constrain(repeat, 0, MAX_REPEAT));
  tx.periodUs = p.code.proto == Proto::RAW ? rawPeriodUs(txRaw, p.n) : ircodec::periodUs(p.code.proto);
  tx.nextAt = micros();
  tx.active = true;
}

static void txLoop() {
  if (!tx.active || (int32_t)(micros() - tx.nextAt) < 0 || rmt::txBusy()) return;
  if (!tx.held && tx.sent >= tx.total) {
    tx.active = false;
    return;
  }
  size_t n;
  uint32_t hz = ircodec::DEFAULT_KHZ * 1000;
  if (tx.p.code.proto == Proto::RAW) {
    memcpy(frame, txRaw, tx.p.n * sizeof(uint16_t));
    n = tx.p.n;
    hz = tx.p.khz * 1000;
  } else {
    n = tx.sent == 0 ? ircodec::encode(tx.p.code, frame, ircodec::MAX_RAW, tx.toggle)
                     : ircodec::encodeRepeat(tx.p.code, frame, ircodec::MAX_RAW, tx.toggle);
    hz = ircodec::carrierHz(tx.p.code.proto);
  }
  if (!n || !rmt::send(frame, n, hz)) {
    tx.active = false;
    return;
  }
  tx.sent++;
  tx.nextAt += tx.periodUs;
  if ((int32_t)(micros() - tx.nextAt) > 0) tx.nextAt = micros();  // fell behind: no burst
}

bool sending() { return tx.active; }

void releaseHeld() {
  if (!tx.held) return;
  tx.held = false;
  tx.total = max(tx.sent, (int)ircodec::minFrames(tx.p.code.proto));
}

static const char* sendCode(const String& code, int repeat, bool held) {
  Parsed p;
  if (!txRaw || !frame) return "hardware";
  if (!ircodec::parse(code.c_str(), p, txRaw)) return "code";
  if (!rmt::txReady()) return "hardware";
  startSend(p, txRaw, repeat, held);
  return nullptr;
}

const char* sendButton(const char* dev, const char* btn, int repeat, bool held) {
  char id[irnames::DEVICE_LEN + 1], name[irnames::BUTTON_LEN + 1];
  if (!irnames::deviceId(dev, id) || !irnames::buttonName(btn, name)) return "notfound";
  irdoc::Device d;
  if (!store::load(id, d)) return "notfound";
  const int at = d.find(name);
  if (at < 0) return "notfound";
  const char* why = sendCode(d.buttons[at].code.c_str(), repeat, held);
  if (!why) Log.printf("IR: sent %s / %s\n", id, name);
  return why;
}

// --- Learning --------------------------------------------------------------------------------------

static irlearn::Learner* learnerP = nullptr;
static uint16_t* capBuf = nullptr;
static LearnView view_;
static bool once_ = false;
static uint32_t deadline = 0;

const LearnView& learnView() { return view_; }
bool learning() { return view_.active; }

static void learnEvent(const char* state, const char* extra = nullptr, const char* value = nullptr) {
  JsonDocument d;
  d["state"] = state;
  d["dev"] = view_.dev;
  if (view_.btn.length()) d["btn"] = view_.btn;
  if (extra) d[extra] = value;
  if (!strcmp(state, "learning")) d["step"] = view_.step;
  event(d);
  Log.printf("IR learn: %s %s/%s%s%s\n", state, view_.dev.c_str(), view_.btn.c_str(), value ? " " : "",
             value ? value : "");
}

static void finish(const char* last, bool good) {
  rmt::rxStop();
  view_.active = false;
  view_.listening = false;
  view_.last = last;
  view_.lastAt = millis();
  view_.ok = good;
}

const char* startLearn(const char* dev, const char* btn, bool once) {
  char id[irnames::DEVICE_LEN + 1], name[irnames::BUTTON_LEN + 1] = "";
  if (!irnames::deviceId(dev && *dev ? dev : "remote", id)) return "name";
  if (btn && *btn && !irnames::buttonName(btn, name)) return "name";
  if (view_.active) cancelLearn("agent");
  if (!store::begin()) return "storage";
  if (!store::exists(id)) {
    char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
    if (store::list(ids, irnames::MAX_DEVICES) >= irnames::MAX_DEVICES) return "full";
  } else if (!*name) {
    irdoc::Device d;
    if (store::load(id, d) && (int)d.buttons.size() >= irnames::MAX_BUTTONS) return "full";
  }
  if (!rmt::rxStart()) return "hardware";
  if (!learnerP) return "hardware";
  learnerP->start(once);
  once_ = once;
  view_ = LearnView{};
  view_.active = true;
  view_.step = 1;
  view_.dev = id;
  view_.btn = name;
  deadline = millis() + LEARN_STEP_MS;
  learnEvent("learning");
  return nullptr;
}

static const char* startListen(uint32_t secs) {
  if (view_.active) cancelLearn("agent");
  if (!rmt::rxStart()) return "hardware";
  view_ = LearnView{};
  view_.active = view_.listening = true;
  view_.dev = "listen";
  deadline = millis() + constrain(secs, 1u, 60u) * 1000;
  JsonDocument d;
  d["state"] = "listening";
  d["secs"] = constrain(secs, 1u, 60u);
  event(d);
  return nullptr;
}

void cancelLearn(const char* by) {
  if (!view_.active) return;
  const bool listening = view_.listening;
  finish("Cancelled", false);
  JsonDocument d;
  d["state"] = listening ? "listened" : "cancelled";
  d["by"] = by;
  d["frames"] = rmt::rxFrames();
  event(d);
}

static void saveLearned() {
  char* code = codeBuf;
  if (!learnerP->code(code, CODE_MAX)) {
    learnEvent("error", "error", "toolong");
    finish("Too long to keep", false);
    return;
  }
  irdoc::Device d;
  if (!store::load(view_.dev.c_str(), d)) d.id = view_.dev.c_str();
  if (!view_.btn.length()) view_.btn = d.nextName().c_str();
  const char* why = nullptr;
  if (!d.put(view_.btn.c_str(), code, false, why) || !store::save(d, why)) {
    learnEvent("error", "error", why ? why : "write");
    finish(why && !strcmp(why, "full") ? "No room left" : "Couldn't save", false);
    return;
  }
  JsonDocument e;
  e["state"] = "saved";
  e["dev"] = view_.dev;
  e["btn"] = view_.btn;
  e["p"] = learnerP->decoded() ? ircodec::protoName(learnerP->protoCode()) : "raw";
  e["code"] = strlen(code) <= 64 ? code : "";
  e["n"] = (int)learnerP->rawCount();
  event(e);
  Log.printf("IR learn: saved %s/%s = %.60s\n", view_.dev.c_str(), view_.btn.c_str(), code);
  finish("Saved", true);
}

static void learnLoop() {
  if (!capBuf || !learnerP) return;
  if (!view_.active) return;
  const size_t n = rmt::takeCapture(capBuf, ircodec::MAX_RAW);
  if (n && view_.listening) {
    char* code = codeBuf;
    ircodec::Code c;
    JsonDocument d;
    d["state"] = "heard";
    d["n"] = (int)n;
    if (ircodec::decodeCapture(capBuf, n, c) && ircodec::format(c, code, CODE_MAX)) d["code"] = code;
    else d["p"] = "raw";
    view_.last = d["code"] | "raw";
    view_.lastAt = millis();
    event(d);
  } else if (n) {
    switch (learnerP->capture(capBuf, n)) {
      case irlearn::Outcome::IGNORED: break;
      case irlearn::Outcome::FIRST:
        view_.step = 2;
        view_.last = "Got it";
        view_.lastAt = millis();
        deadline = millis() + LEARN_STEP_MS;
        learnEvent("learning");
        break;
      case irlearn::Outcome::MISMATCH:
        view_.last = "Didn't match";
        view_.lastAt = millis();
        deadline = millis() + LEARN_STEP_MS;
        learnEvent("mismatch");
        break;
      case irlearn::Outcome::DONE: saveLearned(); return;
    }
  }
  if ((int32_t)(millis() - deadline) > 0) {
    if (view_.listening) {
      cancelLearn("timeout");
    } else {
      learnEvent("timeout");
      finish("Nothing heard", false);
    }
  }
}

// --- @ir -------------------------------------------------------------------------------------------

static String deviceListJson() {
  char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
  const int n = store::list(ids, irnames::MAX_DEVICES);
  String j = "\"devices\":[";
  for (int i = 0; i < n; i++) {
    irdoc::Device d;
    int nb = 0, nf = 0;
    if (store::load(ids[i], d)) {
      nb = d.buttons.size();
      for (auto& b : d.buttons) nf += b.fav;
    }
    j += String(i ? "," : "") + "{\"id\":" + jsonStr(ids[i]) + ",\"n\":" + nb + ",\"fav\":" + nf + "}";
  }
  return j + "],\"max\":" + irnames::MAX_DEVICES + ",\"maxButtons\":" + irnames::MAX_BUTTONS +
         ",\"free\":" + store::freeKB();
}

static const char* message(const char* code) {
  if (!strcmp(code, "notfound")) return "No such remote or button";
  if (!strcmp(code, "code")) return "Not a code the stick can send";
  if (!strcmp(code, "name")) return "Names: letters, digits, spaces, + - _ . #";
  if (!strcmp(code, "full")) return "No room: too many remotes / buttons, or the flash is full";
  if (!strcmp(code, "hardware")) return "The IR hardware didn't start";
  if (!strcmp(code, "storage")) return "The stick's storage isn't available";
  if (!strcmp(code, "exists")) return "That name is taken";
  return "Couldn't do that";
}

static protocol::Result failCode(const char* code) {
  return fail(code, message(code), !strcmp(code, "notfound") ? 404 : !strcmp(code, "full") ? 409 : 400);
}

protocol::Result run(const protocol::Args& arg) {
  const String op = arg("op");
  store::begin();
  const String dev = arg("dev"), btn = arg("btn");
  char id[irnames::DEVICE_LEN + 1] = "", name[irnames::BUTTON_LEN + 1] = "";
  const bool hasId = irnames::deviceId(dev.c_str(), id), hasName = irnames::buttonName(btn.c_str(), name);

  if (op == "" || op == "list") {
    if (!dev.length()) return ok(deviceListJson());
    irdoc::Device d;
    if (!hasId || !store::load(id, d)) return failCode("notfound");
    const int page = max(0L, arg("page").toInt());
    String j = "\"dev\":" + jsonStr(id) + ",\"buttons\":[";
    const int from = page * PAGE, to = min<int>(from + PAGE, d.buttons.size());
    char p[16];
    for (int i = from; i < to; i++) {
      const auto& b = d.buttons[i];
      const size_t c = b.code.find(':');
      snprintf(p, sizeof p, "%s", b.code.substr(0, c).c_str());
      j += String(i > from ? "," : "") + "{\"n\":" + jsonStr(b.name.c_str()) + ",\"p\":" + jsonStr(p) +
           (b.fav ? ",\"f\":1" : "") + "}";
    }
    return ok(j + "],\"page\":" + page + ",\"total\":" + d.buttons.size() +
              ",\"more\":" + ((int)d.buttons.size() > to ? "true" : "false"));
  }
  if (op == "get") {
    irdoc::Device d;
    int at;
    if (!hasId || !hasName || !store::load(id, d) || (at = d.find(name)) < 0) return failCode("notfound");
    return ok("\"dev\":" + jsonStr(id) + ",\"btn\":" + jsonStr(name) + ",\"c\":" +
              jsonStr(d.buttons[at].code.c_str()) + (d.buttons[at].fav ? ",\"f\":1" : ""));
  }
  if (op == "send") {
    const int repeat = arg("repeat").toInt();
    const String code = arg("code");
    const char* why = code.length() ? sendCode(code, repeat, false) : sendButton(dev.c_str(), btn.c_str(), repeat, false);
    if (why) return failCode(why);
    return ok(code.length() ? String("\"sent\":\"code\"") : "\"sent\":" + jsonStr(name) + ",\"dev\":" + jsonStr(id));
  }
  if (op == "learn") {
    const String o = arg("once");
    const char* why = startLearn(dev.c_str(), btn.c_str(), o == "1" || o == "true");
    if (why) return failCode(why);
    return ok("\"learning\":true,\"dev\":" + jsonStr(view_.dev) + ",\"btn\":" + jsonStr(view_.btn));
  }
  if (op == "listen") {
    const char* why = startListen(arg("secs").length() ? arg("secs").toInt() : 10);
    if (why) return failCode(why);
    return ok("\"listening\":true");
  }
  if (op == "cancel") {
    const bool was = view_.active;
    cancelLearn("agent");
    return ok(String("\"cancelled\":") + (was ? "true" : "false"));
  }
  if (op == "put") {
    if (!hasId) return failCode("name");
    irdoc::Device d;
    if (!store::load(id, d)) d.id = id;
    const char* why = nullptr;
    const String fav = arg("fav");
    const int at = hasName ? d.find(name) : -1;
    const bool f = fav.length() ? (fav == "1" || fav == "true") : (at >= 0 && d.buttons[at].fav);
    if (!d.put(btn.c_str(), arg("code").c_str(), f, why) || !store::save(d, why)) return failCode(why ? why : "write");
    return ok("\"dev\":" + jsonStr(id) + ",\"btn\":" + jsonStr(name));
  }
  if (op == "rm") {
    if (!hasId || !store::exists(id)) return failCode("notfound");
    if (!btn.length()) {
      store::remove(id);
      return ok("\"removed\":" + jsonStr(id));
    }
    irdoc::Device d;
    const char* why = nullptr;
    if (!store::load(id, d) || !hasName || !d.remove(name)) return failCode("notfound");
    if (!store::save(d, why)) return failCode(why);
    return ok("\"removed\":" + jsonStr(name));
  }
  if (op == "rename") {
    const String to = arg("to");
    const char* why = nullptr;
    if (!hasId) return failCode("notfound");
    if (!btn.length()) {
      char toId[irnames::DEVICE_LEN + 1];
      if (!irnames::deviceId(to.c_str(), toId)) return failCode("name");
      if (!store::rename(id, toId, why)) return failCode(why);
      return ok("\"dev\":" + jsonStr(toId));
    }
    irdoc::Device d;
    if (!store::load(id, d)) return failCode("notfound");
    if (!d.rename(name, to.c_str(), why) || !store::save(d, why)) return failCode(why);
    char toName[irnames::BUTTON_LEN + 1];
    irnames::buttonName(to.c_str(), toName);
    return ok("\"btn\":" + jsonStr(toName));
  }
  if (op == "fav") {
    irdoc::Device d;
    int at;
    const char* why = nullptr;
    if (!hasId || !hasName || !store::load(id, d) || (at = d.find(name)) < 0) return failCode("notfound");
    const String on = arg("on");
    d.buttons[at].fav = on.length() ? (on == "1" || on == "true") : !d.buttons[at].fav;
    if (!store::save(d, why)) return failCode(why);
    return ok(String("\"fav\":") + (d.buttons[at].fav ? "true" : "false"));
  }
  if (op == "diag") {
    const int tx = rmt::pinLevel(TX_PIN), rx = rmt::pinLevel(RX_PIN);
    // a receiver module idles high, the LED driver's input low
    const char* guess = rx == 1 && tx == 0 ? "ok" : rx == 0 && tx == 1 ? "swapped" : "unknown";
    char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
    Log.printf("IR diag: TX G%d idle %d, RX G%d idle %d -> %s\n", (int)TX_PIN, tx, (int)RX_PIN, rx, guess);
    return ok(String("\"tx\":") + (int)TX_PIN + ",\"rx\":" + (int)RX_PIN + ",\"txIdle\":" + tx + ",\"rxIdle\":" + rx +
              ",\"pins\":\"" + guess + "\",\"rmtTx\":\"" + rmt::txState() + "\",\"rmtRx\":\"" + rmt::rxState() +
              "\",\"devices\":" + store::list(ids, irnames::MAX_DEVICES) + ",\"free\":" + store::freeKB() +
              ",\"hint\":\"rxIdle should be 1 (a receiver idles high); then '@ir listen' and press a remote\"");
  }
  return fail("op", "Unknown @ir op: " + op, 404);
}

void control(JsonObjectConst o) {
  // {"learn":{"dev","btn","once"}}, {"send":{"dev","btn","repeat"} | {"code"}}, {"cancel":1}, ...:
  // the same ops as "@ir", answered with {"evt":"ir","op":...,"reply":{...}}
  for (JsonPairConst kv : o) {
    const String op = kv.key().c_str();
    JsonObjectConst args = kv.value().as<JsonObjectConst>();
    protocol::Result r = run([&](const char* n) -> String {
      if (!strcmp(n, "op")) return op;
      JsonVariantConst v = args[n];
      if (v.isNull()) return String();
      if (v.is<bool>()) return v.as<bool>() ? "1" : "0";
      return v.as<String>();
    });
    String out = "{\"src\":\"agent\",\"evt\":\"ir\",\"op\":" + jsonStr(op) + ",\"reply\":" + r.json + "}";
    buddy::sendLine(out);
    return;  // one op per message
  }
}

String stateJson() {
  char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
  return String("\"ir\":{\"v\":1,\"n\":") + store::list(ids, irnames::MAX_DEVICES) + ",\"tx\":" + (int)TX_PIN +
         ",\"rx\":" + (int)RX_PIN + "}";
}

void begin() {
  store::begin();
  txRaw = (uint16_t*)bigBuf(ircodec::MAX_RAW * sizeof(uint16_t));
  frame = (uint16_t*)bigBuf(ircodec::MAX_RAW * sizeof(uint16_t));
  capBuf = (uint16_t*)bigBuf(ircodec::MAX_RAW * sizeof(uint16_t));
  codeBuf = (char*)bigBuf(CODE_MAX);
  learnerP = new (bigBuf(sizeof(irlearn::Learner))) irlearn::Learner();
}

void loop() {
  txLoop();
  learnLoop();
}

}  // namespace ir
