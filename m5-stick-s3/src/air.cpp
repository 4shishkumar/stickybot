// Remote modes: air mouse, presenter, media remote (air.h). The maths is in
// logic/air_motion.* and logic/remote.* (unit-tested); this file wires it to the IMU,
// the buttons, Bluetooth (hid.h) and the screen (air_ui.h).
#include "air.h"
#include <M5Unified.h>
#include <Preferences.h>
#include <HijelHID_BLEKeyboard.h>  // KEY_*, MEDIA_*, MOUSE_BUTTON_*
#include "air_ui.h"
#include "battery.h"
#include "hid.h"
#include "motion.h"
#include "netlog.h"
#include "settings.h"
#include "ui.h"
#include "logic/air_motion.h"
#include "logic/orientation.h"
#include "logic/remote.h"

namespace air {

constexpr uint32_t IDLE_OFF_MS       = 5 * 60000;   // mouse / media: untouched (lying still) this long -> Home
constexpr uint32_t IDLE_OFF_TALK_MS  = 30 * 60000;  // presenter, timer not running
constexpr uint32_t SCREEN_MS         = 20000;       // mouse / media: the screen sleeps after this (buttons wake it)
constexpr uint32_t RESTING_MS        = 1500;        // still this long: shown as "Resting"
constexpr uint32_t MOUSE_UNKNOWN_MS  = 6000;        // connected this long without a mouse subscription: say so
constexpr uint32_t FLASH_MS          = 900;
constexpr uint32_t DRAW_MIN_MS       = 90;

static Settings set_;
static Mode     mode_ = Mode::OFF;
static volatile int8_t requested = -1;  // a Mode to switch to at the next loop, -1 none

static airmotion::Frame     frame;
static airmotion::Stillness still;
static airmotion::Pointer   pointer;
static airmotion::Scroller  scroller;
static airmotion::Knob      knob;
static remote::Buttons      buttons;
static remote::TalkTimer    timer;

static uint32_t lastCount = 0, lastUs = 0, lastActivity = 0, lastDraw = 0, connectedAt = 0, flashAt = 0;
static bool     paused = false, scrolling = false, leftDown = false, laser = false, black = false;
static bool     dirty = true, wasConnected = false;
static int      slide = 0;
static uint8_t  mediaAction = airui::NONE;
static String   flash;
static orientation::Screen shown{false, 0};
static uint32_t shownSecond = UINT32_MAX;

// --- Settings ------------------------------------------------------------------------------

static Preferences prefs;

void begin() {
  if (!prefs.begin("air", false)) return;
  set_.sens = constrain(prefs.getUChar("sens", 5), 1, 10);
  set_.invX = prefs.getBool("invx", false);
  set_.invY = prefs.getBool("invy", false);
  set_.scroll = constrain(prefs.getUChar("scroll", 5), 1, 10);
  set_.invScroll = prefs.getBool("invs", false);
  set_.talkMin = min((int)prefs.getUShort("talk", 0), 600);
  set_.pageKeys = prefs.getBool("pgkeys", false);
}

const Settings& settings() { return set_; }

void save(const Settings& s) {
  set_ = s;
  prefs.putUChar("sens", s.sens);
  prefs.putBool("invx", s.invX);
  prefs.putBool("invy", s.invY);
  prefs.putUChar("scroll", s.scroll);
  prefs.putBool("invs", s.invScroll);
  prefs.putUShort("talk", s.talkMin);
  prefs.putBool("pgkeys", s.pageKeys);
  pointer.configure(s.sens, s.invX, s.invY);
  scroller.configure(s.scroll, s.invScroll);
  dirty = true;
}

// --- Modes ---------------------------------------------------------------------------------

const char* modeName(Mode m) {
  switch (m) {
    case Mode::MOUSE:     return "mouse";
    case Mode::PRESENTER: return "presenter";
    case Mode::MEDIA:     return "media";
    default:              return "home";
  }
}

bool parseMode(const String& s, Mode& out) {
  String v = s;
  v.trim();
  v.toLowerCase();
  if (v == "mouse" || v == "air" || v == "airmouse") out = Mode::MOUSE;
  else if (v == "presenter" || v == "present" || v == "slides") out = Mode::PRESENTER;
  else if (v == "media" || v == "music") out = Mode::MEDIA;
  else if (v == "home" || v == "off" || v == "none" || v == "") out = Mode::OFF;
  else return false;
  return true;
}

void request(Mode m) { requested = (int8_t)m; }
bool active() { return mode_ != Mode::OFF || requested > 0; }
Mode mode() { return mode_; }

static void say(const char* word) {
  flash = word;
  flashAt = millis();
  dirty = true;
}

static void releaseMouse() {
  if (leftDown || laser) hid::mouseButtons(0);
  leftDown = laser = scrolling = false;
}

static void startMode(Mode m) {
  mode_ = m;
  motion::setFast(true);
  frame.reset();
  still.reset();  // keeps the learned gyro bias
  pointer.configure(set_.sens, set_.invX, set_.invY);
  pointer.reset();
  scroller.configure(set_.scroll, set_.invScroll);
  knob.reset();
  remote::ButtonTiming t;
  if (m == Mode::PRESENTER) {
    t.holdMsA = 300;    // hold A: laser
    t.holdMsB = 800;    // hold B: restart the timer
    t.doubleMsB = 300;  // double B: black screen
    timer.reset();
    slide = 0;
  } else if (m == Mode::MEDIA) {
    t.holdMsA = t.holdMsB = 700;
  } else {
    t.holdMsB = 300;    // hold B: tilt to scroll
  }
  buttons.configure(t);
  buttons.reset(M5.BtnA.isPressed(), M5.BtnB.isPressed());  // the press that opened it doesn't count
  paused = scrolling = leftDown = laser = black = false;
  mediaAction = airui::NONE;
  flash = "";
  lastCount = motion::sampleCount();
  lastUs = motion::sampleMicros();
  lastActivity = millis();
  connectedAt = millis();
  wasConnected = hid::connected();
  shown = {false, 0};
  shownSecond = UINT32_MAX;
  dirty = true;
  ui::wake(m == Mode::PRESENTER ? 120000 : SCREEN_MS);
  Log.printf("Remote mode: %s\n", modeName(m));
}

static void endMode(const char* why) {
  releaseMouse();
  Log.printf("Remote mode %s ended (%s)\n", modeName(mode_), why);
  mode_ = Mode::OFF;
  motion::setFast(false);
  ui::wake(SCREEN_MS);
}

// --- Actions -------------------------------------------------------------------------------

static void tapKey(uint8_t key) {
  if (hid::connected()) hid::tap(Combo{0, key});
}

static void onMouseButton(remote::Btn b, uint32_t now) {
  using remote::Btn;
  switch (b) {
    case Btn::A_PRESS:
      if (paused) { paused = false; say("Pointing"); break; }  // A wakes a paused mouse (no click)
      leftDown = true;
      hid::mouseButtons(MOUSE_BUTTON_LEFT);
      pointer.freeze(now);
      break;
    case Btn::A_RELEASE:
      if (!leftDown) break;
      leftDown = false;
      hid::mouseButtons(0);
      pointer.freeze(now);
      break;
    case Btn::B_CLICK:
      if (paused) break;
      pointer.freeze(now);
      hid::mouseButtons((leftDown ? MOUSE_BUTTON_LEFT : 0) | MOUSE_BUTTON_RIGHT);
      hid::mouseButtons(leftDown ? MOUSE_BUTTON_LEFT : 0);
      break;
    case Btn::B_HOLD:
      if (paused) break;
      scrolling = true;
      scroller.begin();
      break;
    case Btn::B_RELEASE:
      scrolling = false;
      pointer.reset();
      break;
    case Btn::CHORD:
      paused = !paused;
      if (paused) releaseMouse();
      pointer.reset();
      say(paused ? "Paused" : "Pointing");
      break;
    default: break;
  }
}

static void onPresenterButton(remote::Btn b, uint32_t now) {
  using remote::Btn;
  switch (b) {
    case Btn::A_CLICK:
      tapKey(set_.pageKeys ? KEY_PAGE_DOWN : KEY_RIGHT);
      timer.start(now);
      slide++;
      say("Next");
      break;
    case Btn::A_HOLD:
      laser = true;
      pointer.reset();
      timer.start(now);
      break;
    case Btn::A_RELEASE:
      laser = false;
      break;
    case Btn::B_CLICK:
      tapKey(set_.pageKeys ? KEY_PAGE_UP : KEY_LEFT);
      timer.start(now);
      slide--;
      say("Back");
      break;
    case Btn::B_DOUBLE:
      tapKey(KEY_B);  // Keynote, PowerPoint and Google Slides: B = black screen (again = back)
      black = !black;
      say(black ? "Black" : "Back on");
      break;
    case Btn::B_HOLD:
      timer.reset();
      slide = 0;
      say("Timer reset");
      break;
    case Btn::CHORD:
      timer.toggle(now);
      say(timer.running() ? "Timer on" : "Timer paused");
      break;
    default: break;
  }
}

static void media(uint16_t usage, uint8_t action) {
  hid::media(usage);
  mediaAction = action;
  flashAt = millis();
  dirty = true;
}

static void onMediaButton(remote::Btn b) {
  using remote::Btn;
  if (b == Btn::A_CLICK) media(MEDIA_PLAY_PAUSE, airui::PLAY_PAUSE);
  else if (b == Btn::B_CLICK) media(MEDIA_MUTE, airui::MUTE);
}

// --- Motion ---------------------------------------------------------------------------------

static int frameSide() {
  switch (motion::facing().current()) {
    case orientation::Orient::PORTRAIT_FLIPPED: return 2;
    case orientation::Orient::LANDSCAPE_RIGHT:  return 1;
    case orientation::Orient::LANDSCAPE_LEFT:   return -1;
    default:                                    return 0;
  }
}

static void onSample(uint32_t now, float dt) {
  float a[3], g[3];
  motion::accel(a[0], a[1], a[2]);
  motion::gyro(g[0], g[1], g[2]);
  frame.feedAccel(a, dt);
  bool isStill = still.feed(now, a, g, dt);
  float b[3];
  still.bias(b);
  float gc[3] = {g[0] - b[0], g[1] - b[1], g[2] - b[2]};
  airmotion::Rates r = frame.rates(gc, frameSide());
  if (!isStill) lastActivity = now;

  if (mode_ == Mode::MEDIA) {
    switch (knob.feed(now, dt, r)) {
      case airmotion::KnobEvent::VOLUME_UP:   media(MEDIA_VOLUME_UP, airui::VOL_UP); break;
      case airmotion::KnobEvent::VOLUME_DOWN: media(MEDIA_VOLUME_DOWN, airui::VOL_DOWN); break;
      case airmotion::KnobEvent::NEXT:        media(MEDIA_NEXT_TRACK, airui::NEXT); break;
      case airmotion::KnobEvent::PREVIOUS:    media(MEDIA_PREV_TRACK, airui::PREVIOUS); break;
      default: break;
    }
    return;
  }
  if (scrolling) {
    int w, p;
    scroller.feed(dt, r.yaw, r.pitch, w, p);
    if (w || p) hid::mouseMove(0, 0, w, p);
    return;
  }
  bool moving = mode_ == Mode::MOUSE ? !paused : laser;
  if (!moving || isStill) {
    pointer.reset();
    return;
  }
  int dx, dy;
  pointer.feed(now, dt, r.yaw, r.pitch, dx, dy);
  if ((dx || dy) && !hid::mouseMove(dx, dy)) pointer.giveBack(dx, dy);
}

// --- Screen ---------------------------------------------------------------------------------

static void draw(uint32_t now) {
  airui::View v;
  v.screen = mode_ == Mode::PRESENTER ? airui::Screen::PRESENTER : mode_ == Mode::MEDIA ? airui::Screen::MEDIA
                                                                                        : airui::Screen::MOUSE;
  v.connected = hid::connected();
  v.mouseUnknown = mode_ == Mode::MOUSE && v.connected && !hid::mouseKnown() && now - connectedAt > MOUSE_UNKNOWN_MS;
  v.battery = battery::level();
  v.charging = battery::charging();
  v.sens = set_.sens;
  v.state = paused ? airui::PAUSED : scrolling ? airui::SCROLLING : leftDown ? airui::DRAGGING
          : still.stillFor(now) > RESTING_MS ? airui::RESTING : airui::POINTING;
  v.elapsedMs = timer.elapsedMs(now);
  v.targetMin = set_.talkMin;
  v.phase = (uint8_t)timer.phase(now, set_.talkMin);
  v.timerRunning = timer.running();
  v.laser = laser;
  v.black = black;
  v.slide = slide;
  v.lastAction = mediaAction;
  v.knob = knob.partial() / airmotion::Knob::STEP_DEG;
  float t = flashAt ? (now - flashAt) / (float)FLASH_MS : 1;
  v.flashT = t > 1 ? 1 : t;
  v.flash = v.flashT < 1 ? flash : String();
  airui::draw(v);
}

// --- Loop -----------------------------------------------------------------------------------

bool loop() {
  if (requested >= 0) {
    Mode m = (Mode)requested;
    requested = -1;
    if (m != mode_) {
      if (mode_ != Mode::OFF) endMode("switched");
      if (m != Mode::OFF) startMode(m);
    }
  }
  if (mode_ == Mode::OFF) return false;
  const uint32_t now = millis();

  // Buttons
  bool a = M5.BtnA.isPressed(), b = M5.BtnB.isPressed();
  buttons.feed(now, a, b);
  for (remote::Btn e = buttons.next(); e != remote::Btn::NONE; e = buttons.next()) {
    lastActivity = now;
    dirty = true;
    ui::wake(mode_ == Mode::PRESENTER ? 120000 : SCREEN_MS);  // a press still does its job with the screen off
    if (e == remote::Btn::CHORD_HOLD) {
      endMode("A+B held");
      return false;
    }
    if (mode_ == Mode::MOUSE) onMouseButton(e, now);
    else if (mode_ == Mode::PRESENTER) onPresenterButton(e, now);
    else onMediaButton(e);
  }

  // Motion: each new IMU sample once (motion.cpp reads them at 100 Hz meanwhile)
  uint32_t count = motion::sampleCount();
  if (count != lastCount) {
    uint32_t us = motion::sampleMicros();
    float dt = (us - lastUs) / 1e6f;
    if (dt <= 0 || dt > 0.1f) dt = 0.01f;  // the first one, or after a long stall
    lastCount = count;
    lastUs = us;
    onSample(now, dt);
  }

  // Connection: when the Mac (re)connects, give it a moment to subscribe to the mouse
  bool conn = hid::connected();
  if (conn != wasConnected) {
    wasConnected = conn;
    connectedAt = now;
    if (!conn) releaseMouse();
    dirty = true;
  }

  // Lying still (or the presenter left alone) long enough: back to Home
  bool talking = mode_ == Mode::PRESENTER && timer.running();
  if (!talking && now - lastActivity > (mode_ == Mode::PRESENTER ? IDLE_OFF_TALK_MS : IDLE_OFF_MS)) {
    endMode("idle");
    return false;
  }

  // The screen follows how the stick is held, like the reading screens
  orientation::Screen sc =
      orientation::choose(motion::facing(), ::settings::autoRotate(), ::settings::landscape(), ::settings::flipped());
  if (sc != shown) {
    shown = sc;
    ui::setOrientation(sc.flipped, sc.side);
    dirty = true;
  }
  if (talking || laser) ui::keepAwake();
  uint32_t second = timer.elapsedMs(now) / 1000;
  if (mode_ == Mode::PRESENTER && second != shownSecond) { shownSecond = second; dirty = true; }
  if (flashAt && now - flashAt < FLASH_MS + 100) dirty = true;  // the flash fades
  if (mode_ == Mode::MEDIA && fabsf(knob.partial()) > 1) dirty = true;
  static uint32_t resting = 0;  // the "Resting" state shows up on its own
  uint32_t r = still.stillFor(now) > RESTING_MS;
  if (r != resting) { resting = r; dirty = true; }
  if (mode_ == Mode::MOUSE && !hid::mouseKnown() && conn && now - connectedAt > MOUSE_UNKNOWN_MS &&
      now - connectedAt < MOUSE_UNKNOWN_MS + 200)
    dirty = true;

  // Drawing takes ~20 ms: not while the pointer moves fast, at most ~11 times a second
  if (dirty && ui::awake() && now - lastDraw >= DRAW_MIN_MS) {
    lastDraw = now;
    dirty = false;
    draw(now);
  }
  ui::tick(true);
  return true;
}

// --- Protocol -------------------------------------------------------------------------------

String stateJson() {
  String j = "\"air\":{\"mode\":\"" + String(modeName(mode_)) + "\"";
  j += ",\"sens\":" + String(set_.sens);
  j += String(",\"invX\":") + (set_.invX ? "true" : "false");
  j += String(",\"invY\":") + (set_.invY ? "true" : "false");
  j += ",\"scroll\":" + String(set_.scroll);
  j += String(",\"invScroll\":") + (set_.invScroll ? "true" : "false");
  j += ",\"talkMin\":" + String(set_.talkMin);
  j += String(",\"keys\":\"") + (set_.pageKeys ? "pages" : "arrows") + "\"";
  j += String(",\"mouseKnown\":") + (hid::mouseKnown() ? "true" : "false");
  return j + "}";
}

static bool flagArg(const String& v, bool cur) { return v.length() ? (v == "1" || v == "true" || v == "on") : cur; }

bool command(const String& action, const std::function<String(const char*)>& arg, int& status, String& json) {
  if (action == "mode") {  // name=mouse|presenter|media|home
    Mode m;
    if (!parseMode(arg("name"), m)) {
      status = 400;
      json = "{\"ok\":false,\"error\":\"Mode must be mouse, presenter, media or home\"}";
      return true;
    }
    request(m);
    status = 200;
    json = "{\"ok\":true,\"mode\":\"" + String(modeName(m)) + "\"}";
    return true;
  }
  if (action == "air") {  // sens, invx, invy, scroll, invscroll, talk, keys (any of them)
    Settings s = set_;
    auto num = [&](const char* n, int lo, int hi, int cur, bool& bad) {
      String v = arg(n);
      if (!v.length()) return cur;
      int x = v.toInt();
      if (x < lo || x > hi || (x == 0 && v != "0")) bad = true;
      return x;
    };
    bool bad = false;
    s.sens = num("sens", 1, 10, s.sens, bad);
    s.scroll = num("scroll", 1, 10, s.scroll, bad);
    s.talkMin = num("talk", 0, 600, s.talkMin, bad);
    s.invX = flagArg(arg("invx"), s.invX);
    s.invY = flagArg(arg("invy"), s.invY);
    s.invScroll = flagArg(arg("invscroll"), s.invScroll);
    String k = arg("keys");
    if (k.length()) {
      if (k == "pages") s.pageKeys = true;
      else if (k == "arrows") s.pageKeys = false;
      else bad = true;
    }
    if (bad) {
      status = 400;
      json = "{\"ok\":false,\"error\":\"sens and scroll 1-10, talk 0-600 minutes, keys arrows or pages\"}";
      return true;
    }
    save(s);
    Log.printf("Remote settings: sensitivity %u, scroll %u, talk %u min\n", s.sens, s.scroll, s.talkMin);
    status = 200;
    json = "{\"ok\":true," + stateJson() + "}";
    return true;
  }
  return false;
}

}  // namespace air
