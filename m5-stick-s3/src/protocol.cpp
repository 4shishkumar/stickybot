#include "protocol.h"
#include "netlog.h"
#include "config.h"
#include "settings.h"
#include "hid.h"
#include "web.h"
#include "battery.h"
#include "ota.h"
#include <M5Unified.h>
#include "ui.h"
#include "motion.h"
#include "mic.h"
#include <Preferences.h>
#include "faces/faces.h"
#include "faces/pkg/package.h"

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
  String j = "\"proto\":1,\"board\":\"" STICK_BOARD "\",\"fw\":\"" FW_VERSION "\"";
  j += ",\"name\":" + q(hid::name());
  j += ",\"combo\":" + q(comboToString(c)) + ",\"mods\":" + c.mods + ",\"key\":" + c.key;
  j += String(",\"connected\":") + (hid::connected() ? "true" : "false");
  j += String(",\"bt\":\"") + btName(hid::state()) + "\"";
  j += ",\"battery\":" + String(battery::level()) + ",\"charging\":" + (battery::charging() ? "true" : "false");
  j += ",\"batteryEta\":" + q(battery::etaLabel());
  j += ",\"batteryMv\":" + String(battery::millivolts());
  j += String(",\"motion\":{\"lift\":") + (settings::liftWakes() ? "true" : "false") +
       ",\"face\":" + (settings::faceDownSleeps() ? "true" : "false") +
       ",\"shake\":" + (settings::shakeEscapes() ? "true" : "false") + "}";
  j += ",\"standby\":" + String(settings::standbyMin());
  j += ",\"brightness\":" + String(settings::brightnessPct());
  j += String(",\"autoRotate\":") + (settings::autoRotate() ? "true" : "false");
  j += String(",\"landscape\":") + (settings::landscape() ? "true" : "false");  // 1.9.0
  j += String(",\"clock24\":") + (settings::clock24() ? "true" : "false");      // 2.4.0
  j += String(",\"devMode\":") + (settings::devMode() ? "true" : "false");
  j += ",\"face\":" + q(faces::active().id()) + ",\"faces\":" + faces::json();  // 2.0.0
  j += "," + faces::pkg::stateJson();  // 2.4.0: the face packages (their list: @faces)
  j += String(",\"wirelessMic\":") + (settings::wirelessMic() ? "true" : "false");
  j += String(",\"ota\":1,\"otaPending\":") + (ota::pendingVerify() ? "true" : "false");  // Bluetooth update (1.8.0+)
  j += String(",\"usbMounted\":") + (mic::usbMounted() ? "true" : "false");
  j += String(",\"macPassSet\":") + (settings::macPassword().length() ? "true" : "false");
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
  for (size_t i = 0; i < MOD_COUNT; i++) j += String(i ? "," : "") + "[\"" + MODS[i].name + "\"," + MODS[i].bit + "]";
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
      c.mods = arg("mods").toInt() & combo::MOD_MASK;
      c.key  = arg("key").toInt();
      if (keyIndex(c.key) < 0 || !c.valid()) return fail("Pick a key or at least one modifier");
    }
    settings::saveCombo(c);
    Log.printf("Combo set to %s\n", comboToString(c).c_str());
    if (!hid::needsRestart(c)) return ok("\"combo\":" + q(comboToString(c)));
    // Fn added or removed: the stick's keyboard identity changes (see hid.h)
    Log.println("Keyboard identity changes for Fn, restarting - re-pair on the Mac");
    Result r = ok("\"combo\":" + q(comboToString(c)) + ",\"restarting\":true,\"repair\":true");
    r.restart = true;
    return r;
  }

  if (action == "enter") {
    int ms = arg("delay").toInt();
    if (ms < 0 || ms > 10000) return fail("Delay must be 0-10000 ms (0 = off)");
    settings::saveEnterDelayMs(ms);
    Log.printf("Enter after hold: %s\n", ms ? (String(ms) + " ms").c_str() : "off");
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
    Log.printf("Switching to slot %d, restarting\n", n);
    Result r = ok("\"slot\":" + String(n) + ",\"restarting\":true");
    r.restart = true;
    return r;
  }

  if (action == "disconnect") { hid::disconnect(); Log.println("Bluetooth off"); return ok(); }
  if (action == "reconnect")  { hid::reconnect();  Log.println("Bluetooth on");  return ok(); }

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
    Log.printf("Forgot %s%s\n", which == "all" ? "all slots" : ("slot " + which).c_str(),
                  r.restart ? ", restarting" : "");
    if (r.restart) r.json = "{\"ok\":true,\"restarting\":true}";
    return r;
  }

  if (action == "name") {
    String n = arg("name");
    n.trim();
    if (!settings::saveName(n)) return fail("Name must be 1-20 plain characters");
    Log.printf("Name set to \"%s\", restarting\n", n.c_str());
    Result r = ok("\"name\":" + q(n) + ",\"restarting\":true");
    r.restart = true;
    return r;
  }

  if (action == "wifi") {
    settings::saveWifi(arg("ssid"), arg("pass"));
    Log.printf("Home Wi-Fi saved: %s\n", arg("ssid").c_str());
    return ok();
  }

  if (action == "password") {
    if (!settings::savePassword(arg("password"))) return fail("Password must be 8-63 characters");
    Log.println("Device password changed");
    return ok();
  }

  if (action == "standby") {
    int m = arg("min").toInt();
    if (m < 0 || m > 120) return fail("Standby must be 0-120 minutes (0 = never)");
    settings::saveStandbyMin(m);
    Log.printf("Standby after %d min on battery%s\n", m, m ? "" : " (off)");
    return ok("\"standby\":" + String(m));
  }

  if (action == "motion") {
    auto flag = [&](const char* n, bool cur) { String v = arg(n); return v.length() ? (v == "1" || v == "true") : cur; };
    bool lift = flag("lift", settings::liftWakes()), face = flag("face", settings::faceDownSleeps()),
         shake = flag("shake", settings::shakeEscapes());
    settings::saveMotion(lift, face, shake);
    Log.printf("Motion: lift %s, face-down %s, shake %s\n", lift ? "on" : "off", face ? "on" : "off", shake ? "on" : "off");
    return ok();
  }

  if (action == "macpass") {  // write-only: the password is never sent back
    String pw = arg("pass");
    settings::saveMacPassword(pw);
    Log.println(pw.length() ? "Mac password saved" : "Mac password cleared");
    return ok(String("\"macPassSet\":") + (pw.length() ? "true" : "false"));
  }

  if (action == "devmode") {
    String v = arg("on");
    settings::saveDevMode(v == "1" || v == "true");
    return ok();
  }

  if (action == "autorotate") {
    String v = arg("on");
    settings::saveAutoRotate(v == "1" || v == "true");
    return ok();
  }

  if (action == "landscape") {  // landscape screens when held sideways (with auto-rotate)
    String v = arg("on");
    settings::saveLandscape(v == "1" || v == "true");
    return ok(String("\"landscape\":") + (settings::landscape() ? "true" : "false"));
  }

  if (action == "clock24") {  // the time as 24-hour (2.4.0)
    String v = arg("on");
    settings::saveClock24(v == "1" || v == "true");
    Log.printf("Clock: %s\n", settings::clock24() ? "24-hour" : "12-hour");
    return ok(String("\"clock24\":") + (settings::clock24() ? "true" : "false"));
  }

  if (action == "face") {  // id=<face>: the Home screen's face; demo=<face|all>: a preview on the stick
    String demo = arg("demo"), id = arg("id");
    if (demo.length()) {
      if (!faces::demoStart(demo)) return fail("Unknown face: " + demo);
      Log.printf("Face demo: %s\n", demo.c_str());
      return ok("\"demo\":" + q(demo));
    }
    if (faces::find(id) < 0) return fail("Unknown face: " + id);
    settings::saveFace(id);
    faces::select(id);
    Log.printf("Face: %s\n", id.c_str());
    return ok("\"face\":" + q(id));
  }

  if (action == "faces") {  // the face packages (2.4.0): page=<n> one page of the list; rm=<id|all>
    String rm = arg("rm");
    if (rm.length()) {
      int n = faces::pkg::remove(rm);
      if (!n && rm != "all") return fail("Not installed: " + rm, 404);
      return ok("\"removed\":" + String(n));
    }
    return ok(faces::pkg::listJson(arg("page").toInt()));
  }

  if (action == "wmic") {
    String v = arg("on");
    settings::saveWirelessMic(v == "1" || v == "true");
    Log.printf("Wireless Wispr mic %s\n", settings::wirelessMic() ? "on" : "off");
    return ok(String("\"wirelessMic\":") + (settings::wirelessMic() ? "true" : "false"));
  }

  if (action == "brightness") {
    int p = arg("pct").toInt();
    if (p < 10 || p > 100) return fail("Brightness must be 10-100 %");
    settings::saveBrightnessPct(p);
    ui::setBrightness(p);
    return ok("\"brightness\":" + String(p));
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

void finish(const Result& r) {
  if (r.restart) {
    Serial.flush();
    delay(300);
    ESP.restart();
  }
}

Result runLine(const String& line) {
  if (!line.length() || line[0] != '@') {
    Result r;
    r.status = 400;
    r.json = "{\"ok\":false,\"error\":\"expected @action\"}";
    return r;
  }
  int sp = line.indexOf(' ');
  String action = line.substring(1, sp < 0 ? line.length() : sp);
  String query = sp < 0 ? "" : line.substring(sp + 1);
  return run(action, [&](const char* name) { return queryArg(query, name); });
}

static void runMachine(const String& line) {
  Result r = runLine(line);
  Log.print('@');
  Log.println(r.json);
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
    Log.println("Commands: show | set <combo> | enter off|<ms> | name <text> | slot <1-3> | disconnect | reconnect | "
                   "forget <1-3|all> | web on | web off | standby <min> | brightness <pct> | face <id> | face demo [<id>] | "
                   "screens [landscape|faces]");
    return;
  } else if (cmd == "show") {
    Log.printf("%s | combo %s | %s | Wi-Fi %s %s\n", hid::name().c_str(),
                  comboToString(settings::combo()).c_str(), btName(hid::state()),
                  web::mode(), web::url().c_str());
    bool chgEnabled = false;
    M5.Power.M5pm1.getBatteryCharge(&chgEnabled);
    Log.printf("  battery %d%% (%d mV%s) | %s | standby %u min\n", battery::level(), battery::millivolts(),
                  battery::charging() ? ", charging" : "", battery::etaLabel().c_str(), settings::standbyMin());
    Log.printf("  USB %u mV | charger %s | power source 0x%02x | charger restarts %d\n",
                  M5.Power.getVBUSVoltage(), chgEnabled ? "enabled" : "DISABLED",
                  (unsigned)M5.Power.M5pm1.getPowerSource(), battery::chargerRestarts());
    for (uint8_t n = 1; n <= settings::SLOTS; n++)
      Log.printf("  slot %u: %s%s\n", n, hid::paired(n) ? "paired" : "empty",
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
  else if (cmd == "chg") {  // debug: raw charger status pin for 10 s
    int low = 0, n = 0;
    for (uint32_t t0 = millis(); millis() - t0 < 10000; delay(100), n++)
      if (M5.Power.isCharging() == m5::Power_Class::is_charging_t::is_charging) low++;
    Log.printf("CHRG pin low (charging) in %d of %d samples | USB %u mV | battery %d mV | state: %s\n", low, n,
                  M5.Power.getVBUSVoltage(), (int)M5.Power.getBatteryVoltage(), battery::line().c_str());
    return;
  }
  else if (cmd == "chglog") {  // debug: 10 s timeline of the charger pin + voltages at 20 Hz
    String tl;
    int low = 0, n = 0, flips = 0, vbMin = 9999, vbMax = 0, vaMin = 9999, vaMax = 0;
    bool last = false;
    for (uint32_t t0 = millis(); millis() - t0 < 10000; delay(50), n++) {
      bool c = M5.Power.isCharging() == m5::Power_Class::is_charging_t::is_charging;
      int vb = M5.Power.getVBUSVoltage(), va = M5.Power.getBatteryVoltage();
      if (c) low++;
      if (n && c != last) flips++;
      last = c;
      vbMin = min(vbMin, vb); vbMax = max(vbMax, vb); vaMin = min(vaMin, va); vaMax = max(vaMax, va);
      if (n % 2 == 0) tl += c ? '#' : '.';
    }
    Log.printf("CHG %d%% (%d flips) | USB %d-%d mV | BAT %d-%d mV\n", low * 100 / n, flips, vbMin, vbMax, vaMin, vaMax);
    Log.printf("timeline (# = charging, 100 ms each): %s\n", tl.c_str());
    return;
  }
  else if (cmd == "macpass") {  // macpass <password> | macpass clear
    r = run("macpass", [=](const char* n) { return String(n) == "pass" && rest != "clear" ? rest : String(); });
  }
  else if (cmd == "imurec") {  // calibration: record N s at 10 Hz on the stick, dump later
    static int16_t rec[600][6];
    static int recN = 0;
    Preferences store;  // the recording survives a restart (USB reconnects reset the stick)
    if (rest == "dump" && recN == 0 && store.begin("imurec", true)) {
      recN = store.getBytes("data", rec, sizeof rec) / sizeof rec[0];
      store.end();
    }
    if (rest == "dump") {
      for (int i = 0; i < recN; i++)
        Log.printf("%5.1f a %5.2f %5.2f %5.2f | g %6d %6d %6d\n", i / 10.0f, rec[i][0] / 100.0f, rec[i][1] / 100.0f,
                   rec[i][2] / 100.0f, rec[i][3], rec[i][4], rec[i][5]);
      Log.printf("imurec: %d samples\n", recN);
      return;
    }
    int secs = constrain(rest.toInt() > 0 ? rest.toInt() : 30, 1, 60);
    Log.printf("imurec: recording %d s...\n", secs);
    recN = 0;
    for (; recN < secs * 10; recN++) {
      float ax, ay, az, gx, gy, gz;
      M5.Imu.getAccel(&ax, &ay, &az);
      M5.Imu.getGyro(&gx, &gy, &gz);
      int16_t v[6] = {(int16_t)(ax * 100), (int16_t)(ay * 100), (int16_t)(az * 100), (int16_t)gx, (int16_t)gy, (int16_t)gz};
      memcpy(rec[recN], v, sizeof v);
      delay(100);
    }
    if (store.begin("imurec", false)) {
      store.putBytes("data", rec, recN * sizeof rec[0]);
      store.end();
    }
    Log.println("imurec: done (saved) - 'imurec dump' to read it");
    return;
  }
  else if (cmd == "imulog") {  // calibration: 5 s of accel + gyro at 10 Hz
    for (int i = 0; i < 50; i++) {
      float ax, ay, az, gx, gy, gz;
      M5.Imu.getAccel(&ax, &ay, &az);
      M5.Imu.getGyro(&gx, &gy, &gz);
      Log.printf("a %5.2f %5.2f %5.2f | g %6.0f %6.0f %6.0f\n", ax, ay, az, gx, gy, gz);
      delay(100);
    }
    return;
  }
  else if (cmd == "imu") {  // debug: raw accelerometer reading
    float x, y, z;
    motion::accel(x, y, z);
    Log.printf("accel x %.2f  y %.2f  z %.2f g\n", x, y, z);
    return;
  }
  else if (cmd == "led") {  // test: does the power chip's LED_EN pin drive the green LED?
    M5.Power.M5pm1.setLedEnLevel(rest == "on");
    Log.printf("LED_EN %s\n", rest == "on" ? "high" : "low");
    return;
  }
  else if (cmd == "face") {  // face <id> | face demo [<id>]
    String demo = rest.startsWith("demo") ? rest.substring(4) : String();
    demo.trim();
    if (rest.startsWith("demo") && !demo.length()) demo = "all";
    r = run("face", [=](const char* n) { return String(n) == "demo" ? demo : String(n) == "id" ? rest : String(); });
  }
  else if (cmd == "standby")      r = run("standby", with("min"));
  else if (cmd == "brightness")   r = run("brightness", with("pct"));
  else if (cmd == "web")          r = run(rest == "off" ? "web-off" : "web-on", with(""));
  else {
    Log.println("Unknown command. Type: help");
    return;
  }
  if (r.status != 200) Log.println(r.json);
  finish(r);
}

void serialLine(String line) {
  line.trim();
  if (line.isEmpty()) return;
  if (line[0] == '@') runMachine(line);
  else runHuman(line);
}

}  // namespace protocol
