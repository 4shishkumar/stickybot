#pragma once
#include "Arduino.h"
#include <deque>
#include <memory>

using wifi_mode_t = unsigned;
constexpr wifi_mode_t WIFI_STA = 1, WIFI_AP = 2;
constexpr int WL_CONNECTED = 3;
struct FakeWiFi {
  wifi_mode_t mode = 0;
  wifi_mode_t getMode() const { return mode; }
  int status() const { return WL_CONNECTED; }
};
inline FakeWiFi WiFi;

struct FakeConnection {
  std::string input, output;
  bool connected = true;
};
class WiFiClient : public Print {
 public:
  std::shared_ptr<FakeConnection> state;
  WiFiClient() = default;
  explicit WiFiClient(std::shared_ptr<FakeConnection> s) : state(std::move(s)) {}
  explicit operator bool() const { return state && state->connected; }
  bool connected() const { return (bool)*this; }
  void stop() { if (state) state->connected = false; }
  size_t available() const { return state ? state->input.size() : 0; }
  int read() { unsigned char c = state->input[0]; state->input.erase(0, 1); return c; }
  using Print::write;
  size_t write(uint8_t c) override { if (connected()) state->output += (char)c; return 1; }
};
class WiFiServer {
 public:
  static inline std::deque<std::shared_ptr<FakeConnection>> pending;
  explicit WiFiServer(uint16_t) {}
  void begin() {}
  void end() { pending.clear(); }
  void setNoDelay(bool) {}
  WiFiClient accept() {
    if (pending.empty()) return {};
    auto connection = pending.front(); pending.pop_front();
    return WiFiClient(connection);
  }
};
