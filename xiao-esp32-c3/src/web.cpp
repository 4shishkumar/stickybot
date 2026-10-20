#include "web.h"
#include <WiFi.h>
#include <WebServer.h>
#include <uri/UriBraces.h>
#include <ESPmDNS.h>
#include <ArduinoOTA.h>
#include <Update.h>
#include <esp_coexist.h>
#include "config.h"
#include "settings.h"
#include "protocol.h"
#include "web_page.h"

namespace web {

constexpr uint32_t JOIN_MS     = 15000;
constexpr uint32_t IDLE_OFF_MS = 10 * 60 * 1000;  // auto-off after 10 min without requests

// Low transmit power: the page is used from the same desk, and the C3 runs
// hot at full power (very hot without its antenna).
constexpr wifi_power_t TX_POWER = WIFI_POWER_8_5dBm;

enum class State { OFF, JOINING, HOME, HOTSPOT };

// The C3 has one radio for Wi-Fi and Bluetooth; keep the Mac link stable
// while Wi-Fi is on.
static void radioForWifi(bool on) {
  if (on) WiFi.setTxPower(TX_POWER);
  esp_coex_preference_set(on ? ESP_COEX_PREFER_BT : ESP_COEX_PREFER_BALANCE);
}

static WebServer server(80);
static State     state = State::OFF;
static uint32_t  startedAt = 0, lastRequest = 0;
static bool      routesAdded = false, updating = false;

// Every /api/<action> goes through the same handler as USB serial.
static void handleApi() {
  lastRequest = millis();
  String action = server.uri().substring(5);  // after "/api/"
  protocol::Result r = protocol::run(action, [](const char* name) { return server.arg(name); });
  server.send(r.status, "application/json", r.json);
  if (r.restart) {
    delay(300);
    ESP.restart();
  }
}

// OTA by uploading firmware.bin from the page. Protected by the device password.
static void handleUpdateDone() {
  bool ok = !Update.hasError();
  server.send(ok ? 200 : 500, "application/json",
              ok ? "{\"ok\":true,\"restarting\":true}" : "{\"ok\":false,\"error\":\"Update failed\"}");
  if (ok) {
    Serial.println("OTA: update done, restarting");
    delay(500);
    ESP.restart();
  }
}

static void handleUpload() {
  HTTPUpload& up = server.upload();
  lastRequest = millis();
  if (up.status == UPLOAD_FILE_START) {
    if (!server.authenticate("admin", settings::password().c_str())) {
      Serial.println("OTA: wrong password");
      return;
    }
    updating = true;
    Serial.printf("OTA: receiving %s\n", up.filename.c_str());
    Update.begin(UPDATE_SIZE_UNKNOWN);
  } else if (up.status == UPLOAD_FILE_WRITE && updating) {
    Update.write(up.buf, up.currentSize);
  } else if (up.status == UPLOAD_FILE_END && updating) {
    Update.end(true);
    Serial.printf("OTA: %u bytes\n", up.totalSize);
  } else if (up.status == UPLOAD_FILE_ABORTED) {
    Update.abort();
    updating = false;
  }
}

static void beginServer() {
  if (!routesAdded) {
    server.on("/", HTTP_GET, [] {
      lastRequest = millis();
      server.send_P(200, "text/html", WEB_PAGE);
    });
    server.on(UriBraces("/api/{}"), HTTP_ANY, handleApi);
    server.on("/update", HTTP_POST, [] {
      if (!server.authenticate("admin", settings::password().c_str())) return server.requestAuthentication();
      handleUpdateDone();
    }, handleUpload);
    server.onNotFound([] { server.send(404, "text/plain", "Not found"); });
    routesAdded = true;
  }
  server.begin();
  MDNS.begin(HOSTNAME);
  MDNS.addService("http", "tcp", 80);

  // OTA straight from PlatformIO: pio run -e xiao_c3_ota -t upload
  ArduinoOTA.setHostname(HOSTNAME);
  ArduinoOTA.setPassword(settings::password().c_str());
  ArduinoOTA.setMdnsEnabled(false);  // already started above
  ArduinoOTA.onStart([] { Serial.println("OTA: PlatformIO upload started"); });
  ArduinoOTA.onEnd([] { Serial.println("OTA: done, restarting"); });
  ArduinoOTA.begin();
  lastRequest = millis();
}

static void startHotspot() {
  if (WiFi.getMode() & WIFI_STA) WiFi.disconnect(true);
  WiFi.mode(WIFI_AP);
  WiFi.softAP(AP_SSID, settings::password().c_str());
  radioForWifi(true);
  state = State::HOTSPOT;
  beginServer();
  Serial.printf("Web: hotspot %s -> http://%s\n", AP_SSID, WiFi.softAPIP().toString().c_str());
}

void start() {
  if (state != State::OFF) return;
  String ssid = settings::wifiSsid();
  if (ssid.isEmpty()) {
    startHotspot();
    return;
  }
  WiFi.mode(WIFI_STA);
  WiFi.setHostname(HOSTNAME);
  WiFi.begin(ssid.c_str(), settings::wifiPass().c_str());
  radioForWifi(true);
  state = State::JOINING;
  startedAt = millis();
  Serial.printf("Web: joining %s...\n", ssid.c_str());
}

void stop() {
  if (state == State::OFF) return;
  ArduinoOTA.end();
  server.stop();
  MDNS.end();
  if (state == State::HOTSPOT) WiFi.softAPdisconnect(true);
  else WiFi.disconnect(true);
  WiFi.mode(WIFI_OFF);
  radioForWifi(false);
  state = State::OFF;
  Serial.println("Web: off");
}

bool active() { return state != State::OFF; }

const char* mode() {
  switch (state) {
    case State::JOINING: return "joining";
    case State::HOME:    return "home";
    case State::HOTSPOT: return "hotspot";
    default:             return "off";
  }
}

String ssid() {
  if (state == State::HOTSPOT) return AP_SSID;
  if (state != State::OFF) return settings::wifiSsid();
  return "";
}

String url() {
  if (state == State::HOME)    return String("http://") + HOSTNAME + ".local";
  if (state == State::HOTSPOT) return "http://" + WiFi.softAPIP().toString();
  return "";
}

void loop() {
  switch (state) {
    case State::OFF:
      return;
    case State::JOINING:
      if (WiFi.status() == WL_CONNECTED) {
        state = State::HOME;
        beginServer();
        Serial.printf("Web: http://%s.local  (http://%s)\n", HOSTNAME, WiFi.localIP().toString().c_str());
      } else if (millis() - startedAt > JOIN_MS) {
        Serial.println("Web: couldn't join Wi-Fi, starting hotspot");
        startHotspot();
      }
      return;
    case State::HOME:
    case State::HOTSPOT:
      server.handleClient();
      ArduinoOTA.handle();
      if (!updating && millis() - lastRequest > IDLE_OFF_MS) stop();
      return;
  }
}

}  // namespace web
