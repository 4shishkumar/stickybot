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
#include "launcher_session.h"
#include "events.h"
#include "ota.h"
#include "faces/faces.h"
#include "faces/pkg/install.h"
#include "ir/ir.h"
#include "logic/reader.h"
#include "logic/hosts.h"
#include "mic.h"
#include "config.h"
#include <mutex>
#include <atomic>

namespace buddy {

static const char* NUS_SVC = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
static const char* NUS_RX  = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";  // desktop -> stick
static const char* NUS_TX  = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";  // stick -> desktop
static const char* AUDIO   = "6e400004-b5a3-f393-e0a9-e50e24dcca9e";  // stick -> agent: ADPCM frames
static const char* AUDIO_IN = "6e400005-b5a3-f393-e0a9-e50e24dcca9e"; // agent -> stick: sound clips
static const char* OTA_DATA = "6e400006-b5a3-f393-e0a9-e50e24dcca9e"; // agent -> stick: firmware image (1.8.0+), face packages (2.4.0+)

constexpr uint32_t LINK_TIMEOUT_MS = 30000;
constexpr uint32_t AGENT_TIMEOUT_MS = 25000;  // agent sends a hello every 10 s
constexpr size_t   MAX_LINE = 4096;
constexpr size_t   CHUNK = 180;  // stays under the usual macOS MTU

static NimBLECharacteristic* tx = nullptr;
static NimBLECharacteristic* audio = nullptr;
static Agent        agent_, lastAnswer_;
static uint32_t     agentSeen = 0, agentAt = 0;
struct Inbox { uint16_t handle = hosts::NONE; std::string bytes; hosts::Lines lines; bool overflow = false; uint32_t epoch = 0; };
static Inbox inboxes[hosts::CAPACITY];
static std::recursive_mutex peersMutex;
static hosts::Registry peers;
static hosts::Transfer bulk;
static hosts::Lease speakerOwner, micOwner, irOwner;
static uint32_t speakerSeen = 0;
static uint16_t requestPeer = hosts::NONE;
static uint16_t buttonHost = hosts::NONE;  // negotiated by the selected peer on this connection
static bool shared = false;
static std::string bondKey(const NimBLEConnInfo& info) {
  auto address = info.getIdAddress();
  return address.toString() + "/" + std::to_string(address.getType());
}
static bool hostsChanged = false;
struct HostPrompts { Prompt cli, desktop; bool hasCli = false, hasDesktop = false; uint32_t seen = 0; String answered; };
static HostPrompts hostPrompts[hosts::CAPACITY];
static String focusRequested;
static hosts::Lease focusRequester;
static uint32_t focusRequestedAt = 0;
static Prompt focusPrompt;
static std::atomic<bool> cancelOwned{false};
static uint32_t inboxEpoch = 0;
struct EventRoute { String local, original; hosts::Lease owner; };
static EventRoute eventRoutes[8];
static uint32_t eventSerial = 0;
static uint16_t targetHandle() { return requestPeer != hosts::NONE ? requestPeer : hid::selectedHost(); }
static bool peerOwns(hosts::Lease owner, uint16_t handle) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  return peers.owns(owner, handle);
}
static bool occupied() {
  return mic::live() || speakerOwner.handle != hosts::NONE || sound::busy() || ota::active() || faces::pkg::install::receiving() || ir::learning() || ir::sending() ||
      agent_.state == "thinking" || agent_.state == "working" || agent_.state == "listening";
}
bool hostBusy() {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  return occupied();
}
static void clearActiveView();
static void cancelResources();
static void publishHosts();
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
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo& info) override {
    if (!info.isEncrypted()) return;
    std::string v = c->getValue();
    if (v.empty()) return;
    std::lock_guard<std::recursive_mutex> lock(peersMutex);
    if (shared) {
      uint16_t h = info.getConnHandle();
      if (h != peers.selected() || peers.byHandle(h) < 0 || !peers.valid(h, peers.peers[peers.byHandle(h)].session)) return;
      if ((uint8_t)v[0] == 0) {
        if ((speakerOwner.handle != hosts::NONE && !peers.owns(speakerOwner,h)) || mic::live() || ota::active()) return;
        speakerOwner = peers.lease(h);
      } else if (!peers.owns(speakerOwner,h)) return;
    }
    speakerSeen = millis();
    sound::clipPacket((const uint8_t*)v.data(), v.size());
  }
};

class OtaCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo& info) override {
    std::lock_guard<std::recursive_mutex> lock(peersMutex);
    if (!info.isEncrypted() || (shared && !peerOwns(bulk.owner,info.getConnHandle()))) return;
    std::string v = c->getValue();
    if (faces::pkg::install::receiving()) faces::pkg::install::onPacket((const uint8_t*)v.data(), v.size());
    else ota::onPacket((const uint8_t*)v.data(), v.size());
  }
};

class RxCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* c, NimBLEConnInfo& info) override {
    if (!info.isEncrypted()) return;
    std::string v = c->getValue();
    std::lock_guard<std::recursive_mutex> lock(peersMutex);
    Inbox* in = nullptr;
    for (auto& i : inboxes) if (i.handle == info.getConnHandle()) { in = &i; break; }
    if (!in) for (auto& i : inboxes) if (i.handle == hosts::NONE) { i.handle = info.getConnHandle(); i.epoch = ++inboxEpoch; in = &i; break; }
    if (!in) return;
    if (in->bytes.size() + v.size() <= MAX_LINE * 2) in->bytes += v;
    else { in->bytes.clear(); in->overflow = true; }
  }
};

static void send(const String& json, bool log = true) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  uint16_t handle = targetHandle();
  if (!tx || handle == hosts::NONE) return;
  NimBLEServer* server = NimBLEDevice::getServer();
  if (!server || !server->getPeerInfoByHandle(handle).isEncrypted()) return;
  String s = json + "\n";
  for (size_t i = 0; i < s.length(); i += CHUNK) {
    String part = s.substring(i, min(s.length(), i + CHUNK));
    // A burst of notifications can run out of Bluetooth buffers: retry (up to 300 ms; a
    // dropped part would garble the line). Spaced, so a long reply leaves buffers for keys.
    for (int tries = 0; tries < 75 && !tx->notify((const uint8_t*)part.c_str(), part.length(), handle); tries++) delay(4);
    if (s.length() > CHUNK) delay(2);
  }
  if (log && !shared) Log.event(diagnostics::Event::Link);
}

static void hostError(const char* code, const char* message) {
  JsonDocument d;
  d["src"] = "agent"; d["evt"] = "host_error"; d["error"] = code; d["message"] = message;
  String out; serializeJson(d,out); send(out,false);
}
static void protoReply(JsonVariantConst id, const String& result) {
  JsonDocument out; out["src"] = "agent"; out["evt"] = "proto"; out["id"] = id;
  JsonDocument reply; deserializeJson(reply,result); out["reply"] = reply.as<JsonVariant>();
  String line; serializeJson(out,line); send(line,false);
}

static void ack(const char* what, bool ok = true) {
  send(String("{\"ack\":\"") + what + "\",\"ok\":" + (ok ? "true" : "false") + "}");
}

void attach() {
  shared = settings::sharedHosts();
  peers.active = settings::activeHost().c_str();
  for (size_t i = 0; i < hosts::CAPACITY; ++i) {
    JsonDocument d;
    if (!deserializeJson(d, settings::hostRecord(i))) {
      auto& p = peers.peers[i];
      p.address = d["address"] | ""; p.id = d["id"] | "";
      p.name = d["name"] | ""; p.label = d["label"] | "";
    }
  }
  NimBLEServer* server = NimBLEDevice::getServer();
  NimBLEDevice::stopAdvertising();
  NimBLEService* svc = server->createService(NUS_SVC);
  NimBLECharacteristic* rx = svc->createCharacteristic(
      NUS_RX, NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_NR | NIMBLE_PROPERTY::WRITE_ENC);
  tx = svc->createCharacteristic(NUS_TX, NIMBLE_PROPERTY::NOTIFY);
  audio = svc->createCharacteristic(AUDIO, NIMBLE_PROPERTY::NOTIFY);
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
  Log.event(diagnostics::Event::Link);
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
  if (s["on"].is<bool>()) sound::setOn(s["on"].as<bool>());
  if (s["volume"].is<int>()) sound::setVolume(constrain(s["volume"].as<int>(), 0, 255));
  if (s["play"].is<const char*>()) sound::play(String(s["play"].as<const char*>()));
  sendSounds();  // reply with the current settings
}

static bool wmic_ = false;    // the agent's wireless Wispr mic is on and has its output device
static bool wready_ = false;  // ... and it answers "prepare" with "ready" (makes the stick-mic-bluetooth device)
static volatile bool readyPending_ = false;
static bool busyPending_ = false;
static uint16_t busyId_ = 0;
static uint16_t readyId_ = 0;

// Plugged into a computer by USB: the agent must not make the Bluetooth mic then.
// Sent when it changes, to a new agent, and every 20 s with the hellos.
static bool usbMounted_ = false;
static uint32_t usbSentAt_ = 0;
static bool usbSent_ = false;

static void sendUsb() {
  if (shared) usbMounted_ = false;
  send(String("{\"src\":\"agent\",\"evt\":\"usb\",\"mounted\":") + (usbMounted_ ? "true" : "false") + "}");
  usbSent_ = true;
  usbSentAt_ = millis();
}

static reader::Assembly reply_;          // the full reply, in parts after "done"
static bool     speaks_ = false;          // the agent reads replies aloud on request (2.6.0 agents)
static bool     speaking_ = false;
static uint32_t speakAskedAt_ = 0;        // asked on the stick, the agent hasn't said "on" yet
static uint32_t viewChanges_ = 0;

static void clearActiveView() {
  buttonHost = hosts::NONE;
  agentSeen = agentAt = lastHeard = 0;
  wmic_ = wready_ = speaks_ = speaking_ = false;
  readyPending_ = false;
  agent_ = {}; lastAnswer_ = {}; home_ = {}; wx_ = {};
  reply_.begin(0, 0);
  running_ = waiting_ = total_ = 0; tokensToday_ = 0; lastMsg_ = "";
  usbSent_ = false; ++viewChanges_;
}

String deviceId() {
  // The eFuse identity is independent of the slot-specific advertising address.
  uint64_t id = ESP.getEfuseMac();
  char out[24];
  snprintf(out, sizeof(out), "stick-%012llx", (unsigned long long)(id & 0xffffffffffffULL));
  return out;
}
String hostState() {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  JsonDocument d;
  d["src"] = "agent"; d["evt"] = "hosts"; d["shared"] = shared;
  d["active"] = peers.active; d["deviceId"] = deviceId(); d["board"] = STICK_BOARD;
  d["multiHost"] = 1; d["maxHosts"] = hosts::CAPACITY;
  JsonArray a = d["hosts"].to<JsonArray>();
  for (size_t i = 0; i < hosts::CAPACITY; ++i) {
    const auto& p = peers.peers[i]; if (p.id.empty()) continue;
    JsonObject h = a.add<JsonObject>();
    h["id"] = p.id; h["name"] = p.name; h["label"] = p.label;
    h["display"] = peers.display(i);
    h["online"] = p.handle != hosts::NONE; h["app"] = peers.online(i, millis());
    h["active"] = peers.active == p.id;
  }
  String out; serializeJson(d, out); return out;
}
static void publishHosts() {
  String out = hostState(); uint16_t old = requestPeer;
  for (auto& p : peers.peers) if (p.handle != hosts::NONE && !p.session.empty()) {
    requestPeer = p.handle; send(out, false);
  }
  requestPeer = old;
}
String activeHostLabel() {
  if (!shared) return "";
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  int i = peers.byId(peers.active);
  return String(peers.display(i).c_str()) + (i >= 0 && peers.peers[i].handle == hosts::NONE ? " (off)" : "");
}
String activeHostName() {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  int i = peers.byId(peers.active);
  return i < 0 ? String("No Mac has introduced itself yet") : String(peers.peers[i].name.c_str());
}
String macPassword() {
  if (!shared) return settings::macPassword();
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  int i = peers.byId(peers.active);
  return i < 0 ? String() : settings::hostPassword(i);
}
bool setMacPassword(const String& value) {
  if (!shared) { settings::saveMacPassword(value); return true; }
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  int i = peers.byId(peers.active);
  if (i < 0 || (requestPeer != hosts::NONE && requestPeer != peers.selected())) return false;
  settings::saveHostPassword(i, value); return true;
}
static bool selectHost(const String& id) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (!peers.select(id.c_str(), occupied())) return false;
  // The keyboard driver releases every held report on the old handle first.
  hid::selectHost(peers.selected());
  settings::saveActiveHost(id);
  clearActiveView(); hostsChanged = true;
  return true;
}
bool nextHost() {
  if (!shared) return false;
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  int at = peers.byId(peers.active);
  for (int n = 1; n <= (int)hosts::CAPACITY; ++n) {
    int i = (at + n) % hosts::CAPACITY;
    if (peers.peers[i].handle != hosts::NONE && selectHost(peers.peers[i].id.c_str())) return true;
  }
  return false;
}
void peerDisconnected(uint16_t handle) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (shared && (handle == peers.selected() || handle == bulk.owner.handle ||
      handle == speakerOwner.handle || handle == micOwner.handle)) cancelOwned = true;
  if (buttonHost == handle) buttonHost = hosts::NONE;
  peers.disconnect(handle);
  for (auto& in : inboxes) if (in.handle == handle) in = {};
  hostsChanged = true;
}
bool forgetHosts(const String& id) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (occupied() || (id != "all" && peers.byId(id.c_str()) < 0)) return false;
  for (size_t i = 0; i < hosts::CAPACITY; ++i) {
    if (id != "all" && peers.peers[i].id != id.c_str()) continue;
    peers.forget(peers.peers[i].id);
    settings::saveHostRecord(i, "");
    settings::saveHostPassword(i, "");
    hostPrompts[i] = {};
  }
  settings::saveActiveHost(peers.active.c_str());
  // The caller restarts after acknowledging: release input before reconnecting the bonds.
  hid::selectHost(hosts::NONE);
  clearActiveView();
  return true;
}
static void persistPeer(int i) {
  const auto& p = peers.peers[i]; JsonDocument d;
  d["address"] = p.address; d["id"] = p.id; d["name"] = p.name; d["label"] = p.label;
  String out; serializeJson(d, out); settings::saveHostRecord(i, out);
}

static void handleAgent(JsonDocument& d) {
  // Paused or quitting on the Mac (2.6.0): the Bluetooth link stays up for the keyboard, so
  // without this the stick would only notice when the hellos stop (AGENT_TIMEOUT_MS).
  if (d["bye"] == true) {
    if (agentSeen) Log.event(diagnostics::Event::Link);
    buttonHost = hosts::NONE;
    agentSeen = 0;
    wmic_ = wready_ = false;
    speaking_ = false;
    speakAskedAt_ = 0;
    viewChanges_++;
    return;
  }
  bool foreground = !shared || requestPeer == peers.selected();
  bool fresh = foreground && !agentLinked();  // a new agent (or it was gone): tell it where we are
  if (fresh) buttonHost = hosts::NONE;
  if (foreground) agentSeen = millis();
  if (fresh) viewChanges_++;  // back (Resume): the screens show it at once
  if (d["reply"].is<JsonObject>()) {  // a part of the full reply (after "done")
    JsonObject r = d["reply"];
    reader::Assembly::Step st = reply_.add(r["rid"] | 0u, r["i"] | -1, r["t"] | "");
    if (st == reader::Assembly::Step::COMPLETE) {
      Log.event(diagnostics::Event::Reply);
      Log.count(diagnostics::Counter::ReplyBytes, reply_.text().size());
      viewChanges_++;
    } else if (st == reader::Assembly::Step::BROKEN) {
      Log.event(diagnostics::Event::Reply);
      Log.count(diagnostics::Counter::ReplyBytes, reply_.text().size());
    }
    return;
  }
  if (d["speech"].is<const char*>()) {  // reading aloud started / stopped (any speech, not just ours)
    bool on = d["speech"] == "on";
    if (on != speaking_) viewChanges_++;
    speaking_ = on;
    speakAskedAt_ = 0;
    return;
  }
  if (d["wmic"]["state"] == "busy") {
    busyId_ = d["wmic"]["id"] | 0; busyPending_ = true; return;
  }
  if (d["wispr"] == "ready") {  // the mic device is there: the key can go down
    readyId_ = d["id"] | 0;
    readyPending_ = true;
    return;
  }
  if (d["ota"].is<JsonObject>()) {
    if (shared && !bulk.claim(peers, requestPeer, d["ota"]["begin"].is<JsonObject>(), ota::active())) {
      hostError("transfer_owner", "Reconnect the Mac that started this update and begin it again to resume");
      return;
    }
    ota::control(d["ota"].as<JsonObjectConst>());
    return;
  }
  if (d["faces"].is<JsonObject>()) {
    if (shared && !bulk.claim(peers, requestPeer, d["faces"]["put"].is<JsonObject>(), faces::pkg::install::receiving())) {
      hostError("transfer_owner", "Reconnect the Mac that started this install and begin it again to resume");
      return;
    }
    faces::pkg::install::control(d["faces"].as<JsonObjectConst>());
    return;
  }
  if (d["ir"].is<JsonObject>()) {  // the IR remote: learn, send, cancel... (ir/ir.cpp)
    if (shared) irOwner = peers.lease(requestPeer);
    ir::control(d["ir"].as<JsonObjectConst>());
    return;
  }
  if (foreground && (fresh || !usbSent_ || millis() - usbSentAt_ > 20000)) sendUsb();
  if (d["wmic"].is<bool>()) {  // in every hello, and on its own when changed in the dashboard
    bool on = d["wmic"].as<bool>();
    if (on != wmic_) Log.event(diagnostics::Event::Mic);
    wmic_ = on;
  }
  if (d["hello"].is<int>()) {
    if (foreground) buttonHost = d["buttonActions"] == true ? requestPeer : hosts::NONE;
    wready_ = d["wready"] | false;  // agents before 1.7.1 don't send it
    speaks_ = d["speak"] | false;   // ... before 2.6.0
  }
  if (d["proto"].is<const char*>()) {  // a settings command, same as "@..." over USB
    String line = d["proto"].as<String>();
    line.trim();
    if (shared && line.startsWith("@ir")) irOwner = peers.lease(requestPeer);
    // handle() runs inside buddy::loop under the peer lease lock, never in the BLE callback.
    // Also check the HID destination in legacy mode, where a second connected Mac has no host session.
    const bool selected = foreground && requestPeer != hosts::NONE && requestPeer == hid::selectedHost();
    const auto source = selected ? protocol::Source::Bluetooth : protocol::Source::BackgroundBluetooth;
    protocol::Result r = protocol::runLine(line, source, selected ? launcher::openFromAgent : nullptr);
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
    if (shared) {
      int i = peers.byHandle(requestPeer);
      String local = "h" + String(++eventSerial);
      eventRoutes[eventSerial % 8] = {local, ev.id, peers.lease(requestPeer)};
      ev.id = local;
      ev.title = String(peers.display(i).c_str()) + ": " + ev.title;
    }
    ev.sub = e["sub"] | "";
    ev.icon = e["icon"] | "star";
    ev.sound = e["sound"] | "";
    events::push(ev);
    faces::onEventKind(e["kind"] | "");  // the faces react too (agents since faces send the kind)
    if (!shared) Log.event(diagnostics::Event::Event);
    return;
  }
  if (d["world"].is<JsonObject>()) faces::onWorld(d["world"].as<JsonObjectConst>());  // the faces' data
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
  if (agent_.state == "done") {
    lastAnswer_ = agent_;  // errors don't replace the last answer
    reply_.begin(d["rid"] | 0u, d["parts"] | 0);  // the full reply follows in this many parts
  }
  agentAt = millis();
}

static void handle(const String& l) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  JsonDocument d;
  if (deserializeJson(d, l)) return;
  NimBLEServer* server = NimBLEDevice::getServer();
  if (!server || requestPeer == hosts::NONE) return;
  NimBLEConnInfo info = server->getPeerInfoByHandle(requestPeer);
  if (!info.isEncrypted()) return;
  int index = peers.byHandle(requestPeer);
  bool hello = d["src"] == "agent" && d["hello"].is<int>() && d["host"].is<JsonObject>();
  if (hello && shared) {
    JsonObject h = d["host"];
    std::lock_guard<std::recursive_mutex> lock(peersMutex);
    auto old = peers.lease(requestPeer);
    index = peers.hello(requestPeer, bondKey(info), info.isBonded(),
        h["id"] | "", h["name"] | "", h["label"] | "", h["session"] | "", millis());
    if (index < 0) { hostError("identity_or_capacity", "Mac identity is invalid, already paired elsewhere, or all three hosts are saved"); return; }
    if (old.generation != peers.lease(requestPeer).generation || old.handle != requestPeer) {
      hostPrompts[index] = {};
      if (shared && (bulk.owner.handle == requestPeer || micOwner.handle == requestPeer || speakerOwner.handle == requestPeer || irOwner.handle == requestPeer)) { cancelOwned = true; cancelResources(); }
      if (shared && peers.active == peers.peers[index].id) clearActiveView();
    }
    persistPeer(index);
    if (shared) for (Prompt* prompt : {&hostPrompts[index].cli, &hostPrompts[index].desktop}) {
      if (!prompt->id.length()) continue;
      int sep = prompt->from.indexOf(" / ");
      prompt->from = String(peers.display(index).c_str()) + (sep < 0 ? String() : prompt->from.substring(sep));
    }
    if (peers.active.empty()) { peers.active = peers.peers[index].id; settings::saveActiveHost(peers.active.c_str()); }
    if (shared) hid::selectHost(peers.selected());
    hostsChanged = true; ++viewChanges_;
    send(hostState(), false);
    if (shared) {
      JsonDocument clean; clean["src"] = "agent";
      for (const char* key : {"hello", "wmic", "wready", "speak", "buttonActions", "t", "tz", "wx", "home", "world"})
        if (!d[key].isNull()) clean[key] = d[key];
      d = clean;
    }
  }
  if (shared) {
    String proto = d["proto"] | ""; proto.trim();
    bool discovery = d["src"] == "agent" && (proto == "@state" || proto == "@hosts" || proto == "@keys");
    if (discovery && !hello) {
      protocol::Result r = protocol::runLine(proto, protocol::Source::Bluetooth);
      protoReply(d["id"],r.json); return;
    }
    if (!hello && !peers.valid(requestPeer, d["hs"] | "")) {
      send("{\"src\":\"agent\",\"evt\":\"host_error\",\"error\":\"session_required\",\"message\":\"Update the Mac app or use legacy mode\"}",false);
      return;
    }
    if (index >= 0) {
      peers.peers[index].seen = millis();
      if (d["bye"] == true) {
        peers.retire(requestPeer);
        hostPrompts[index] = {}; hostsChanged = true;
        if (requestPeer == peers.selected()) { clearActiveView(); cancelOwned = true; }
        return;
      }
    }
    if (d["hosts"]["select"].is<const char*>()) {
      String wanted = d["hosts"]["select"].as<String>();
      if (focusRequested.length() || peers.byId(wanted.c_str()) < 0 || occupied()) {
        hostError("busy_or_unknown", "Finish the current task or pending confirmation before choosing a connected Mac"); return;
      }
      focusRequested = wanted; focusRequester = peers.lease(requestPeer); focusRequestedAt = millis();
      focusPrompt = {"host-focus-" + wanted, "Switch input Mac?", String("Use ") + peers.display(peers.byId(wanted.c_str())).c_str(),
          String("From ") + peers.display(index).c_str()};
      send("{\"src\":\"agent\",\"evt\":\"host_focus\",\"state\":\"confirm_on_stick\"}"); return;
    }
    if (d["src"] == "agent" && requestPeer != peers.selected()) {
      // Read-only settings and bounded notifications are independent of input focus.
      if (d["event"].is<JsonObject>()) {
        JsonDocument isolated; isolated["event"] = d["event"];
        handleAgent(isolated);
      } else if (d["proto"].is<const char*>()) {
        protoReply(d["id"], "{\"ok\":false,\"error\":\"Select this Mac on the stick before changing settings\"}");
      } else if (d["ota"].is<JsonObject>() || d["faces"].is<JsonObject>() ||
                 d["ir"].is<JsonObject>() || d["sound"].is<JsonObject>()) {
        hostError("not_active", "Select this Mac on the stick before starting this operation");
      }
      return;
    }
    if (d["src"] == "agent" && (d["ota"].is<JsonObject>() || d["faces"].is<JsonObject>())) {
      if ((d["ota"].is<JsonObject>() && faces::pkg::install::receiving()) ||
          (d["faces"].is<JsonObject>() && ota::active())) {
        hostError("busy", "Another transfer is using the stick"); return;
      }
      if (mic::live() || sound::busy()) {
        hostError("busy", "Finish recording or playback before starting a transfer"); return;
      }
    }
    if (d["src"] != "agent" && index < 0) return;
  }
  if (d["src"] == "agent") {
    if (!shared && d["state"].is<const char*>()) Log.event(diagnostics::Event::Link);
    handleAgent(d);
    return;
  }
  if (!shared) Log.event(diagnostics::Event::Link);
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
  HostPrompts* hp = shared ? &hostPrompts[index] : nullptr;
  Prompt& slot = hp ? (fromCli ? hp->cli : hp->desktop) : (fromCli ? cli : desktop);
  bool& has = hp ? (fromCli ? hp->hasCli : hp->hasDesktop) : (fromCli ? hasCli : hasDesktop);
  String& answered = hp ? hp->answered : answeredId;
  if (hp) hp->seen = millis();
  if (d["clear"].is<const char*>()) {
    if (slot.id == d["clear"].as<String>()) has = false;
    return;
  }

  if (!fromCli && (!shared || requestPeer == peers.selected())) {  // desktop heartbeat snapshot
    running_ = d["running"] | 0;
    waiting_ = d["waiting"] | 0;
    total_ = d["total"] | 0;
    tokensToday_ = d["tokens_today"] | 0L;
    if (d["msg"].is<const char*>()) lastMsg_ = d["msg"].as<String>();
  } else {
    cliSeen = millis();
  }
  if (!p.isNull() && p["id"].as<String>() != answered) {
    slot = {p["id"].as<String>(), p["tool"].as<String>(), p["hint"].as<String>(),
            fromCli ? (p["from"] | "terminal") : ""};
    if (shared) slot.from = String(peers.display(index).c_str()) + (slot.from.length() ? " / " + slot.from : "");
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

static void cancelResources() {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (cancelOwned.exchange(false)) {
    mic::agentEnd(); mic::wisprEnd(); sound::cancelClip();
    if (ir::learning()) ir::cancelLearn("disconnected");
    // Keep the transfer receiver and its verified offset for the existing three-minute
    // reconnect window. A matching bonded host must send a new begin to renew its lease.
    bulk.revoke();
    speakerOwner = micOwner = irOwner = {};
    clearActiveView();
  }
}
void loop() {
  cancelResources();
  for (auto& in : inboxes) {
    std::string chunk;
    uint16_t handle; uint32_t epoch;
    {
      std::lock_guard<std::recursive_mutex> lock(peersMutex);
      handle = in.handle; epoch = in.epoch;
      chunk.swap(in.bytes);
      if (in.overflow) { in.lines.partial.clear(); in.lines.dropping = true; in.overflow = false; }
    }
    if (handle == hosts::NONE) continue;
    for (char c : chunk) {
      std::lock_guard<std::recursive_mutex> lock(peersMutex);
      if (in.handle != handle || in.epoch != epoch) break;
      std::string complete;
      if (in.lines.push(c, complete)) {
        requestPeer = handle;
        if (handle == in.handle) buddy::handle(String(complete.c_str()));
        requestPeer = hosts::NONE;
      }
    }
  }
  cancelResources();
  if (shared) {
    std::lock_guard<std::recursive_mutex> lock(peersMutex);
    auto* server = NimBLEDevice::getServer();
    if (server) for (auto h : server->getPeerDevices()) {
      auto info = server->getPeerInfoByHandle(h);
      if (info.isEncrypted() && info.isBonded()) peers.reconnect(h, bondKey(info));
    }
    hid::selectHost(peers.selected());
    int active = peers.byId(peers.active);
    if (agentSeen && !peers.online(active,millis())) {
      cancelOwned = true; cancelResources(); hostsChanged = true;
    }
    if (speakerOwner.handle != hosts::NONE && !sound::busy() && millis() - speakerSeen > 3000) {
      sound::cancelClip(); speakerOwner = {};
    }
    if (!ir::learning() && !ir::sending()) irOwner = {};
    if (hostsChanged) { hostsChanged = false; publishHosts(); }
    if (focusRequested.length() && (millis() - focusRequestedAt > 30000 || !peers.owns(focusRequester,focusRequester.handle))) focusRequested = "";
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
static int promptHost() {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  for (size_t i = 0; i < hosts::CAPACITY; ++i) {
    auto& p = hostPrompts[i];
    if (!peers.online(i,millis()) || millis() - p.seen > CLI_PROMPT_TTL_MS) { p.hasCli = p.hasDesktop = false; }
    if (p.hasCli || p.hasDesktop) return (int)i;
  }
  return -1;
}
bool hasPrompt() {
  if (shared) return focusRequested.length() || promptHost() >= 0;
  if (hasCli && millis() - cliSeen > CLI_PROMPT_TTL_MS) hasCli = false;
  return hasCli || hasDesktop;
}
const Prompt& prompt() {
  if (shared) {
    if (focusRequested.length()) return focusPrompt;
    int i = promptHost();
    if (i >= 0) return hostPrompts[i].hasCli ? hostPrompts[i].cli : hostPrompts[i].desktop;
  }
  return hasCli ? cli : desktop;
}
void answer(const char* decision) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (!hasPrompt()) return;
  if (shared && focusRequested.length()) {
    bool ok = String(decision) != "deny" && selectHost(focusRequested);
    uint16_t old = requestPeer;
    if (peers.owns(focusRequester,focusRequester.handle)) {
      requestPeer = focusRequester.handle;
      send(String("{\"src\":\"agent\",\"evt\":\"host_focus\",\"state\":\"") + (ok ? "selected" : "denied") + "\"}");
    }
    requestPeer = old; focusRequested = ""; return;
  }
  int i = shared ? promptHost() : -1;
  HostPrompts* hp = i >= 0 ? &hostPrompts[i] : nullptr;
  Prompt& p = hp ? (hp->hasCli ? hp->cli : hp->desktop) : (hasCli ? cli : desktop);
  uint16_t old = requestPeer;
  if (hp) requestPeer = peers.peers[i].handle;
  JsonDocument d; d["cmd"] = "permission"; d["id"] = p.id; d["decision"] = decision;
  String out; serializeJson(d, out); send(out);
  requestPeer = old;
  if (hp) { hp->answered = p.id; (hp->hasCli ? hp->hasCli : hp->hasDesktop) = false; }
  else { answeredId = p.id; (hasCli ? hasCli : hasDesktop) = false; }
}

void decide(bool approve) { answer(approve ? "once" : "deny"); }

// --- Mac agent ---------------------------------------------------------------

// Local time from the agent's clock; formatting is in logic/clock.cpp (unit-tested).
static bool localNow(clockfmt::Local& t) { return clockfmt::local((int64_t)time(nullptr), tzOffset, t); }

bool localTime(clockfmt::Local& t) { return localNow(t); }

String clock() {
  clockfmt::Local t;
  if (!localNow(t)) return String();
  return String(settings::clock24() ? clockfmt::time24(t).c_str() : clockfmt::time12(t).c_str());
}

int hour() {
  clockfmt::Local t;
  return localNow(t) ? t.hour : -1;
}

int minuteOfDay() {
  clockfmt::Local t;
  return localNow(t) ? t.hour * 60 + t.min : -1;
}

String longDate() {
  clockfmt::Local t;
  return localNow(t) ? String(clockfmt::longDate(t).c_str()) : String();
}

String ampm() {
  clockfmt::Local t;
  return localNow(t) && !settings::clock24() ? String(clockfmt::ampm(t).c_str()) : String();
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
  return targetHandle();
}

void eventShown(const String& id) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  uint16_t old = requestPeer;
  String original = id;
  if (shared) {
    EventRoute* route = nullptr;
    for (auto& e : eventRoutes) if (e.local == id) { route = &e; break; }
    if (!route || !peers.owns(route->owner,route->owner.handle)) return;
    requestPeer = route->owner.handle; original = route->original; *route = {};
  }
  JsonDocument d;
  d["src"] = "agent";
  d["evt"] = "event_shown";
  d["id"] = original;
  String out;
  serializeJson(d, out);
  send(out);
  requestPeer = old;
}

void fastLink() {
  // A quick connection interval (15-30 ms, within Apple's accessory
  // guidelines); keyboards normally idle slower.
  uint16_t h = connHandle();
  if (h != BLE_HS_CONN_HANDLE_NONE) NimBLEDevice::getServer()->updateConnParams(h, 12, 24, 0, 400);
}

void sendLine(const String& json, bool log) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  uint16_t old = requestPeer;
  if (shared && requestPeer == hosts::NONE) {
    JsonDocument d;
    if (deserializeJson(d,json)) return;
    String evt = d["evt"] | "";
    hosts::Lease owner = (evt == "ota" || evt == "faces") ? bulk.owner :
        evt == "ir" ? irOwner : peers.lease(hid::selectedHost());
    if (!peers.owns(owner,owner.handle)) return;
    requestPeer = owner.handle;
  }
  send(json, log); requestPeer = old;
}

void fastData() {
  fastLink();
  uint16_t h = connHandle();
  if (h == BLE_HS_CONN_HANDLE_NONE) return;
  NimBLEServer* server = NimBLEDevice::getServer();
  server->setDataLen(h, 251);  // long link-layer packets (usually on already)
  server->updatePhy(h, BLE_GAP_LE_PHY_2M_MASK, BLE_GAP_LE_PHY_2M_MASK, 0);  // 2M PHY if the Mac agrees
}

bool buttonAction(const char* gesture, const char* action) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (requestPeer != hosts::NONE || !agentLinked() || buttonHost == hosts::NONE ||
      buttonHost != hid::selectedHost()) return false;
  JsonDocument d;
  d["src"] = "agent"; d["evt"] = "button_action";
  d["gesture"] = gesture; d["action"] = action;
  String line; serializeJson(d, line); send(line);
  return true;
}

void agentCancel() { send("{\"src\":\"agent\",\"evt\":\"cancel\"}"); }

const std::string& replyFull() {
  static const std::string none;
  return reply_.complete() ? reply_.text() : none;
}

bool agentSpeaks() { return speaks_ && agentLinked(); }

void agentSpeak(bool start) {
  send(start ? "{\"src\":\"agent\",\"evt\":\"speak\",\"what\":\"last\"}"
             : "{\"src\":\"agent\",\"evt\":\"speak\",\"what\":\"stop\"}");
  speaking_ = start;  // shown at once; the agent's "speech" says how it went
  speakAskedAt_ = start ? millis() : 0;
  viewChanges_++;
}

bool agentSpeaking() {
  if (speaking_ && speakAskedAt_ && millis() - speakAskedAt_ > 8000) {  // asked, never started
    speaking_ = false;
    speakAskedAt_ = 0;
    viewChanges_++;
  }
  return speaking_ && agentLinked();
}

uint32_t viewChanges() {
  static bool linked = false;  // the hellos stopped (no bye: an older agent, a crash)
  if (agentLinked() != linked) {
    linked = !linked;
    viewChanges_++;
  }
  return viewChanges_;
}

void agentTalkEvent(bool start) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (shared && !start && !peers.owns(micOwner,micOwner.handle)) return;
  if (start) { if (shared) micOwner = peers.lease(hid::selectedHost()); fastLink(); }
  send(start ? "{\"src\":\"agent\",\"evt\":\"talk\",\"state\":\"start\",\"rate\":16000,\"codec\":\"ima-adpcm\"}"
             : "{\"src\":\"agent\",\"evt\":\"talk\",\"state\":\"stop\"}");
}

bool agentWirelessMic() { return wmic_ && agentLinked(); }
bool agentPreparesMic() { return wready_ && agentWirelessMic(); }

void wisprEvent(bool start) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (shared && !start && !peers.owns(micOwner,micOwner.handle)) return;
  if (start) { if (shared) micOwner = peers.lease(hid::selectedHost()); fastLink(); }
  send(start ? "{\"src\":\"agent\",\"evt\":\"wispr\",\"state\":\"start\",\"rate\":16000,\"codec\":\"ima-adpcm\"}"
             : "{\"src\":\"agent\",\"evt\":\"wispr\",\"state\":\"stop\"}");
}

void wisprPrepare(uint16_t id) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (shared) micOwner = peers.lease(hid::selectedHost());
  fastLink();
  readyPending_ = false;
  send(String("{\"src\":\"agent\",\"evt\":\"wispr\",\"state\":\"prepare\",\"id\":") + id +
       ",\"rate\":16000,\"codec\":\"ima-adpcm\"}");
}

bool takeWisprBusy(uint16_t& id) {
  if (!busyPending_) return false;
  busyPending_ = false; id = busyId_; return true;
}
bool takeWisprReady(uint16_t& id) {
  if (!readyPending_) return false;
  readyPending_ = false;
  id = readyId_;
  return true;
}

void setUsbMounted(bool on) {
  if (shared) on = false;
  if (on == usbMounted_) return;
  usbMounted_ = on;
  Log.event(diagnostics::Event::Link);
  if (agentLinked()) sendUsb();
  else usbSent_ = false;
}

size_t audioPayloadMax() {
  uint16_t h = shared ? micOwner.handle : connHandle();
  if (h == BLE_HS_CONN_HANDLE_NONE) return 20;
  uint16_t mtu = NimBLEDevice::getServer()->getPeerMTU(h);
  return mtu > 3 ? mtu - 3 : 20;
}

String linkInfo() {
  uint16_t h = connHandle();
  if (h == BLE_HS_CONN_HANDLE_NONE) return "down";
  NimBLEConnInfo info = NimBLEDevice::getServer()->getPeerInfoByHandle(h);
  return "MTU " + String(info.getMTU()) + ", " + String(info.getConnInterval() * 1.25f, 1) + " ms";
}

bool sendAudio(const uint8_t* data, size_t len) {
  std::lock_guard<std::recursive_mutex> lock(peersMutex);
  if (!audio) return true;  // nobody to send to: drop
  uint16_t h = shared ? micOwner.handle : connHandle();
  if (h == hosts::NONE || (shared && !peerOwns(micOwner,h))) return true;
  return audio->notify(data, len, h);  // its own buffer: no shared value between frames
}

}  // namespace buddy
