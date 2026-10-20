#include "protocol.h"
#include "config.h"
#include "settings.h"
#include "hid.h"
#include "web.h"

namespace protocol {

static String esc(const String& s) {
  String o;
  for (char c : s) {
    if (c == '"' || c == '\\') o += '\\';
    o += c;
  }
  return o;
}

static String q(const String& s) { return "\"" + esc(s) + "\""; }

static Result ok(const String& extra = "") {
  return {200, "{\"ok\":true" + (extra.length() ? "," + extra : String()) + "}"};
}

static Result fail(const String& msg, int status = 400) {
  return {status, "{\"ok\":false,\"error\":" + q(msg) + "}"};
}

static const char* btName(hid::Bt b) {
  switch (b) {
    case hid::Bt::CONNECTED: return "connected";
    case hid::Bt::OFF:       return "off";
    default:                 return "advertising";
  }
}

static String stateJson() {
  Combo c = settings::combo();
  String j = "\"proto\":1,\"board\":\"" BOARD_ID "\",\"fw\":\"" FW_VERSION "\"";
  j += ",\"name\":" + q(hid::name());
  j += ",\"combo\":" + q(comboToString(c)) + ",\"mods\":" + c.mods + ",\"key\":" + c.key;
  j += String(",\"connected\":") + (hid::connected() ? "true" : "false");
  j += String(",\"bt\":\"") + btName(hid::state()) + "\"";
  j += ",\"battery\":-1,\"charging\":false";
  j += ",\"enterDelay\":" + String(settings::enterDelayMs());
  j += ",\"slot\":" + String(hid::slot()) + ",\"slots\":[";
  for (uint8_t n = 1; n <= settings::SLOTS; n++)
    j += String(n > 1 ? "," : "") + "{\"n\":" + n + ",\"paired\":" + (hid::paired(n) ? "true" : "false") + "}";
  j += "],\"web\":{\"on\":" + String(web::active() ? "true" : "false");
  j += ",\"mode\":\"" + String(web::mode()) + "\",\"ssid\":" + q(web::ssid());
  j += ",\"url\":" + q(web::url()) + ",\"homeSsid\":" + q(settings::wifiSsid()) + "}";
  return j;
}

static String keysJson() {
  String j = "\"mods\":[";
  for (int i = 0; i < 4; i++) j += String(i ? "," : "") + "[\"" + MODS[i].name + "\"," + MODS[i].bit + "]";
  j += "],\"keys\":[";
  for (size_t i = 0; i < KEY_COUNT; i++) j += String(i ? "," : "") + "[" + q(KEYS[i].name) + "," + KEYS[i].code + "]";
  return j + "]";
}

static int slotArg(const String& s) {
  int n = s.toInt();
  return (n >= 1 && n <= settings::SLOTS) ? n : 0;
}

Result run(const String& action, const Args& arg) {
  if (action == "state") return ok(stateJson());
  if (action == "keys")  return ok(keysJson());

  if (action == "combo") {
    Combo c;
    String text = arg("text");
    if (text.length()) {
      if (!parseCombo(text, c)) return fail("Couldn't understand that combo");
    } else {
      c.mods = arg("mods").toInt() & 0x0F;
      c.key  = arg("key").toInt();
      if (keyIndex(c.key) < 0 || !c.valid()) return fail("Pick a key or at least one modifier");
    }
    settings::saveCombo(c);
    Serial.printf("Combo set to %s\n", comboToString(c).c_str());
    return ok("\"combo\":" + q(comboToString(c)));
  }

  if (action == "enter") {
    int ms = arg("delay").toInt();
    if (ms < 0 || ms > 10000) return fail("Delay must be 0-10000 ms (0 = off)");
    settings::saveEnterDelayMs(ms);
    Serial.printf("Enter after hold: %s\n", ms ? (String(ms) + " ms").c_str() : "off");
    return ok("\"enterDelay\":" + String(ms));
  }

  if (action == "test") {
    if (!hid::connected()) return fail("Not connected to a Mac", 409);
    hid::tap(settings::combo());
    return ok();
  }

  if (action == "slot") {
    int n = slotArg(arg("n"));
    if (!n) return fail("Slot must be 1-3");
    if (n == hid::slot()) return ok("\"slot\":" + String(n));
    settings::saveSlot(n);
    Serial.printf("Switching to slot %d, restarting\n", n);
    Result r = ok("\"slot\":" + String(n) + ",\"restarting\":true");
    r.restart = true;
    return r;
  }

  if (action == "disconnect") { hid::disconnect(); Serial.println("Bluetooth off"); return ok(); }
  if (action == "reconnect")  { hid::reconnect();  Serial.println("Bluetooth on");  return ok(); }

  if (action == "forget") {
    String which = arg("n");
    Result r = ok();
    if (which == "all") {
      hid::forgetAll();
      r.restart = true;
    } else {
      int n = slotArg(which);
      if (!n) return fail("Slot must be 1-3 or all");
      hid::forget(n);
      r.restart = (n == hid::slot());
    }
    Serial.printf("Forgot %s%s\n", which == "all" ? "all slots" : ("slot " + which).c_str(),
                  r.restart ? ", restarting" : "");
    if (r.restart) r.json = "{\"ok\":true,\"restarting\":true}";
    return r;
  }

  if (action == "name") {
    String n = arg("name");
    n.trim();
    if (!settings::saveName(n)) return fail("Name must be 1-20 plain characters");
    Serial.printf("Name set to \"%s\", restarting\n", n.c_str());
    Result r = ok("\"name\":" + q(n) + ",\"restarting\":true");
    r.restart = true;
    return r;
  }

  if (action == "wifi") {
    settings::saveWifi(arg("ssid"), arg("pass"));
    Serial.printf("Home Wi-Fi saved: %s\n", arg("ssid").c_str());
    return ok();
  }

  if (action == "password") {
    if (!settings::savePassword(arg("password"))) return fail("Password must be 8-63 characters");
    Serial.println("Device password changed");
    return ok();
  }

  if (action == "web-on")  { web::start(); return ok(); }
  if (action == "web-off") { web::stop();  return ok(); }

  return fail("Unknown command: " + action, 404);
}

// --- Serial -------------------------------------------------------------------

static String urlDecode(const String& s) {
  String o;
  for (size_t i = 0; i < s.length(); i++) {
    char c = s[i];
    if (c == '+') o += ' ';
    else if (c == '%' && i + 2 < s.length()) {
      o += (char)strtol(s.substring(i + 1, i + 3).c_str(), nullptr, 16);
      i += 2;
    } else o += c;
  }
  return o;
}

// "a=1&b=x%20y" -> value of `name`
static String queryArg(const String& query, const char* name) {
  String key = String(name) + "=";
  int start = 0;
  while (start < (int)query.length()) {
    int amp = query.indexOf('&', start);
    if (amp < 0) amp = query.length();
    if (query.substring(start, amp).startsWith(key))
      return urlDecode(query.substring(start + key.length(), amp));
    start = amp + 1;
  }
  return "";
}

static void finish(const Result& r) {
  if (r.restart) {
    Serial.flush();
    delay(300);
    ESP.restart();
  }
}

static void runMachine(const String& line) {
  int sp = line.indexOf(' ');
  String action = line.substring(1, sp < 0 ? line.length() : sp);
  String query = sp < 0 ? "" : line.substring(sp + 1);
  Result r = run(action, [&](const char* name) { return queryArg(query, name); });
  Serial.print('@');
  Serial.println(r.json);
  finish(r);
}

static void runHuman(const String& line) {
  int sp = line.indexOf(' ');
  String cmd = line.substring(0, sp < 0 ? line.length() : sp);
  String rest = sp < 0 ? "" : line.substring(sp + 1);
  rest.trim();
  auto with = [&](const char* name) { return [=](const char* n) { return String(n) == name ? rest : String(); }; };

  Result r;
  if (cmd == "help") {
    Serial.println("Commands: show | set <combo> | enter off|<ms> | name <text> | slot <1-3> | disconnect | reconnect | "
                   "forget <1-3|all> | web on | web off");
    return;
  } else if (cmd == "show") {
    Serial.printf("%s | combo %s | %s | Wi-Fi %s %s\n", hid::name().c_str(),
                  comboToString(settings::combo()).c_str(), btName(hid::state()),
                  web::mode(), web::url().c_str());
    for (uint8_t n = 1; n <= settings::SLOTS; n++)
      Serial.printf("  slot %u: %s%s\n", n, hid::paired(n) ? "paired" : "empty",
                    n == hid::slot() ? " (active)" : "");
    return;
  } else if (cmd == "set")        r = run("combo", with("text"));
  else if (cmd == "slot")         r = run("slot", with("n"));
  else if (cmd == "name")         r = run("name", with("name"));
  else if (cmd == "enter") {  // enter off | enter 1500
    String ms = rest == "off" ? "0" : rest == "on" ? "1500" : rest;
    r = run("enter", [=](const char* n) { return String(n) == "delay" ? ms : String(); });
  }
  else if (cmd == "forget")       r = run("forget", with("n"));
  else if (cmd == "disconnect")   r = run("disconnect", with(""));
  else if (cmd == "reconnect")    r = run("reconnect", with(""));
  else if (cmd == "web")          r = run(rest == "off" ? "web-off" : "web-on", with(""));
  else {
    Serial.println("Unknown command. Type: help");
    return;
  }
  if (r.status != 200) Serial.println(r.json);
  finish(r);
}

void serialLine(String line) {
  line.trim();
  if (line.isEmpty()) return;
  if (line[0] == '@') runMachine(line);
  else runHuman(line);
}

}  // namespace protocol
