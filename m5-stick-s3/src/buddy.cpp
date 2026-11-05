#include "buddy.h"
#include "netlog.h"
#include <NimBLEDevice.h>
#include <ArduinoJson.h>
#include <M5Unified.h>
#include "settings.h"
#include "battery.h"
#include "hid.h"
#include <sys/time.h>
#include "logic/clock.h"
#include "sound.h"
#include "protocol.h"
#include "events.h"
#include "ota.h"

namespace buddy {

static const char* NUS_SVC = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
static const char* NUS_RX  = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";  // desktop -> stick
static const char* NUS_TX  = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";  // stick -> desktop
static const char* AUDIO   = "6e400004-b5a3-f393-e0a9-e50e24dcca9e";  // stick -> agent: ADPCM frames
static const char* AUDIO_IN = "6e400005-b5a3-f393-e0a9-e50e24dcca9e"; // agent -> stick: sound clips
static const char* OTA_DATA = "6e400006-b5a3-f393-e0a9-e50e24dcca9e"; // agent -> stick: firmware image (1.8.0+)

constexpr uint32_t LINK_TIMEOUT_MS = 30000;
constexpr uint32_t AGENT_TIMEOUT_MS = 25000;  // agent sends a hello every 10 s
constexpr size_t   MAX_LINE = 4096;
constexpr size_t   CHUNK = 180;  // stays under the usual macOS MTU

static NimBLECharacteristic* tx = nullptr;
static NimBLECharacteristic* audio = nullptr;
static Agent        agent_, lastAnswer_;
static uint32_t     agentSeen = 0, agentAt = 0;
static portMUX_TYPE mux = portMUX_INITIALIZER_UNLOCKED;
static std::string  inbox;  // filled by the BLE task, drained in loop()
static String       line;
static uint32_t     lastHeard = 0, lastAdvCheck = 0;
static int          running_ = 0, waiting_ = 0, total_ = 0;
static long         tokensToday_ = 0;
static String       lastMsg_;
constexpr uint32_t CLI_PROMPT_TTL_MS = 30000;  // hook re-sends every few seconds

static Prompt       desktop, cli;
static bool         hasDesktop = false, hasCli = false;
static uint32_t     cliSeen = 0;
static String       answeredId;

class AudioInCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    std::string v = c->getValue();
    sound::clipPacket((const uint8_t*)v.data(), v.size());
  }
};

// Firmware image packets; WRITE_ENC already needs the encrypted (paired) link
class OtaCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo& info) override {
    if (!info.isEncrypted()) return;
    std::string v = c->getValue();
    ota::onPacket((const uint8_t*)v.data(), v.size());
  }
};

class RxCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo&) override {
    std::string v = c->getValue();
    portENTER_CRITICAL(&mux);
    if (inbox.size() < MAX_LINE * 2) inbox += v;
    portEXIT_CRITICAL(&mux);
  }
};

static void send(const String& json, bool log = true) {
  if (!tx) return;
  String s = json + "\n";
  for (size_t i = 0; i < s.length(); i += CHUNK) {
    String part = s.substring(i, min(s.length(), i + CHUNK));
    // A burst of notifications can run out of Bluetooth buffers: retry briefly
    for (int tries = 0; tries < 25 && !tx->notify((const uint8_t*)part.c_str(), part.length()); tries++) delay(4);
  }
  if (log) Log.printf("buddy> %s\n", json.c_str());
}

static void ack(const char* what, bool ok = true) {
  send(String("{\"ack\":\"") + what + "\",\"ok\":" + (ok ? "true" : "false") + "}");
}

void attach() {
  NimBLEServer* server = NimBLEDevice::getServer();
  NimBLEDevice::stopAdvertising();
  NimBLEService* svc = server->createService(NUS_SVC);
  NimBLECharacteristic* rx = svc->createCharacteristic(
      NUS_RX, NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_NR | NIMBLE_PROPERTY::WRITE_ENC);
  tx = svc->createCharacteristic(NUS_TX, NIMBLE_PROPERTY::NOTIFY | NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::READ_ENC);
  audio = svc->createCharacteristic(AUDIO, NIMBLE_PROPERTY::NOTIFY | NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::READ_ENC);
  NimBLECharacteristic* audioIn = svc->createCharacteristic(
      AUDIO_IN, NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_NR | NIMBLE_PROPERTY::WRITE_ENC);
  static AudioInCallbacks audioInCb;
  audioIn->setCallbacks(&audioInCb);
  NimBLECharacteristic* otaData = svc->createCharacteristic(
      OTA_DATA, NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_NR | NIMBLE_PROPERTY::WRITE_ENC);
  static OtaCallbacks otaCb;
  otaData->setCallbacks(&otaCb);
  static RxCallbacks rxCb;
  rx->setCallbacks(&rxCb);
  server->start();  // re-registers all services (keyboard + buddy)

  // The desktop app's device list filters on the buddy service UUID in the
  // advertisement. A packet holds 31 bytes, so rebuild it: flags (3) +
  // keyboard appearance (4) + HID service (4) + buddy service (18) = 29.
  // (The library's battery UUID and preferred-interval hint are dropped from
  // the advert only; both still work.)
  NimBLEAdvertising* adv = NimBLEDevice::getAdvertising();
  NimBLEAdvertisementData ad;
  ad.setFlags(BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP);
  ad.setAppearance(0x03C1);
  ad.addServiceUUID(NimBLEUUID((uint16_t)0x1812));
  ad.addServiceUUID(NimBLEUUID(NUS_SVC));
  adv->setAdvertisementData(ad);

  NimBLEAdvertisementData scan;
  scan.setName(hid::name().c_str());
  adv->setScanResponseData(scan);
  Log.printf("Mac link ready, advertising as \"%s\"\n", hid::name().c_str());
}

static void handleStatus() {
  JsonDocument d;
  d["ack"] = "status";
  d["ok"] = true;
  JsonObject data = d["data"].to<JsonObject>();
  data["name"] = settings::name();
  data["sec"] = hid::connected();
  JsonObject bat = data["bat"].to<JsonObject>();
  bat["pct"] = battery::level();
  bat["mV"] = battery::millivolts();
  bat["usb"] = M5.Power.getVBUSVoltage() > 4000;
  JsonObject sys = data["sys"].to<JsonObject>();
  sys["up"] = millis() / 1000;
  sys["heap"] = ESP.getFreeHeap();
  String out;
  serializeJson(d, out);
  send(out);
}

RTC_DATA_ATTR static long tzOffset = 0;  // seconds east of UTC (survives standby)
static HomeCard home_;
static Weather wx_;


static void sendSounds() {
  JsonDocument r;
  r["src"] = "agent";
  r["evt"] = "sounds";
  sound::describe(r["sounds"].to<JsonObject>());
  String out;
  serializeJson(r, out);
  send(out);
}

// {"src":"agent","sound":{"get":1} | {"set":{event:tune,...}} | {"play":name} | {"on":bool} | {"volume":0-255}}
static void handleSound(JsonObject s) {
  if (s["set"].is<JsonObject>())
    for (JsonPair kv : s["set"].as<JsonObject>()) sound::setTune(kv.key().c_str(), kv.value().as<String>());
  if (s["on"].is<bool>()) settings::saveSoundOn(s["on"].as<bool>());
  if (s["volume"].is<int>()) settings::saveSoundVolume(constrain(s["volume"].as<int>(), 0, 255));
  if (s["play"].is<const char*>()) sound::play(String(s["play"].as<const char*>()));
  sendSounds();  // reply with the current settings
}

static bool wmic_ = false;    // the agent's wireless Wispr mic is on and has its output device
static bool wready_ = false;  // ... and it answers "prepare" with "ready" (makes the stick-mic-bluetooth device)
static volatile bool readyPending_ = false;
static uint16_t readyId_ = 0;

// Plugged into a computer by USB: the agent must not make the Bluetooth mic then.
// Sent when it changes, to a new agent, and every 20 s with the hellos.
static bool usbMounted_ = false;
static uint32_t usbSentAt_ = 0;
static bool usbSent_ = false;

static void sendUsb() {
  send(String("{\"src\":\"agent\",\"evt\":\"usb\",\"mounted\":") + (usbMounted_ ? "true" : "false") + "}");
  usbSent_ = true;
  usbSentAt_ = millis();
}

static void handleAgent(JsonDocument& d) {
  bool fresh = !agentLinked();  // a new agent (or it was gone): tell it where we are
  agentSeen = millis();
  if (d["wispr"] == "ready") {  // the mic device is there: the key can go down
    readyId_ = d["id"] | 0;
    readyPending_ = true;
    return;
  }
  if (d["ota"].is<JsonObject>()) {  // firmware update over this link (ota.cpp)
    ota::control(d["ota"].as<JsonObjectConst>());
    return;
  }
  if (fresh || !usbSent_ || millis() - usbSentAt_ > 20000) sendUsb();
  if (d["wmic"].is<bool>()) {  // in every hello, and on its own when changed in the dashboard
    bool on = d["wmic"].as<bool>();
    if (on != wmic_) Log.printf("Agent: wireless Wispr mic %s\n", on ? "on" : "off");
    wmic_ = on;
  }
  if (d["hello"].is<int>()) wready_ = d["wready"] | false;  // agents before 1.7.1 don't send it
  if (d["proto"].is<const char*>()) {  // a settings command, same as "@..." over USB
    String line = d["proto"].as<String>();
    line.trim();
    protocol::Result r = protocol::runLine(line);
    String id = d["id"] | "";
    String esc;
    for (char c : id) if (c != '"' && c != '\\') esc += c;  // ids are short tokens
    send("{\"src\":\"agent\",\"evt\":\"proto\",\"id\":\"" + esc + "\",\"reply\":" +
         (r.json.length() ? r.json : String("null")) + "}");
    protocol::finish(r);  // restarts after e.g. a name or slot change
    return;
  }
  if (d["sound"].is<JsonObject>()) {
    handleSound(d["sound"].as<JsonObject>());
    return;
  }
  if (d["event"].is<JsonObject>()) {  // celebrate / nice / headsup screen
    JsonObject e = d["event"];
    events::Event ev;
    if (!events::parseLevel(e["level"] | "", ev.level)) return;
    ev.id = e["id"] | "";
    ev.title = e["title"] | "";
    ev.sub = e["sub"] | "";
    ev.icon = e["icon"] | "star";
    ev.sound = e["sound"] | "";
    events::push(ev);
    Log.printf("event (%s): %s\n", e["level"] | "", ev.title.c_str());
    return;
  }
  if (d["t"].is<long>()) {  // the hello carries the Mac's clock
    timeval tv{(time_t)d["t"].as<long>(), 0};
    settimeofday(&tv, nullptr);
    tzOffset = d["tz"] | 0L;
  }
  if (d["wx"].is<JsonObject>()) {
    JsonObject w = d["wx"];
    wx_.valid = true;
    wx_.temp = w["t"] | 0;
    wx_.hi = w["hi"] | 0;
    wx_.lo = w["lo"] | 0;
    wx_.icon = w["i"] | "cloudy";
    wx_.cond = w["c"] | "";
    wx_.day = (w["day"] | 1) != 0;
    wx_.place = w["at"] | "";
    wx_.at = millis();
  }
  if (d["home"].is<JsonObject>()) {
    JsonObject h = d["home"];
    home_.kind = h["k"] | "";
    home_.a = h["a"] | "";
    home_.b = h["b"] | "";
    home_.urgent = h["u"] | false;
    home_.index = h["i"] | 0;
    home_.count = h["n"] | 0;
  }
  if (!d["state"].is<const char*>()) return;  // just a hello
  agent_.state = d["state"].as<String>();
  agent_.heard = d["heard"] | "";
  agent_.text = d["text"] | "";
  if (agent_.state == "done") lastAnswer_ = agent_;  // errors don't replace the last answer
  agentAt = millis();
}

static void handle(const String& l) {
  JsonDocument d;
  if (deserializeJson(d, l)) return;
  if (d["src"] == "agent") {
    if (d["state"].is<const char*>()) Log.printf("agent< %s\n", l.c_str());
    handleAgent(d);
    return;
  }
  Log.printf("buddy< %s\n", l.c_str());
  lastHeard = millis();

  if (d["cmd"].is<const char*>()) {
    String cmd = d["cmd"].as<String>();
    if (cmd == "status") handleStatus();
    else if (cmd == "unpair") ack("unpair");  // deliberately keeps the keyboard pairing
    else ack(cmd.c_str());                    // name, owner: nothing to store yet
    return;
  }
  if (d["time"].is<JsonArray>() || d["evt"].is<const char*>()) return;

  JsonObject p = d["prompt"];
  bool fromCli = d["src"] == "cli";
  Prompt& slot = fromCli ? cli : desktop;
  bool& has = fromCli ? hasCli : hasDesktop;

  if (!fromCli) {  // desktop heartbeat snapshot
    running_ = d["running"] | 0;
    waiting_ = d["waiting"] | 0;
    total_ = d["total"] | 0;
    tokensToday_ = d["tokens_today"] | 0L;
    if (d["msg"].is<const char*>()) lastMsg_ = d["msg"].as<String>();
  } else {
    cliSeen = millis();
  }
  if (!p.isNull() && p["id"].as<String>() != answeredId) {
    slot = {p["id"].as<String>(), p["tool"].as<String>(), p["hint"].as<String>(),
            fromCli ? (p["from"] | "terminal") : ""};
    slot.ask = fromCli && p["kind"] == "ask";
    if (slot.ask) {
      slot.q = p["q"] | "";
      slot.a = p["a"] | "";
      slot.b = p["b"] | "";
    }
    has = true;
  } else if (p.isNull()) {
    has = false;  // answered elsewhere (e.g. on the Mac) or withdrawn
  }
}

void loop() {
  std::string chunk;
  portENTER_CRITICAL(&mux);
  chunk.swap(inbox);
  portEXIT_CRITICAL(&mux);
  for (char c : chunk) {
    if (c == '\n') {
      if (line.length()) handle(line);
      line = "";
    } else if (line.length() < MAX_LINE) {
      line += c;
    }
  }

  // Keep advertising while connected as a keyboard, so the desktop app (and
  // the Claude Code hook) can still find us.
  if (millis() - lastAdvCheck > 3000) {
    lastAdvCheck = millis();
    NimBLEAdvertising* adv = NimBLEDevice::getAdvertising();
    if (hid::state() == hid::Bt::CONNECTED && !adv->isAdvertising()) NimBLEDevice::startAdvertising();
  }
}

bool linked()  { return lastHeard && millis() - lastHeard < LINK_TIMEOUT_MS; }
int  running() { return running_; }
int  waiting() { return waiting_; }
int  total()   { return total_; }
long tokensToday() { return tokensToday_; }
String lastMessage() { return lastMsg_; }
bool hasPrompt() {
  if (hasCli && millis() - cliSeen > CLI_PROMPT_TTL_MS) hasCli = false;  // hook gone
  return hasCli || hasDesktop;
}

// Terminal prompts first: the desktop app also shows its own on screen.
const Prompt& prompt() { return hasCli ? cli : desktop; }

void answer(const char* decision) {
  if (!hasPrompt()) return;
  Prompt& p = hasCli ? cli : desktop;
  send("{\"cmd\":\"permission\",\"id\":\"" + p.id + "\",\"decision\":\"" + decision + "\"}");
  answeredId = p.id;
  (hasCli ? hasCli : hasDesktop) = false;
}

void decide(bool approve) { answer(approve ? "once" : "deny"); }

// --- Mac agent ---------------------------------------------------------------

// Local time from the agent's clock; formatting is in logic/clock.cpp (unit-tested).
static bool localNow(clockfmt::Local& t) { return clockfmt::local((int64_t)time(nullptr), tzOffset, t); }

String clock() {
  clockfmt::Local t;
  return localNow(t) ? String(clockfmt::time12(t).c_str()) : String();
}

int hour() {
  clockfmt::Local t;
  return localNow(t) ? t.hour : -1;
}

String longDate() {
  clockfmt::Local t;
  return localNow(t) ? String(clockfmt::longDate(t).c_str()) : String();
}

String ampm() {
  clockfmt::Local t;
  return localNow(t) ? String(clockfmt::ampm(t).c_str()) : String();
}

String date() {
  clockfmt::Local t;
  return localNow(t) ? String(clockfmt::shortDate(t).c_str()) : String();
}

HomeCard homeCard() { return agentLinked() ? home_ : HomeCard{}; }
const Weather& weather() {
  if (wx_.valid && millis() - wx_.at > 2 * 3600 * 1000UL) wx_.valid = false;  // agent gone for a while
  return wx_;
}



bool agentLinked() { return agentSeen && millis() - agentSeen < AGENT_TIMEOUT_MS; }
const Agent& agent() { return agent_; }
const Agent& lastAnswer() { return lastAnswer_; }
uint32_t agentUpdated() { return agentAt; }

static uint16_t connHandle() {
  NimBLEServer* server = NimBLEDevice::getServer();
  if (!server || server->getConnectedCount() == 0) return BLE_HS_CONN_HANDLE_NONE;
  return server->getPeerInfo(0).getConnHandle();
}

void eventShown(const String& id) {
  JsonDocument d;
  d["src"] = "agent";
  d["evt"] = "event_shown";
  d["id"] = id;
  String out;
  serializeJson(d, out);
  send(out);
}

void fastLink() {
  // A quick connection interval (15-30 ms, within Apple's accessory
  // guidelines); keyboards normally idle slower.
  uint16_t h = connHandle();
  if (h != BLE_HS_CONN_HANDLE_NONE) NimBLEDevice::getServer()->updateConnParams(h, 12, 24, 0, 400);
}

void sendLine(const String& json, bool log) { send(json, log); }

void fastData() {
  fastLink();
  uint16_t h = connHandle();
  if (h == BLE_HS_CONN_HANDLE_NONE) return;
  NimBLEServer* server = NimBLEDevice::getServer();
  server->setDataLen(h, 251);  // long link-layer packets (usually on already)
  server->updatePhy(h, BLE_GAP_LE_PHY_2M_MASK, BLE_GAP_LE_PHY_2M_MASK, 0);  // 2M PHY if the Mac agrees
}

void agentCancel() { send("{\"src\":\"agent\",\"evt\":\"cancel\"}"); }

void agentTalkEvent(bool start) {
  if (start) fastLink();
  send(start ? "{\"src\":\"agent\",\"evt\":\"talk\",\"state\":\"start\",\"rate\":16000,\"codec\":\"ima-adpcm\"}"
             : "{\"src\":\"agent\",\"evt\":\"talk\",\"state\":\"stop\"}");
}

bool agentWirelessMic() { return wmic_ && agentLinked(); }
bool agentPreparesMic() { return wready_ && agentWirelessMic(); }

void wisprEvent(bool start) {
  if (start) fastLink();
  send(start ? "{\"src\":\"agent\",\"evt\":\"wispr\",\"state\":\"start\",\"rate\":16000,\"codec\":\"ima-adpcm\"}"
             : "{\"src\":\"agent\",\"evt\":\"wispr\",\"state\":\"stop\"}");
}

void wisprPrepare(uint16_t id) {
  fastLink();
  readyPending_ = false;
  send(String("{\"src\":\"agent\",\"evt\":\"wispr\",\"state\":\"prepare\",\"id\":") + id +
       ",\"rate\":16000,\"codec\":\"ima-adpcm\"}");
}

bool takeWisprReady(uint16_t& id) {
  if (!readyPending_) return false;
  readyPending_ = false;
  id = readyId_;
  return true;
}

void setUsbMounted(bool on) {
  if (on == usbMounted_) return;
  usbMounted_ = on;
  Log.printf("USB host %s\n", on ? "connected" : "gone");
  if (agentLinked()) sendUsb();
  else usbSent_ = false;
}

size_t audioPayloadMax() {
  uint16_t h = connHandle();
  if (h == BLE_HS_CONN_HANDLE_NONE) return 20;
  uint16_t mtu = NimBLEDevice::getServer()->getPeerMTU(h);
  return mtu > 3 ? mtu - 3 : 20;
}

String linkInfo() {
  uint16_t h = connHandle();
  if (h == BLE_HS_CONN_HANDLE_NONE) return "down";
  NimBLEConnInfo info = NimBLEDevice::getServer()->getPeerInfo(0);
  return "MTU " + String(info.getMTU()) + ", " + String(info.getConnInterval() * 1.25f, 1) + " ms";
}

bool sendAudio(const uint8_t* data, size_t len) {
  if (!audio || connHandle() == BLE_HS_CONN_HANDLE_NONE) return true;  // nobody to send to: drop
  return audio->notify(data, len);  // its own buffer: no shared value between frames
}

}  // namespace buddy
