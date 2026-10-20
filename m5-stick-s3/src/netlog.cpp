#include "netlog.h"
#include <WiFi.h>
#include "protocol.h"

TeeLog Log;

namespace netlog {

constexpr int MAX_CLIENTS = 3;

static WiFiServer server(PORT);
static WiFiClient conns[MAX_CLIENTS];
static String     inbox[MAX_CLIENTS];
static bool       listening = false;

static bool wifiUp() {
  wifi_mode_t m = WiFi.getMode();
  return ((m & WIFI_STA) && WiFi.status() == WL_CONNECTED) || (m & WIFI_AP);
}

void send(const uint8_t* buf, size_t len) {
  if (!listening) return;
  for (auto& c : conns)
    if (c && c.connected()) c.write(buf, len);
}

int clients() {
  int n = 0;
  for (auto& c : conns)
    if (c && c.connected()) n++;
  return n;
}

void loop() {
  if (!wifiUp()) {
    if (listening) {
      for (auto& c : conns) c.stop();
      server.end();
      listening = false;
    }
    return;
  }
  if (!listening) {
    server.begin();
    server.setNoDelay(true);
    listening = true;
  }

  if (WiFiClient fresh = server.accept()) {
    int slot = -1;
    for (int i = 0; i < MAX_CLIENTS; i++)
      if (!conns[i] || !conns[i].connected()) { slot = i; break; }
    if (slot < 0) {
      fresh.println("Too many log viewers");
      fresh.stop();
    } else {
      conns[slot] = fresh;
      inbox[slot] = "";
      fresh.println("Connected to the log. Type 'help' for commands.");
    }
  }

  for (int i = 0; i < MAX_CLIENTS; i++) {
    WiFiClient& c = conns[i];
    while (c && c.connected() && c.available()) {
      char ch = c.read();
      if (ch == '\n' || ch == '\r') {
        if (inbox[i].length()) protocol::serialLine(inbox[i]);
        inbox[i] = "";
      } else if (ch >= 0x20 && inbox[i].length() < 300) {
        inbox[i] += ch;
      }
    }
  }
}

}  // namespace netlog

size_t TeeLog::write(uint8_t c) { return write(&c, 1); }

size_t TeeLog::write(const uint8_t* buf, size_t len) {
  Serial.write(buf, len);
  netlog::send(buf, len);
  return len;
}
