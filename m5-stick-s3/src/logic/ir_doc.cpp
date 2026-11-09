#include "ir_doc.h"
#include <ArduinoJson.h>
#include <string.h>
#include "ir_codec.h"

namespace irdoc {

bool validCode(const std::string& code) {
  if (code.size() >= 4096) return false;
  std::vector<uint16_t> scratch(ircodec::MAX_RAW);
  ircodec::Parsed p;
  return ircodec::parse(code.c_str(), p, scratch.data());
}

int Device::find(const std::string& name) const {
  for (size_t i = 0; i < buttons.size(); i++)
    if (buttons[i].name == name) return (int)i;
  return -1;
}

static bool cleanButton(const std::string& in, std::string& out) {
  char b[irnames::BUTTON_LEN + 1];
  if (!irnames::buttonName(in.c_str(), b)) return false;
  out = b;
  return true;
}

bool Device::put(const std::string& name, const std::string& code, bool fav, const char*& why) {
  std::string n;
  if (!cleanButton(name, n)) { why = "name"; return false; }
  if (!validCode(code)) { why = "code"; return false; }
  const int at = find(n);
  if (at >= 0) {
    buttons[at].code = code;
    buttons[at].fav = fav;
    return true;
  }
  if ((int)buttons.size() >= irnames::MAX_BUTTONS) { why = "full"; return false; }
  buttons.push_back({n, code, fav});
  return true;
}

bool Device::rename(const std::string& from, const std::string& to, const char*& why) {
  const int at = find(from);
  if (at < 0) { why = "notfound"; return false; }
  std::string n;
  if (!cleanButton(to, n)) { why = "name"; return false; }
  const int other = find(n);
  if (other >= 0 && other != at) { why = "exists"; return false; }
  buttons[at].name = n;
  return true;
}

bool Device::remove(const std::string& name) {
  const int at = find(name);
  if (at < 0) return false;
  buttons.erase(buttons.begin() + at);
  return true;
}

std::string Device::nextName() const {
  std::vector<const char*> names;
  for (const Button& b : buttons) names.push_back(b.name.c_str());
  return "button " + std::to_string(irnames::nextNumber(names.data(), (int)names.size(), "button"));
}

bool read(const char* json, size_t len, Device& out, const char*& why) {
  out = Device{};
  if (!json || !len || len > irnames::MAX_FILE) { why = "size"; return false; }
  JsonDocument doc;
  if (deserializeJson(doc, json, len)) { why = "json"; return false; }
  if ((doc["v"] | 0) != FORMAT) { why = "format"; return false; }
  char id[irnames::DEVICE_LEN + 1];
  const char* dev = doc["dev"] | "";
  if (!irnames::deviceId(dev, id) || strcmp(id, dev)) { why = "dev"; return false; }
  out.id = id;
  JsonArrayConst list = doc["buttons"].as<JsonArrayConst>();
  for (JsonObjectConst b : list) {
    const char* w = nullptr;
    // a bad button is skipped (the rest of the remote still works); the limit stops the list
    if (!out.put(b["n"] | "", b["c"] | "", (b["f"] | 0) != 0, w) && w && !strcmp(w, "full")) break;
  }
  return true;
}

std::string write(const Device& d) {
  JsonDocument doc;
  doc["v"] = FORMAT;
  doc["dev"] = d.id;
  JsonArray list = doc["buttons"].to<JsonArray>();
  for (const Button& b : d.buttons) {
    JsonObject o = list.add<JsonObject>();
    o["n"] = b.name;
    o["c"] = b.code;
    if (b.fav) o["f"] = 1;
  }
  std::string out;
  serializeJson(doc, out);
  return out;
}

}  // namespace irdoc
