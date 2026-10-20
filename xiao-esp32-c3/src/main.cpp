// BLE macro keyboard — XIAO ESP32-C3
//
// The BOOT button (GPIO9) mirrors a key combo over Bluetooth:
//   button down = combo down, button up = combo up
//   (single press -> once, double -> twice, hold -> held)
//
// Wi-Fi settings page: press RESET, then within 2 s press and hold BOOT for
// 3 s. No keys are sent during that gesture. Any restart turns Wi-Fi off.
//
// Settings (combo, Mac slots, Wi-Fi) are stored in flash and changed from the
// Wi-Fi page, the USB settings page or serial commands — see ../PROTOCOL.md.

#include <Arduino.h>
#include "config.h"
#include "settings.h"
#include "hid.h"
#include "web.h"
#include "protocol.h"

constexpr uint8_t  BUTTON_PIN        = 9;     // BOOT button, active LOW
constexpr uint32_t DEBOUNCE_MS       = 15;
constexpr uint32_t GESTURE_WINDOW_MS = 2000;  // after boot: a press starting now is the Wi-Fi gesture
constexpr uint32_t GESTURE_HOLD_MS   = 3000;
constexpr uint32_t ENTER_MIN_HOLD_MS = 1000;  // "Enter after hold" ignores shorter presses

static bool     lastRaw = false, pressed = false, comboDown = false;
static bool     gesture = false, gestureDone = false;
static uint32_t lastChange = 0, gestureStart = 0, comboSince = 0, enterAt = 0;

static void onPress() {
  uint32_t now = millis();
  if (now < GESTURE_WINDOW_MS) {  // just after boot: Wi-Fi gesture, never keys
    gesture = true;
    gestureDone = false;
    gestureStart = now;
    Serial.println("Hold BOOT for 3 s to turn on Wi-Fi settings...");
    return;
  }
  if (!hid::connected()) {
    Serial.println("[button] not connected, ignored");
    return;
  }
  enterAt = 0;  // a new press cancels a pending Enter
  hid::down(settings::combo());
  comboDown = true;
  comboSince = now;
  Serial.printf("-> %s down\n", comboToString(settings::combo()).c_str());
}

static void onRelease() {
  if (gesture) {
    gesture = false;
    if (!gestureDone) Serial.println("Released too early, Wi-Fi not started");
    return;
  }
  if (!comboDown) return;
  hid::up();  // always release, even if the Mac dropped mid-press
  comboDown = false;
  Serial.printf("-> %s up\n", comboToString(settings::combo()).c_str());
  uint16_t delayMs = settings::enterDelayMs();
  if (delayMs && millis() - comboSince >= ENTER_MIN_HOLD_MS) enterAt = millis() + delayMs;
}

static void pollButton() {
  uint32_t now = millis();
  bool raw = digitalRead(BUTTON_PIN) == LOW;
  if (raw != lastRaw) {
    lastRaw = raw;
    lastChange = now;
  }
  if (now - lastChange >= DEBOUNCE_MS && raw != pressed) {
    pressed = raw;
    pressed ? onPress() : onRelease();
  }
  if (gesture && pressed && !gestureDone && now - gestureStart >= GESTURE_HOLD_MS) {
    gestureDone = true;
    web::start();
  }
}

static void pollSerial() {
  static String line;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\n' || c == '\r') {
      protocol::serialLine(line);
      line = "";
    } else if (line.length() < 300) {
      line += c;
    }
  }
}

void setup() {
  Serial.begin(115200);
  pinMode(BUTTON_PIN, INPUT_PULLUP);
  settings::begin();
  hid::begin(settings::slot());
  Serial.printf("ready %s fw %s — combo %s. Type 'help' for commands.\n", hid::name().c_str(),
                FW_VERSION, comboToString(settings::combo()).c_str());
}

void loop() {
  static hid::Bt lastBt = hid::Bt::OFF;
  hid::Bt bt = hid::state();
  if (bt != lastBt) {
    if (bt == hid::Bt::CONNECTED) Serial.println("Mac connected");
    else if (lastBt == hid::Bt::CONNECTED) Serial.println("Mac disconnected");
    lastBt = bt;
  }

  if (enterAt && millis() >= enterAt) {
    enterAt = 0;
    if (hid::connected()) {
      hid::tapEnter();
      Serial.println("-> Enter (after hold)");
    }
  }

  hid::loop();
  web::loop();
  pollSerial();
  pollButton();
  delay(2);
}
