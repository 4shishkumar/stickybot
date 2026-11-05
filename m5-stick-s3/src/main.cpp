// BLE macro keyboard — M5StickS3
//
// Button A (front) mirrors a key combo over Bluetooth: press = combo down,
// release = combo up (single press = once, double = twice, hold = held).
// Button B (side): click = Enter, double = unlock the Mac, hold = talk to the
// Mac agent (tools/mac-agent), triple = wake the screen / open the menu.
//
// Motion: lift = wake screen, face-down = standby, shake = Escape (or deny a
// Claude prompt). Each can be turned off in the menu / settings page.
//
// Battery: CPU at 80 MHz, screen off by default (on for a minute after boot,
// then only via button B), standby (deep sleep) after N minutes without a
// press while on battery — button A wakes it.
//
// Microphone (push-to-talk): while A is held it streams to the Mac as a USB
// microphone (for Wispr Flow etc.); while B is held it streams to the Mac
// agent over Bluetooth.
//
// Settings live in flash and can be changed from the menu, the Wi-Fi or USB
// settings page, or serial commands — see ../PROTOCOL.md.

#include <M5Unified.h>
#include "netlog.h"
#include <WiFi.h>
#include <driver/rtc_io.h>
#include "config.h"
#include "settings.h"
#include "hid.h"
#include "ui.h"
#include "menu.h"
#include "web.h"
#include "battery.h"
#include "protocol.h"
#include "buddy.h"
#include "motion.h"
#include "mic.h"
#include "logic/clock.h"
#include "logic/wispr.h"
#include "logic/prompt.h"
#include "logic/orientation.h"
#include "sound.h"
#include "events.h"
#include "ota.h"
#include <HijelHID_BLEKeyboard.h>  // KEY_MOD_* constants

constexpr uint32_t CPU_MHZ           = 80;
constexpr uint32_t BOOT_SCREEN_MS    = 60000;  // screen on after power-on
constexpr uint32_t WAKE_SCREEN_MS    = 10000;  // screen on after waking from standby
constexpr uint32_t B_SCREEN_MS       = 20000;  // screen on after button B
constexpr uint32_t STATUS_POLL_MS    = 1000;
constexpr uint32_t BATTERY_REPORT_MS = 60000;  // how often the Mac is told the level
constexpr uint32_t REPEAT_DELAY_MS   = 500;
constexpr uint32_t REPEAT_EVERY_MS   = 120;
constexpr uint32_t B_HOLD_MS         = 600;   // B held this long = menu, shorter = Enter
constexpr uint32_t ENTER_MIN_HOLD_MS = 1000;  // "Enter after hold" ignores shorter A presses
constexpr int      USB_POWER_MV      = 4000;   // VBUS above this = plugged in, no standby
constexpr gpio_num_t WAKE_PIN        = GPIO_NUM_11;  // button A

constexpr uint16_t COLOR_OK   = 0x03E0;
constexpr uint16_t COLOR_WARN = 0xC300;
constexpr uint16_t COLOR_DIM  = 0x2104;

enum class Mode { STATUS, MENU, WEB };

static Mode       mode = Mode::STATUS;
static bool       agentTalking = false;
static bool       swallowB = false;     // a B press that dismissed an event: ignore its click
static uint32_t   swallowBUntil = 0;
static uint32_t   agentShowUntil = 0, agentSeenAt = 0, talkEndedAt = 0, lastFrame = 0, promptAt = 0, pageAt = 0;
constexpr uint32_t PAGE_RETURN_MS = 10000;  // other pages go back to Home after this
constexpr uint32_t DOCK_IDLE_MS = 30000;    // on USB and untouched this long: the dock clock
static bool       docked = false;
// The dock clock is always landscape: turned to whichever side the stick lies on,
// or the last such side (default +1) when it stands upright or lies flat.
static int dockSide() {
  static int last = 1;
  if (int s = motion::sideways()) last = s;
  return last;
}
constexpr uint32_t NO_ANSWER_MS = 20000;    // "Thinking..." with no word from the agent: give up

static bool nightTime() { return clockfmt::isNight(buddy::hour()); }
static uint32_t   lastPing = 0;  // approval prompt sound
constexpr uint32_t PROMPT_GUARD_MS = 600;  // presses this soon after a prompt appears don't answer it
static bool       comboDown = false, dirty = true, ignoreAUntilRelease = false, ignoreBUntilRelease = false;
static uint32_t   comboSince = 0, enterAt = 0;
static bool       showingPairing = false, onUsbPower = true;
static wispr::Path aPath = wispr::Path::NONE;  // where the mic goes for the current / last A press
static uint32_t   messageUntil = 0, lastPoll = 0, lastRepeat = 0, lastInput = 0;
static uint32_t   lastBatteryReport = 0;
static int        reportedBattery = -1;
static ui::Status status{};
static ui::Page   page = ui::Page::HOME;
static orientation::Screen shownScreen{false, 0};  // the rotation the screens use now
static bool       devActive = false;  // developer mode: Wi-Fi kept on while on USB

static ui::Status readStatus() {
  hid::Bt bt = hid::state();
  return {settings::combo(), hid::slot(),
          bt == hid::Bt::CONNECTED ? 0 : bt == hid::Bt::ADVERTISING ? 1 : 2,
          battery::level(), battery::charging(), battery::line()};
}

static String   msgTitle, msgSub;  // the message on screen (redrawn when the stick turns)
static bool     msgHasSub = false;
static uint16_t msgBg = 0;

static void showMessage(const char* title, const char* sub, uint16_t bg, uint32_t ms = 1500) {
  ui::wake(ms + B_SCREEN_MS);
  ui::drawMessage(title, sub, bg);
  msgTitle = title;
  msgHasSub = sub != nullptr;
  msgSub = sub ? sub : "";
  msgBg = bg;
  messageUntil = millis() + ms;
}

static void webLines(String out[], int& n) {
  String m = web::mode(), url = web::url();
  url.replace("http://", "");
  n = 0;
  if (m == "joining") {
    out[n++] = "Joining Wi-Fi:";
    out[n++] = web::ssid();
  } else if (m == "home") {
    out[n++] = "On your Mac open:";
    out[n++] = url;
    out[n++] = "or:";
    out[n++] = WiFi.localIP().toString();
  } else if (m == "hotspot") {
    out[n++] = "Join Wi-Fi:";
    out[n++] = web::ssid();
    out[n++] = "Password:";
    out[n++] = web::password();
    out[n++] = "Then open:";
    out[n++] = url;
  }
}

static ui::Dash readDash() {
  ui::Dash d;
  d.combo = settings::combo();
  d.slot = hid::slot();
  hid::Bt bt = hid::state();
  d.bt = bt == hid::Bt::CONNECTED ? 0 : bt == hid::Bt::ADVERTISING ? 1 : 2;
  d.name = hid::name();
  d.battery = battery::level();
  d.charging = battery::charging();
  d.batLine = battery::line();
  d.linked = buddy::linked();
  d.sessions = buddy::total();
  d.running = buddy::running();
  d.waiting = buddy::waiting();
  d.tokens = buddy::tokensToday();
  d.claudeMsg = buddy::lastMessage();
  d.clock = buddy::clock();
  d.ampm = buddy::ampm();
  d.date = buddy::date();
  d.longDate = buddy::longDate();
  buddy::HomeCard hc = buddy::homeCard();
  d.cardKind = hc.kind;
  d.cardA = hc.a;
  d.cardB = hc.b;
  d.cardUrgent = hc.urgent;
  d.cardIndex = hc.index;
  d.cardCount = hc.count;
  const buddy::Weather& wx = buddy::weather();
  d.wxValid = wx.valid;
  d.wxTemp = wx.temp;
  d.wxHi = wx.hi;
  d.wxLo = wx.lo;
  d.wxIcon = wx.icon;
  d.wxCond = wx.cond;
  d.wxDay = wx.day;
  d.agentLinked = buddy::agentLinked();
  d.agentHeard = buddy::lastAnswer().heard;  // the Agent page shows the last good answer only
  d.agentText = buddy::lastAnswer().text;
  d.wifi = web::active() ? String(web::mode()) : "off";
  d.fw = FW_VERSION;
  d.uptimeS = millis() / 1000;
  d.heap = ESP.getFreeHeap();
  d.chargerRestarts = battery::chargerRestarts();
  return d;
}

static void render() {
  switch (mode) {
    case Mode::STATUS:
      if (docked)                         ui::drawDock(readDash(), dockSide(), nightTime());
      else if (comboDown)                 ui::drawHolding(status.combo, aPath == wispr::Path::USB, wispr::streams(aPath));
      else if (agentTalking)              ui::drawTalk(buddy::agentLinked());
      else if (millis() < agentShowUntil && agentSeenAt < talkEndedAt) ui::drawAgent("thinking", "", "");
      else if (millis() < agentShowUntil) ui::drawAgent(buddy::agent().state, buddy::agent().heard, buddy::agent().text);
      else                                ui::drawPage(page, readDash());
      break;
    case Mode::MENU:
      ui::drawMenu(menu::view());
      break;
    case Mode::WEB: {
      String lines[8];
      int n;
      webLines(lines, n);
      ui::drawInfo("Web settings", lines, n, "Triple-B: close");
      break;
    }
  }
}

// --- Standby -----------------------------------------------------------------

static void standby() {
  Log.println("Standby — press A to wake");
  Serial.flush();
  hid::up();
  ui::wake(2000);
  ui::drawMessage("Standby", "Press A to wake", COLOR_DIM);
  delay(1500);
  ui::off();
  // Turn off the L3B rail (LCD, backlight, mic, speaker amp) via the power chip;
  // M5.begin() turns it back on after waking.
  auto& pm = M5.Power.M5pm1;
  for (auto pin : {m5::M5PM1_Class::gpio2, m5::M5PM1_Class::gpio3}) {
    pm.setGPIOFunction(pin, m5::M5PM1_Class::gpio);
    pm.setGPIOMode(pin, m5::M5PM1_Class::output);
    pm.setGPIOOutput(pin, false);
  }
  rtc_gpio_pullup_en(WAKE_PIN);
  rtc_gpio_pulldown_dis(WAKE_PIN);
  esp_sleep_enable_ext0_wakeup(WAKE_PIN, 0);  // button A pulls the pin low
  esp_deep_sleep_start();
}

static void powerOff() {
  Log.println("Power off — press the side power button to turn on");
  Serial.flush();
  hid::up();
  ui::wake(2000);
  ui::drawMessage("Power off", "Side button turns on", COLOR_DIM);
  delay(1500);
  M5.Power.powerOff();  // power chip L0: everything off, charges at full speed
}

// Double-press B: wake the Mac's lock screen and type the saved password.
static void unlockMac() {
  String pass = settings::macPassword();
  if (pass.isEmpty()) { showMessage("No password", "Set it on the settings page", COLOR_WARN, 2500); return; }
  if (!hid::connected()) { showMessage("Not connected", nullptr, COLOR_WARN); return; }
  ui::wake(5000);
  ui::drawMessage("Unlocking", "Mac", COLOR_OK);
  hid::tap(Combo{KEY_MOD_LSHIFT, 0});  // wake the display / lock screen
  delay(1200);
  hid::typeText(pass);
  hid::tapEnter();
  Log.println("Unlock: password typed");  // never log the password itself
  sound::play(sound::Cue::UNLOCK);
  dirty = true;
}

static bool canStandby() {
  uint8_t m = settings::standbyMin();
  return m && !onUsbPower && mode == Mode::STATUS && !comboDown && !web::active() && !ota::active() &&
         !ota::pendingVerify() &&  // waking from standby is a reset: it would roll a new firmware back
         !hid::pairingCode() && millis() - lastInput > (uint32_t)m * 60000;
}

// --- Button A: the combo -------------------------------------------------------

// Wireless Wispr mic: while streaming, the key-up waits a moment (keyUpAt) for the
// audio still on its way to the Mac, so Wispr doesn't stop before the last words.
// With a 1.7.1+ agent the key-down waits too (wisprGate): the agent first makes the
// "stick-mic-bluetooth" device appear, so Wispr has switched to it when it starts.
// Plugged into a computer by USB, A is the USB mic exactly as before (no waiting).
static bool        wisprMic = false;
static uint32_t    keyUpAt = 0, releasedAt = 0;
static wispr::Gate wisprGate;
static uint16_t    holdId = 0;

static const char* pathName(wispr::Path p) {
  switch (p) {
    case wispr::Path::USB:        return " (USB mic)";
    case wispr::Path::BT_DIRECT:  return " (mic to Wispr by Bluetooth)";
    case wispr::Path::BT_PREPARE: return " (mic to Wispr by Bluetooth, <stick>-bt-mic)";
    default:                      return "";
  }
}

static void comboKeyDown() {
  hid::down(settings::combo());
  Log.printf("-> %s down%s\n", comboToString(settings::combo()).c_str(), pathName(aPath));
}

static void comboUp() {
  keyUpAt = 0;
  hid::up();  // always release, even if pairing dropped mid-press
  if (wisprMic) mic::wisprEnd();
  wisprMic = false;
  Log.printf("-> %s up\n", comboToString(settings::combo()).c_str());
  uint16_t delayMs = settings::enterDelayMs();
  if (delayMs && releasedAt - comboSince >= ENTER_MIN_HOLD_MS)
    enterAt = max((uint32_t)millis() + 1, releasedAt + delayMs);  // from the release, as without the tail
}

static void comboPress() {
  if (!hid::connected()) {
    if (ui::awake()) showMessage("Not connected", "Pair on the Mac first", COLOR_WARN);
    Log.println("[button] not connected, ignored");
    return;
  }
  if (keyUpAt) comboUp();  // pressed again during the tail: finish the last press first
  enterAt = 0;  // a new press cancels a pending Enter
  aPath = wispr::choosePath({settings::wirelessMic(), buddy::agentWirelessMic(), buddy::agentPreparesMic(),
                             buddy::agentLinked(), mic::usbMounted(), mic::usbStreaming(),
                             mic::wisprStreaming() || agentTalking});
  bool prepare = aPath == wispr::Path::BT_PREPARE;
  if (wispr::streams(aPath) && !mic::wisprStart(prepare, ++holdId)) aPath = wispr::Path::NONE;  // mic busy
  wisprMic = wispr::streams(aPath);
  comboDown = true;
  mic::usbTalk(true);
  comboSince = millis();
  dirty = true;
  if (aPath == wispr::Path::BT_PREPARE) {  // the stream runs already; the key waits for the agent's mic
    wisprGate.begin(millis(), holdId);
    Log.printf("-> %s: waiting for the Bluetooth mic (hold %u)\n", comboToString(settings::combo()).c_str(), holdId);
  } else {
    comboKeyDown();  // USB / no stick mic / older agent: the key never waits
  }
}

// The agent's "ready" (or the timeout) puts the key down for a waiting press.
static void wisprGateLoop() {
  uint16_t id;
  if (buddy::takeWisprReady(id) && !wisprGate.ready(id)) Log.printf("Agent: late ready (hold %u)\n", id);
  if (wisprGate.poll(millis()) != wispr::Gate::Step::PRESS) return;
  if (wisprGate.timedOut()) Log.printf("Agent: no ready in %lu ms, pressing anyway\n", (unsigned long)wisprGate.waitedMs());
  else Log.printf("Agent: mic ready in %lu ms\n", (unsigned long)wisprGate.waitedMs());
  comboKeyDown();
}

static void sendEnter(const char* why) {
  if (!hid::connected()) return;
  hid::tapEnter();
  Log.printf("-> Enter (%s)\n", why);
}

static void comboRelease() {
  if (!comboDown) return;
  comboDown = false;
  mic::usbTalk(false);
  dirty = true;
  releasedAt = millis();
  if (wisprGate.release() == wispr::Gate::Step::CANCEL) {  // let go before the mic was ready: no key at all
    if (wisprMic) mic::wisprEnd();
    wisprMic = false;
    enterAt = 0;
    Log.printf("-> %s cancelled (released after %lu ms, before the mic was ready)\n",
               comboToString(settings::combo()).c_str(), (unsigned long)(releasedAt - comboSince));
    return;
  }
  uint32_t tail = wispr::keyUpDelayMs(wisprMic);
  if (tail) keyUpAt = millis() + tail;
  else comboUp();
}

// --- Firmware update over Bluetooth ------------------------------------------------

// Why an update can't start right now (the agent shows it), nullptr = it can.
static const char* otaBusy() {
  if (comboDown || mic::wisprStreaming()) return "dictation";
  if (agentTalking) return "talk";
  if (buddy::hasPrompt()) return "approval";
  if (hid::pairingCode()) return "pairing";
  if (mode == Mode::MENU) return "menu";
  if (web::busy()) return "wifi-update";
  return nullptr;
}

// While an update runs it owns the screen and the buttons: B cancels, A does nothing.
static bool otaLoop() {
  static bool was = false;
  ota::loop();
  if (ota::active()) {
    was = true;
    if (M5.BtnB.wasPressed()) {
      ota::cancelByUser();
      swallowB = true;  // that press isn't an Enter
    }
    return true;
  }
  if (was) {
    was = false;
    ignoreAUntilRelease = M5.BtnA.isPressed();
    swallowB = swallowB || M5.BtnB.isPressed();
    dirty = true;
  }
  String notice;
  if (ota::takeNotice(notice)) showMessage(notice.c_str(), "Nothing changed on the stick", COLOR_WARN, 3000);
  return false;
}

// --- Menu ------------------------------------------------------------------------

static void onMenu(menu::Action a) {
  switch (a) {
    case menu::Action::NONE:
      return;
    case menu::Action::REDRAW:
      break;
    case menu::Action::TEST:
      if (hid::connected()) hid::tap(menu::draft().combo);
      else showMessage("Not connected", nullptr, COLOR_WARN);
      break;
    case menu::Action::WEB:
      web::start();
      mode = Mode::WEB;
      break;
    case menu::Action::FLIP:
      settings::saveFlipped(!settings::flipped());
      ui::setFlipped(settings::flipped());
      break;
    case menu::Action::POWER_OFF:
      powerOff();
      break;
    case menu::Action::VOLUME:  // takes effect (and plays a preview) right away
      settings::saveSoundVolume(menu::draft().volume);
      sound::play(sound::Cue::TEST);
      dirty = true;
      return;
    case menu::Action::BRIGHTNESS:
      ui::setBrightness(menu::draft().brightnessPct);
      break;
    case menu::Action::BT_TOGGLE:
      if (hid::state() == hid::Bt::OFF) hid::reconnect();
      else hid::disconnect();
      break;
    case menu::Action::FORGET:
      hid::forget(hid::slot());
      Log.printf("Forgot Mac %u, restarting\n", hid::slot());
      ui::drawMessage("Forgotten", "Remove it on the Mac too", COLOR_WARN);
      delay(2000);
      ESP.restart();
      break;
    case menu::Action::SAVE: {
      const menu::Draft& d = menu::draft();
      settings::saveCombo(d.combo);
      settings::saveEnterDelayMs(d.enterDelayMs);
      settings::saveMotion(d.lift, d.faceDown, d.shake);
      settings::saveAutoRotate(d.autoRotate);
      settings::saveLandscape(d.landscape);
      settings::saveDevMode(d.devMode);
      settings::saveSoundOn(d.sound);
      settings::saveSoundVolume(d.volume);
      settings::saveWirelessMic(d.wmic);
      if (!d.autoRotate) ui::setFlipped(settings::flipped());
      settings::saveStandbyMin(d.standbyMin);
      settings::saveBrightnessPct(d.brightnessPct);
      Log.printf("Menu: combo %s, standby %u min\n", comboToString(d.combo).c_str(), d.standbyMin);
      if (d.slot != hid::slot()) settings::saveSlot(d.slot);
      if (hid::needsRestart(d.combo)) {  // Fn added or removed: new keyboard identity
        Log.println("Keyboard identity changes for Fn, restarting");
        ui::drawMessage("Restarting", "Fn: re-pair on the Mac", COLOR_OK);
        delay(2500);
        ESP.restart();
      }
      if (d.slot != hid::slot()) {
        settings::saveSlot(d.slot);
        Log.printf("Mac %u, restarting\n", d.slot);
        ui::drawMessage("Restarting", "Switching Mac", COLOR_OK);
        delay(1500);
        ESP.restart();
      }
      mode = Mode::STATUS;
      status = readStatus();
      showMessage("Saved", comboToString(d.combo).c_str(), COLOR_OK);
      break;
    }
    case menu::Action::CANCEL:
      ui::setBrightness(settings::brightnessPct());  // undo preview
      mode = Mode::STATUS;
      break;
  }
  dirty = true;
}

// --- Serial ------------------------------------------------------------------------

// Serial "screens": shows every screen with sample data (~3 s each), for design review:
// portrait first, then the landscape layouts (button A on the right, then on the left).
// "screens landscape" shows only the landscape ones.
static void screenDemo(bool portrait = true) {
  Log.println("Screen demo: each screen for ~3 s");
  ui::wake(240000);
  ui::Dash d = readDash();
  auto hold = [](uint32_t ms, auto draw) {
    for (uint32_t t0 = millis(); millis() - t0 < ms;) {
      ui::setLevel((int)(9000 + 7000 * sinf(millis() / 160.0f)));
      draw();
      ui::keepAwake();
      delay(40);
    }
  };
  auto prompts = [&] {
    hold(4000, [] { ui::drawPrompt("Bash", "rm -rf build/ && npm run build", "web"); });
    hold(4000, [] { ui::drawPrompt("Edit", "src/components/settings/ThemeToggle.tsx: replace the toggle with a three-way "
                                           "switch (light / dark / system) and keep the saved choice", "acme-web"); });
    hold(4000, [] { ui::drawAsk("Deploy the preview build to staging now, or wait for the tests to finish first?",
                                "acme-web", "Staging now", "Wait"); });
  };
  auto events = [&] {
    uint32_t t0 = millis();
    hold(6000, [&] { ui::drawEvent(0, "PR #5507 merged!", "acme/web - feat: dark mode", "merge", millis() - t0); });
    t0 = millis();
    hold(4000, [&] { ui::drawEvent(1, "PR approved", "#5510 by teammate - ready to merge", "check", millis() - t0); });
    t0 = millis();
    hold(4000, [&] { ui::drawEvent(2, "Review requested", "#5512 - fix login redirect", "review", millis() - t0); });
  };
  // The screens with text to read (all of them have a landscape layout)
  auto reading = [&](bool moods) {
    for (int p = 0; p < (int)ui::Page::COUNT; p++) hold(3000, [&] { ui::drawPage((ui::Page)p, d); });
    hold(3500, [&] { ui::drawHolding(settings::combo(), true); });
    hold(3500, [] { ui::drawTalk(true); });
    hold(2500, [] { ui::drawTalk(false); });
    hold(3000, [] { ui::drawAgent("thinking", "what's on my calendar today", ""); });
    hold(3000, [] { ui::drawAgent("working", "how much disk space do I have", "Running: df -h /"); });
    hold(4000, [] { ui::drawAgent("done", "what's the weather in Bangalore", "It's 28°C in Bangalore with patchy rain nearby and 54% humidity."); });
    hold(2500, [] { ui::drawAgent("error", "open the thing", "Didn't catch that."); });
    prompts();
    if (moods) {
      static const char* MOODS[] = {"Idle", "Listening", "Thinking", "Working", "Done", "Error", "Needs you", "Offline", "Party"};
      for (int m = 0; m < ui::MOOD_COUNT; m++) hold(1800, [&] { ui::drawMood(m, MOODS[m]); });
    }
    {  // home: a fact, then the eyes peek and a news item fades in
      ui::Dash h = d;
      h.agentLinked = true; h.bt = 0;
      h.cardKind = "fact"; h.cardA = "Did you know?"; h.cardB = "Honey never spoils."; h.cardUrgent = false;
      h.cardIndex = 1; h.cardCount = 5;
      hold(1500, [&] { ui::drawPage(ui::Page::HOME, h); });
      ui::forceHomePeek();
      h.cardKind = "news"; h.cardA = "Hacker News - 433 pts"; h.cardB = "OpenBSD 8.0 released"; h.cardIndex = 2;
      hold(2800, [&] { ui::drawPage(ui::Page::HOME, h); });
      h.cardKind = "wiki"; h.cardA = "Wikipedia: London Monster"; h.cardB = "An attacker of women in London, 1788-1790."; h.cardIndex = 3;
      hold(1500, [&] { ui::drawPage(ui::Page::HOME, h); });
    }
    events();
    hold(3000, [] { ui::drawPairing(482913); });
    hold(2500, [] { ui::drawMessage("Saved", "Option+Cmd", 0x03E0); });
    {
      static const String WEB[] = {"Join Wi-Fi:", AP_SSID, "Password:", DEFAULT_PASSWORD, "Then open:", HOSTNAME ".local"};
      hold(2500, [] { ui::drawInfo("Web settings", WEB, 6, "Triple-B: close"); });
    }
  };

  if (portrait) {
    ui::setOrientation(shownScreen.flipped, 0);
    reading(true);
    {  // dock: landscape, portrait, night
      ui::Dash k = d;
      k.charging = true;
      hold(3500, [&] { ui::drawDock(k, 1, false); });
      hold(3000, [&] { ui::drawDock(k, 0, false); });
      hold(2500, [&] { ui::drawDock(k, 1, true); });
      hold(2000, [&] { ui::drawDock(k, 0, true); });
      ui::dockEnd();
    }
  }
  // Landscape: turn the stick so the arrow on the A chip points at button A
  ui::setOrientation(shownScreen.flipped, 1);
  hold(4000, [] { ui::drawMessage("Landscape, A on the right", "Turn the stick until this reads upright: the arrow on A points at button A", 0x03E0); });
  reading(false);
  ui::setOrientation(shownScreen.flipped, -1);
  hold(4000, [] { ui::drawMessage("Landscape, A on the left", "Turn the stick until this reads upright: the arrow on A points at button A", 0x03E0); });
  prompts();
  events();
  hold(4000, [] { ui::drawAgent("done", "what's the weather in Bangalore", "It's 28°C in Bangalore with patchy rain nearby and 54% humidity."); });
  ui::setOrientation(shownScreen.flipped, shownScreen.side);  // back to how the stick is held
  Log.println("Screen demo done");
}

static void pollSerial() {
  static String line;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\n' || c == '\r') {
      if (line == "screens") screenDemo();
      else if (line == "screens landscape") screenDemo(false);
      else if (line.startsWith("event ")) {  // event celebrate|nice|headsup <title>|<sub>|<icon>
        String rest = line.substring(6);
        rest.trim();
        int sp = rest.indexOf(' ');
        events::Event ev;
        String lvl = sp < 0 ? rest : rest.substring(0, sp), body = sp < 0 ? "" : rest.substring(sp + 1);
        if (!events::parseLevel(lvl, ev.level)) {
          Log.println("event celebrate|nice|headsup <title>|<sub>|<icon>");
        } else {
          int p1 = body.indexOf('|'), p2 = p1 < 0 ? -1 : body.indexOf('|', p1 + 1);
          ev.title = p1 < 0 ? body : body.substring(0, p1);
          ev.sub = p1 < 0 ? "" : body.substring(p1 + 1, p2 < 0 ? body.length() : p2);
          ev.icon = p2 < 0 ? (ev.level == events::Level::CELEBRATE ? "merge" : ev.level == events::Level::NICE ? "check" : "review")
                           : body.substring(p2 + 1);
          if (!ev.title.length()) ev.title = lvl == "celebrate" ? "PR #5507 merged!" : lvl == "nice" ? "PR approved" : "Review requested";
          ev.id = "serial-" + String(millis());
          events::push(ev);
        }
      }
      else if (line.startsWith("sound")) {  // sound test | on | off | vol <0-255>
        String arg = line.substring(5);
        arg.trim();
        if (arg == "on" || arg == "off") settings::saveSoundOn(arg == "on");
        else if (arg.startsWith("vol")) settings::saveSoundVolume(constrain(arg.substring(3).toInt(), 0, 255));
        else if (arg.startsWith("play ")) {
          String name = arg.substring(5);
          name.trim();
          if (!sound::play(name)) Log.printf("unknown sound '%s'\n", name.c_str());
          else Log.printf("playing %s\n", name.c_str());
        } else sound::play(sound::Cue::TEST);
        Log.printf("Sounds %s, volume %u\n", settings::soundOn() ? "on" : "off", settings::soundVolume());
      }
      else protocol::serialLine(line);
      line = "";
      dirty = true;
    } else if (line.length() < 300) {
      line += c;
    }
  }
}

// -----------------------------------------------------------------------------------

void setup() {
  bool fromStandby = esp_sleep_get_wakeup_cause() == ESP_SLEEP_WAKEUP_EXT0;
  auto cfg = M5.config();
  M5.begin(cfg);
  setCpuFrequencyMhz(CPU_MHZ);
  M5.BtnB.setHoldThresh(B_HOLD_MS);
  M5.Power.M5pm1.setBatteryCharge(true);  // make sure the charger is on
  Serial.begin(115200);

  settings::begin();
  ui::begin();
  ui::setFlipped(settings::flipped());
  ui::setBrightness(settings::brightnessPct());
  ui::wake(fromStandby ? WAKE_SCREEN_MS : BOOT_SCREEN_MS);
  hid::begin(settings::slot());
  motion::begin();
  motion::resetFacing(settings::flipped() ? orientation::Orient::PORTRAIT_FLIPPED : orientation::Orient::PORTRAIT);
  mic::begin();
  sound::begin();

  ignoreAUntilRelease = fromStandby;  // the press that woke us isn't a combo press
  lastInput = millis();
  status = readStatus();
  ota::setBusyCheck(otaBusy);
  ota::bootCheck();  // last: it may start the loop watchdog until a new firmware is confirmed
  Log.printf("ready %s fw %s — combo %s%s. Type 'help' for commands.\n", hid::name().c_str(),
                FW_VERSION, comboToString(settings::combo()).c_str(), fromStandby ? " (woke from standby)" : "");
}

void loop() {
  M5.update();
  battery::update();
  hid::loop();
  buddy::loop();
  web::loop();
  netlog::loop();
  mic::loop();
  sound::loop();
  pollSerial();
  if (otaLoop()) {
    delay(2);  // the update screen: no other screens, buttons or standby meanwhile
    return;
  }

  // Developer mode: on USB power keep Wi-Fi (home network) on for OTA + logs.
  {
    bool want = settings::devMode() && battery::usbPower() && settings::wifiSsid().length();
    if (want && !web::active()) web::start(false);  // in the background, no info screen
    if (want != devActive) {
      devActive = want;
      web::setKeepAlive(want);
      Log.printf("Developer mode %s\n", want ? "on: Wi-Fi stays up for OTA + logs" : "off");
    }
  }

  // Mode follows Wi-Fi state (it can be turned on/off over USB or by idle timeout)
  if (mode == Mode::STATUS && web::shown()) { mode = Mode::WEB; ui::wake(B_SCREEN_MS); dirty = true; }
  if (mode == Mode::WEB && !web::shown())   { mode = Mode::STATUS; dirty = true; }

  if (millis() - lastPoll > STATUS_POLL_MS) {
    lastPoll = millis();
    onUsbPower = M5.Power.getVBUSVoltage() > USB_POWER_MV;
    mic::setUsbPower(onUsbPower);
    ui::Status s = readStatus();
    if (s.bt != status.bt) {
      Log.println(s.bt == 0 ? "Mac connected" : "Mac disconnected");
      if (s.bt == 0 || status.bt == 0) sound::play(s.bt == 0 ? sound::Cue::CONNECTED : sound::Cue::DISCONNECTED);
    }
    if (s.charging && !status.charging && millis() > 15000) sound::play(sound::Cue::CHARGING);
    static bool lowWarned = false;
    if (s.battery >= 0 && s.battery <= 15 && !s.charging && !lowWarned) { lowWarned = true; sound::play(sound::Cue::LOW_BATTERY); }
    if (s.battery > 20 || s.charging) lowWarned = false;
    if (s.battery >= 0 && s.battery != reportedBattery &&
        (reportedBattery < 0 || millis() - lastBatteryReport > BATTERY_REPORT_MS)) {
      hid::setBattery(s.battery);
      reportedBattery = s.battery;
      lastBatteryReport = millis();
    }
    if (s != status && !comboDown) dirty = true;
    status = s;
    if (mode == Mode::WEB || (mode == Mode::STATUS && ui::awake() && !comboDown)) dirty = true;
  }

  if (keyUpAt && millis() >= keyUpAt) comboUp();
  wisprGateLoop();
  buddy::setUsbMounted(mic::usbMounted());
  if (enterAt && millis() >= enterAt) {
    enterAt = 0;
    sendEnter("after hold");
  }

  auto& A = M5.BtnA;
  auto& B = M5.BtnB;
  bool aDown = A.wasPressed(), aUp = A.wasReleased(), bDown = B.wasPressed();
  bool bHold = B.wasHold();
  bool bSingle = B.wasDecideClickCount() && B.getClickCount() == 1;
  bool bDouble = B.wasDecideClickCount() && B.getClickCount() == 2;
  bool bTriple = B.wasDecideClickCount() && B.getClickCount() >= 3;
  if (aDown || bDown) lastInput = millis();
  if (ignoreBUntilRelease) {  // the hold that opened the menu
    if (!B.isPressed()) ignoreBUntilRelease = false;
    bDown = bSingle = bDouble = bTriple = bHold = false;
  }
  if (agentTalking && !B.isPressed()) {
    agentTalking = false;
    mic::agentEnd();
    if (buddy::agentLinked()) sound::play(sound::Cue::SENT);  // plays once the mic has stopped
    bSingle = bDouble = bTriple = false;
    talkEndedAt = millis();
    agentShowUntil = buddy::agentLinked() ? millis() + NO_ANSWER_MS : 0;  // "Thinking..." until the agent answers
    dirty = true;
  }
  // New status from the agent: show it (and wake the screen)
  if (buddy::agentUpdated() != agentSeenAt) {
    agentSeenAt = buddy::agentUpdated();
    const String& st = buddy::agent().state;
    if (st == "cancelled") {  // a quick press with no words: back to the normal screen, quietly
      agentShowUntil = 0;
      talkEndedAt = 0;
      dirty = true;
    } else {
      if (st == "done") sound::play(sound::Cue::ANSWER);
      else if (st == "error") sound::play(sound::Cue::ERROR);
      uint32_t keep = st == "done" ? 15000 : st == "error" ? 6000 : 90000;  // working: Claude can take a while
      agentShowUntil = millis() + keep;
      ui::wake(keep);
      dirty = true;
    }
  }
  if (agentShowUntil && millis() >= agentShowUntil) {
    // Still "Thinking..." with no word from the agent at all: say so instead of hanging
    if (agentSeenAt < talkEndedAt && talkEndedAt) {
      talkEndedAt = 0;
      showMessage("No answer", "The Mac agent didn't respond", COLOR_WARN);
      sound::play(sound::Cue::ERROR);
    }
    agentShowUntil = 0;
    dirty = true;
  }


  if (ignoreAUntilRelease) {
    if (aUp || !A.isPressed()) ignoreAUntilRelease = false;
    aDown = aUp = false;
  }
  if (swallowB && !B.isPressed()) {  // keep ignoring until its click count is decided
    swallowB = false;
    swallowBUntil = millis() + 900;
  }
  if (swallowB || millis() < swallowBUntil) bDown = bSingle = bDouble = bTriple = bHold = false;

  motion::Event mev = motion::poll();
  if (mev != motion::Event::NONE) {
    static const char* NAMES[] = {"none", "lift", "face-down", "shake", "flick left", "flick right"};
    Log.printf("motion: %s\n", NAMES[(int)mev]);
  }

  // Escape: on an agent screen, a click of B (or a shake) dismisses it and cancels
  // the request; back to Home. That click doesn't send Enter.
  if (mode == Mode::STATUS && agentShowUntil && millis() < agentShowUntil && !agentTalking &&
      (bSingle || (mev == motion::Event::SHAKE && settings::shakeEscapes()))) {
    const String& st = buddy::agent().state;
    bool busy = agentSeenAt < talkEndedAt || st == "thinking" || st == "working";
    if (busy) buddy::agentCancel();
    Log.printf("Agent screen dismissed%s\n", busy ? " (request cancelled)" : "");
    agentShowUntil = 0;
    talkEndedAt = 0;
    page = ui::Page::HOME;
    bSingle = false;
    mev = motion::Event::NONE;
    dirty = true;
  }
  if ((mev == motion::Event::FLICK_LEFT || mev == motion::Event::FLICK_RIGHT) && mode == Mode::STATUS && !comboDown) {
    int n = (int)ui::Page::COUNT, step = mev == motion::Event::FLICK_RIGHT ? 1 : n - 1;
    page = (ui::Page)(((int)page + step) % n);
    pageAt = millis();
    lastInput = millis();
    ui::wake(B_SCREEN_MS);
    dirty = true;
  }
  // Which way the screens face: the manual flip, or with auto-rotate the motion sensor
  // (held sideways, the reading screens turn landscape). A turn redraws whatever is shown.
  bool turned = false;
  {
    orientation::Screen sc =
        orientation::choose(motion::facing(), settings::autoRotate(), settings::landscape(), settings::flipped());
    if (sc != shownScreen) {
      shownScreen = sc;
      ui::setOrientation(sc.flipped, sc.side);
      turned = true;
      dirty = true;
      lastFrame = 0;  // an event redraws right away
      if (messageUntil && millis() < messageUntil) ui::drawMessage(msgTitle.c_str(), msgHasSub ? msgSub.c_str() : nullptr, msgBg);
    }
  }
  if (mev == motion::Event::LIFT && settings::liftWakes()) {
    lastInput = millis();
    if (ui::wake(B_SCREEN_MS)) dirty = true;
  }

  // Pairing request: show the code, A accepts, B rejects
  uint32_t code = hid::pairingCode();
  if (code) {
    if (!showingPairing) {
      showingPairing = true;
      ui::wake(35000);
      Log.printf("Pairing code %06lu — press A on the stick to accept\n", (unsigned long)code);
    }
    ui::drawPairing(code);
    ui::keepAwake();
    if (aDown) hid::confirmPairing(true);
    if (bDown) hid::confirmPairing(false);
    delay(20);
    return;
  }
  if (showingPairing) {
    showingPairing = false;
    dirty = true;
  }

  // Claude permission prompt: A approves, B denies
  static String shownPrompt;
  if (buddy::hasPrompt()) {
    const buddy::Prompt& p = buddy::prompt();
    bool fresh = shownPrompt != p.id;
    if (fresh) {
      shownPrompt = p.id;
      promptAt = millis();
      lastPing = 0;
      ui::wake(60000);
      if (p.ask) Log.printf("Question (%s): %s — A %s, B %s\n", p.from.c_str(), p.q.c_str(), p.a.c_str(), p.b.c_str());
      else Log.printf("Claude asks (%s): %s %s — A approve, B deny\n", p.from.length() ? p.from.c_str() : "desktop",
                      p.tool.c_str(), p.hint.c_str());
    }
    if (fresh || turned) {  // turning the stick redraws it; its timers carry on
      if (p.ask) ui::drawAsk(p.q.length() ? p.q : p.hint, p.from, p.a, p.b);
      else ui::drawPrompt(p.tool, p.hint, p.from);
    }
    ui::keepAwake();
    if (comboDown) comboRelease();
    if (millis() - promptAt < PROMPT_GUARD_MS) aDown = bDown = false;  // a press already on its way
    if (!lastPing || millis() - lastPing > 15000) {  // ping when it appears, then every 15 s
      lastPing = millis();
      sound::play(sound::Cue::APPROVAL);
    }
    bool ask = p.ask;  // (p is gone once answered)
    bool shake = mev == motion::Event::SHAKE && settings::shakeEscapes() && millis() - promptAt >= PROMPT_GUARD_MS;
    if (aDown) {
      buddy::answer(prompt::decision(ask, prompt::Input::A));
      Log.println(ask ? "Question: A" : "Claude: approved");
      sound::play(sound::Cue::APPROVED);
    } else if (bDown || shake) {
      buddy::answer(prompt::decision(ask, bDown ? prompt::Input::B : prompt::Input::SHAKE));
      Log.println(ask ? (bDown ? "Question: B" : "Question: dismissed") : "Claude: denied");
      sound::play(sound::Cue::DENIED);
    }
    delay(20);
    return;
  }
  if (shownPrompt.length()) {
    shownPrompt = "";
    dirty = true;
  }

  // Event screens from the agent (celebrate / nice / headsup), one at a time,
  // never over dictation, talking, the menu or a press in progress
  {
    bool canShow = (mode == Mode::STATUS || mode == Mode::WEB) && !comboDown && !agentTalking &&
                   !A.isPressed() && !B.isPressed();
    if (!events::active() && events::pending() && canShow) {
      events::start();
      const events::Event& e = events::current();
      buddy::eventShown(e.id);
      ui::wake(e.level == events::Level::HEADSUP ? 61000 : 8000);
      if (settings::soundOn()) sound::play(e.sound.length() ? e.sound : String(events::defaultSound(e.level)));
      Log.printf("Showing event: %s\n", e.title.c_str());
      lastFrame = 0;
    }
    if (events::active()) {
      const events::Event& e = events::current();
      bool press = aDown || bDown;  // any press dismisses, and does nothing else
      if (press || events::expired()) {
        events::finish();
        if (press) {
          ignoreAUntilRelease = A.isPressed();
          swallowB = B.isPressed();
        }
        dirty = true;
        if (press) {
          delay(10);
          return;
        }
      } else {
        ui::keepAwake();
        if (millis() - lastFrame >= 40) {
          lastFrame = millis();
          ui::drawEvent((uint8_t)e.level, e.title, e.sub, e.icon, events::elapsed());
        }
        delay(10);
        return;
      }
    }
  }

  if (mev == motion::Event::SHAKE && settings::shakeEscapes() && mode != Mode::MENU && hid::connected()) {
    hid::tapEscape();
    Log.println("-> Escape (shake)");
  }
  // Face-down = standby, also on USB and with developer-mode Wi-Fi in the
  // background; only skipped while a menu / settings screen / key is in use.
  if (mev == motion::Event::FACE_DOWN && settings::faceDownSleeps() && mode == Mode::STATUS && !comboDown &&
      !web::shown() && !ota::pendingVerify()) {
    Log.println("Face-down: going to standby");
    standby();
  }

  switch (mode) {
    case Mode::STATUS:
    case Mode::WEB:
      if (aDown) comboPress();  // works with the screen off; doesn't wake it
      if (aUp) comboRelease();
      if (bSingle) sendEnter("button B");  // works with the screen off; doesn't wake it
      if (bDouble) unlockMac();
      // Hold B = talk to the Mac agent. Recording starts on every B press
      // (pre-roll) and is dropped again if it turns out to be a click.
      if (bDown && !comboDown) mic::preRoll();
      if (bHold && !comboDown) {
        agentTalking = true;
        if (buddy::agentLinked()) {
          mic::agentTalk();
          sound::playListen();  // "speak now" (the mic pauses ~0.2 s for it)
        }
        else mic::agentEnd();
        Log.printf("Agent: talk%s\n", buddy::agentLinked() ? "" : " (agent not running)");
        ui::wake(B_SCREEN_MS);
        dirty = true;
      }
      if (!B.isPressed() && !agentTalking) mic::agentEnd();  // a click: drop the pre-roll
      if (bTriple && !comboDown) {
        if (ui::wake(B_SCREEN_MS)) {
          dirty = true;  // screen was off: this only wakes it
        } else if (mode == Mode::STATUS) {
          menu::open({settings::combo(), hid::slot(), settings::standbyMin(), settings::brightnessPct(),
                      settings::enterDelayMs(),
                      settings::liftWakes(), settings::faceDownSleeps(), settings::shakeEscapes(),
                      settings::autoRotate(), settings::landscape(), settings::devMode(), settings::soundOn(), settings::soundVolume(),
                      settings::wirelessMic()},
                     hid::state() == hid::Bt::OFF);
          mode = Mode::MENU;
          dirty = true;
        } else {
          if (devActive) web::hide();  // keep Wi-Fi for OTA + logs
          else web::stop();
          mode = Mode::STATUS;
          dirty = true;
        }
      }
      break;

    case Mode::MENU:
      ui::keepAwake();
      if (aDown) onMenu(menu::next());
      if (bDown) {
        onMenu(menu::select());
        lastRepeat = millis();
      }
      if (B.isPressed() && B.pressedFor(REPEAT_DELAY_MS) && millis() - lastRepeat > REPEAT_EVERY_MS) {
        onMenu(menu::repeat());
        lastRepeat = millis();
      }
      if (mode == Mode::MENU && menu::timedOut()) onMenu(menu::Action::CANCEL);
      break;
  }

  // Dock: on USB power and untouched for a while, the stick becomes a desk clock.
  // Any input (button, lift, flick) ends it — and the button still does its job.
  {
    bool want = battery::usbPower() && mode == Mode::STATUS && !comboDown && !agentTalking &&
                millis() >= agentShowUntil && !events::active() && !events::pending() && !messageUntil &&
                millis() - lastInput >= DOCK_IDLE_MS;
    if (want != docked) {
      docked = want;
      if (!docked) ui::dockEnd();
      else page = ui::Page::HOME;
      Log.printf("Dock %s\n", docked ? "on" : "off");
      dirty = true;
    }
    if (docked) {
      if (!ui::awake()) ui::wake(60000);
      ui::keepAwake();
    }
  }

  // Live screens animate (mic level, eyes): redraw them ~25 times a second
  {
    const String& st = buddy::agent().state;
    bool agentBusy = millis() < agentShowUntil && (agentSeenAt < talkEndedAt || st == "thinking" || st == "working");
    bool live = mode == Mode::STATUS && !docked && (comboDown || agentTalking || agentBusy);
    bool calm = mode == Mode::STATUS && !live && !docked && millis() >= agentShowUntil;
    bool homeAnim = calm && page == ui::Page::HOME && ui::homeAnimating();  // card fade / eyes peeking
    bool idleEyes = calm && page == ui::Page::AGENT;                          // blinking, glancing
    uint32_t every = live || homeAnim ? 40 : idleEyes ? 80 : docked && battery::charging() ? 150 : 0;
    if (every && ui::awake() && millis() - lastFrame >= every) {
      lastFrame = millis();
      ui::setLevel(mic::live() ? mic::peak() : 0);
      { float ax, ay, az; motion::accel(ax, ay, az); ui::setTilt(ax, ay); }
      dirty = true;
    }
  }

  if (messageUntil) {
    if (millis() < messageUntil) dirty = false;
    else { messageUntil = 0; dirty = true; }
  }
  if (dirty && ui::awake()) {
    render();
    dirty = false;
  }
  if (mode == Mode::WEB) ui::keepAwake();
  ui::tick(!messageUntil);

  if (!ui::awake()) page = ui::Page::HOME;  // waking the screen starts on Home
  if (page != ui::Page::HOME && millis() - pageAt > PAGE_RETURN_MS) {
    page = ui::Page::HOME;
    dirty = true;
  }
  if (canStandby()) standby();
  delay(10);
}
