// face_shots: draws the Home faces on the Mac, with the stick's own drawing code (ui.cpp,
// src/faces/, LovyanGFX into a sprite), and saves PNGs. No stick needed.
//
//   make -C tools/face_shots            demo frames -> tools/face_shots/out/
//   make -C tools/face_shots FACE=aquadark   one face's demo
//   make -C tools/face_shots previews   the dashboard's face pictures -> mac-agent/.../static/faces/
//   make -C tools/face_shots pkg PKG=<face package>   a face package (docs/faces/FORMAT.md) in
//                                       several states -> out/pkg/ (and with PREVIEWS=1 its previews/)
#include <Arduino.h>
#include <M5Unified.h>
#include <ArduinoJson.h>
#include <sys/stat.h>
#include <vector>
#include <zlib.h>
#include "battery.h"
#include "combo.h"
#include "netlog.h"
#include "settings.h"
#include "sound.h"
#include "ui.h"
#include "ui_draw.h"
#include "faces/faces.h"
#include "faces/pkg/engine.h"
#include "air_ui.h"

// --- The bits of the firmware the screens touch -------------------------------------
static uint32_t now_ = 1000;
uint32_t millis() { return now_; }
uint32_t esp_random() {
  static uint32_t x = 2463534242u;
  x ^= x << 13; x ^= x >> 17; x ^= x << 5;
  return x;
}
FakeM5 M5;
namespace lgfx { inline namespace v1 {
unsigned long millis(void) { return now_; }
unsigned long micros(void) { return now_ * 1000ul; }
void delay(unsigned long) {}
void delayMicroseconds(unsigned int) {}
void gpio_hi(uint32_t) {}
void gpio_lo(uint32_t) {}
bool gpio_in(uint32_t) { return false; }
void pinMode(int_fast16_t, pin_mode_t) {}
void lgfxPinMode(int_fast16_t, pin_mode_t) {}
}}
size_t TeeLog::write(uint8_t c) { return fputc(c, stderr) != EOF; }
size_t TeeLog::write(const uint8_t* b, size_t n) { return fwrite(b, 1, n, stderr); }
TeeLog Log;
namespace battery {
int  etaMinutes() { return 200; }
bool usbPower() { return false; }
bool full() { return false; }
int  recent(uint8_t[RECENT], bool[RECENT]) { return 0; }
}
namespace settings { bool soundOn() { return false; } }
namespace sound { bool play(const String&) { return true; } }
String comboToString(const Combo&) { return "Option+Cmd"; }
// No installed face packages here (no flash): `shots pkg` draws one with the engine directly
namespace faces { namespace pkg {
void     begin() {}
int      count() { return 0; }
Face&    at(int) { return faces::at(0); }
void     drawOverlay(bool) {}
uint32_t overlayFrameMs() { return 0; }
}}

// --- PNG -------------------------------------------------------------------------
static void be32(std::vector<uint8_t>& v, uint32_t x) { for (int i = 3; i >= 0; i--) v.push_back(x >> (i * 8)); }
static void chunk(FILE* f, const char* type, const std::vector<uint8_t>& data) {
  std::vector<uint8_t> v;
  be32(v, data.size());
  v.insert(v.end(), type, type + 4);
  v.insert(v.end(), data.begin(), data.end());
  be32(v, crc32(0, v.data() + 4, v.size() - 4));
  fwrite(v.data(), 1, v.size(), f);
}
struct Shot {
  int w, h;
  std::vector<uint8_t> rgb;
};
static std::vector<Shot> shots;  // for the contact sheet

static void writePng(const std::string& path, int w, int h, const std::vector<uint8_t>& rgb) {
  std::vector<uint8_t> raw;
  for (int y = 0; y < h; y++) {
    raw.push_back(0);
    raw.insert(raw.end(), rgb.begin() + y * w * 3, rgb.begin() + (y + 1) * w * 3);
  }
  uLongf n = compressBound(raw.size());
  std::vector<uint8_t> z(n);
  compress2(z.data(), &n, raw.data(), raw.size(), 9);
  z.resize(n);
  FILE* f = fopen(path.c_str(), "wb");
  if (!f) { fprintf(stderr, "can't write %s\n", path.c_str()); return; }
  const uint8_t sig[] = {137, 80, 78, 71, 13, 10, 26, 10};
  fwrite(sig, 1, 8, f);
  std::vector<uint8_t> ih;
  be32(ih, w);
  be32(ih, h);
  ih.insert(ih.end(), {8, 2, 0, 0, 0});
  chunk(f, "IHDR", ih);
  chunk(f, "IDAT", z);
  chunk(f, "IEND", {});
  fclose(f);
  printf("%s\n", path.c_str());
}

static void savePng(const std::string& path) {
  M5Canvas& s = ui::canvas;
  Shot sh{(int)s.width(), (int)s.height(), {}};
  for (int y = 0; y < sh.h; y++)
    for (int x = 0; x < sh.w; x++) {
      uint16_t c = s.readPixel(x, y);
      sh.rgb.push_back(((c >> 11) & 31) * 255 / 31);
      sh.rgb.push_back(((c >> 5) & 63) * 255 / 63);
      sh.rgb.push_back((c & 31) * 255 / 31);
    }
  shots.push_back(sh);
  const int k = getenv("SCALE") ? atoi(getenv("SCALE")) : 1;  // SCALE=3: bigger, to see the pixels
  if (k > 1) {
    std::vector<uint8_t> big(sh.w * k * sh.h * k * 3);
    for (int y = 0; y < sh.h * k; y++)
      for (int x = 0; x < sh.w * k; x++) memcpy(&big[(y * sh.w * k + x) * 3], &sh.rgb[((y / k) * sh.w + x / k) * 3], 3);
    writePng(path, sh.w * k, sh.h * k, big);
  } else {
    writePng(path, sh.w, sh.h, sh.rgb);
  }
}

// All the shots on one page (240 x 240 cells), to look at them side by side.
static void sheet(const std::string& path, size_t from, size_t to, int cols) {
  const int cell = 244;
  int n = (int)(to - from), rows = (n + cols - 1) / cols, W = cols * cell, H = rows * cell;
  std::vector<uint8_t> rgb(W * H * 3, 40);
  for (int i = 0; i < n; i++) {
    const Shot& s = shots[from + i];
    int ox = (i % cols) * cell + (cell - s.w) / 2, oy = (i / cols) * cell + (cell - s.h) / 2;
    for (int y = 0; y < s.h; y++)
      memcpy(&rgb[((oy + y) * W + ox) * 3], &s.rgb[y * s.w * 3], s.w * 3);
  }
  writePng(path, W, H, rgb);
}

static ui::Dash base() {
  ui::Dash d{};
  d.name = "StickyBot";
  d.slot = 1;
  d.bt = 0;
  d.battery = 76;
  d.charging = false;
  d.clock = "9:41";
  d.ampm = "AM";
  d.date = "Fri 6 Nov";
  d.longDate = "Friday 6 November";
  d.wday = 5;
  d.mday = 6;
  d.mon = 10;
  d.minute = 9 * 60 + 41;
  d.second = (now_ / 1000) % 60;
  d.agentLinked = true;
  d.cardKind = "fact";
  d.cardA = "Did you know?";
  d.cardB = "Honey never spoils.";
  d.wxValid = true;
  d.wxTemp = 24;
  d.wxHi = 27;
  d.wxLo = 18;
  d.wxIcon = "clear";
  d.wxCond = "clear";
  d.wxDay = true;
  d.fw = "2.0.0";
  return d;
}

static std::string slug(const String& s) {
  std::string o;
  for (char c : s) o += isalnum((unsigned char)c) ? (char)tolower(c) : '-';
  return o;
}

// Every face's demo, a picture of each step (a little into it, once things have moved)
static void demo(const std::string& dir, const std::string& face) {
  mkdir(dir.c_str(), 0755);
  faces::demoStart(face.c_str());
  String last;
  uint32_t changedAt = 0;
  bool shot = true;
  int n = 0;
  while (faces::demoActive()) {
    now_ += 40;
    ui::Dash d;
    int side;
    String cap;
    if (!faces::demoFrame(base(), d, side, cap)) break;
    ui::setOrientation(false, side);
    ui::setTilt(side ? 1 : 0, side ? 0 : 1);  // held still, that way up
    ui::drawFaceDemo(d, cap);
    if (cap != last) { last = cap; changedAt = now_; shot = false; }
    if (!shot && now_ - changedAt >= 1600) {
      shot = true;
      char num[8];
      snprintf(num, sizeof num, "%02d-", ++n);
      savePng(dir + "/" + num + slug(cap) + ".png");
    }
  }
  // a contact sheet per face (a new face starts where the caption's face name changes)
  size_t start = 0;
  for (size_t i = 1; i <= shots.size(); i++) {
    if (i < shots.size() && i - start < 24) continue;
    sheet(dir + "/sheet-" + std::to_string(start + 1) + ".png", start, i, 6);
    start = i;
  }
}

// The dashboard's pictures: each face on a calm day with a little going on
static void previews(const std::string& dir) {
  mkdir(dir.c_str(), 0755);
  JsonDocument doc;
  deserializeJson(doc, R"({"mergedToday":2,"merged":27,"ci":"green","asks":0,"reviews":0,
    "claude":[{"p":"web","s":1500,"id":11},{"p":"api","s":200,"id":12}],
    "pet":{"xp":420,"streak":6,"acc":3,"seed":20261106,"greens":14,"towers":[9,21]}})");
  faces::onWorld(doc.as<JsonObjectConst>());
  for (int i = 0; i < faces::count(); i++) {
    faces::select(faces::at(i).id());
    for (int side : {0, 1}) {
      ui::setOrientation(false, side);
      for (int f = 0; f < 190; f++) {  // 7.6 s: the fish swim in, the bot walks a little
        now_ += 40;
        ui::setTilt(side ? 1 : 0, side ? 0 : 1);
        ui::drawPage(ui::Page::HOME, base());
      }
      savePng(dir + "/" + faces::at(i).id() + (side ? "-wide" : "") + ".png");
    }
  }
}

// --- Face packages (docs/faces/FORMAT.md): the stick's own engine (src/faces/pkg/) ---------

struct PkgState {
  const char* name;
  int minute;              // -1: the time isn't known yet
  const char* ci;
  world::Kick kick;        // fired `after` ms before the shot
  bool urgent;
  int battery;
};

static const PkgState PKG_STATES[] = {
    {"day", 9 * 60 + 41, "green", world::Kick::NONE, false, 76},
    {"night", 23 * 60 + 30, "green", world::Kick::NONE, false, 76},
    {"merged", 14 * 60 + 5, "green", world::Kick::MERGED, false, 76},
    {"ci-red", 16 * 60 + 52, "red", world::Kick::CI_RED, false, 76},
    {"urgent", 10 * 60 + 15, "green", world::Kick::NONE, true, 76},
    {"no-time", -1, "none", world::Kick::NONE, false, 12},
};

static ui::Dash pkgDash(const PkgState& s) {
  ui::Dash d = base();
  d.minute = s.minute;
  if (s.minute < 0) {
    d.clock = d.ampm = d.date = d.longDate = "";
    d.second = -1;
    d.wday = -1;
    d.agentLinked = false;
    d.wxValid = false;
  } else {
    int h = s.minute / 60, m = s.minute % 60;
    d.clock = String(h % 12 ? h % 12 : 12) + ":" + (m < 10 ? "0" : "") + String(m);
    d.ampm = h < 12 ? "AM" : "PM";
    d.wxDay = h >= 7 && h < 19;
  }
  d.battery = s.battery;
  if (s.urgent) {
    d.cardUrgent = true;
    d.cardKind = "approval";
    d.cardA = "Approve Bash?";
    d.cardB = "web: npm test -- --watch";
  }
  return d;
}

static world::Snapshot pkgWorld(const PkgState& s) {
  world::Snapshot w;
  w.valid = s.minute >= 0;
  w.mergedToday = 3;
  w.merged = 41;
  w.ci = world::parseCi(s.ci);
  w.asks = s.urgent ? 1 : 0;
  w.reviews = 1;
  w.nSessions = 2;
  w.xp = 420;
  w.streak = 6;
  return w;
}

static bool readFile(const std::string& path, std::vector<uint8_t>& blob) {
  FILE* f = fopen(path.c_str(), "rb");
  if (!f) { fprintf(stderr, "can't read %s\n", path.c_str()); return false; }
  uint8_t buf[4096];
  size_t n;
  while ((n = fread(buf, 1, sizeof buf, f)) > 0) blob.insert(blob.end(), buf, buf + n);
  fclose(f);
  return true;
}

// One shot of a package face in a state: run it from a fresh start for `ms` (its animations
// move; the state's event fires `after` ms before the end), then save.
static bool pkgShot(const std::vector<uint8_t>& blob, const PkgState& s, int side, const std::string& path,
                    uint32_t ms = 2400, uint32_t after = 700) {
  static faces::pkg::Engine face;  // (one, as on the stick: loading frees the last one's arena)
  String err;
  if (face.load(blob.data(), blob.size(), err) != faces::pkg::LoadError::NONE) {
    fprintf(stderr, "invalid face: %s\n", err.c_str());
    return false;
  }
  ui::setOrientation(false, side);
  ui::setTilt(side ? 1 : 0, side ? 0 : 1);
  const ui::Dash d = pkgDash(s);
  const world::Snapshot w = pkgWorld(s);
  const uint32_t start = now_;
  bool kicked = false;
  for (uint32_t t = 0; t <= ms; t += 40) {
    now_ = start + t;
    if (s.kick != world::Kick::NONE && !kicked && t >= ms - after) {
      face.kick(s.kick, now_);
      kicked = true;
    }
    ui::drawPage(ui::Page::HOME, d);  // the canvas in this orientation (the built-in face under it)
    face.draw(d, w, side != 0, now_);
  }
  savePng(path);
  return true;
}

// shots pkg <face.sfb> <dir>: every state, both ways up, and a contact sheet.
// shots pkgpreview <face.sfb> <dir>: the package's previews (a calm day).
static int pkg(const std::string& file, const std::string& dir, bool preview) {
  std::vector<uint8_t> blob;
  if (!readFile(file, blob)) return 1;
  mkdir(dir.c_str(), 0755);
  if (preview) {
    for (int side : {0, 1})
      if (!pkgShot(blob, PKG_STATES[0], side, dir + (side ? "/landscape.png" : "/portrait.png"))) return 1;
    return 0;
  }
  std::string id = file.substr(file.find_last_of('/') + 1);
  id = id.substr(0, id.find('.'));
  const size_t first = shots.size();
  const int n = sizeof PKG_STATES / sizeof PKG_STATES[0];
  for (int side : {0, 1})
    for (int i = 0; i < n; i++)
      if (!pkgShot(blob, PKG_STATES[i], side, dir + "/" + id + "-" + PKG_STATES[i].name + (side ? "-wide" : "") + ".png"))
        return 1;
  sheet(dir + "/" + id + "-sheet.png", first, shots.size(), n);
  return 0;
}

// shots fuzz <face.sfb> [n]: the engine's loader against n broken copies of a good face (bytes
// changed, cut short, the manifest's text garbled); whatever loads is drawn both ways up. It
// must never crash or read out of bounds (build with SAN=1 for AddressSanitizer).
static int fuzz(const std::string& file, int n) {
  std::vector<uint8_t> blob;
  if (!readFile(file, blob)) return 1;
  static faces::pkg::Engine face;
  const ui::Dash d = pkgDash(PKG_STATES[0]);
  const world::Snapshot w = pkgWorld(PKG_STATES[0]);
  uint32_t seed = 12345;
  auto rnd = [&]() { seed = seed * 1103515245u + 12345u; return seed >> 8; };
  int loaded = 0;
  for (int i = 0; i < n; i++) {
    std::vector<uint8_t> b = blob;
    const int kind = rnd() % 4, changes = 1 + rnd() % 8;
    for (int k = 0; k < changes; k++) {
      const size_t at = rnd() % b.size();
      if (kind == 0) b[at] = rnd();                                 // any byte
      else if (kind == 1) b[at] ^= 1 << (rnd() % 8);                // a bit
      else if (kind == 2) b.resize(std::max<size_t>(1, at));        // cut short
      else {                                                        // the manifest: digits and quotes
        const size_t m = 16 + rnd() % std::min<size_t>(b.size() - 16, blob[8] | blob[9] << 8);
        const char* junk = "0123456789-\"{}[],:9e9";
        if (m < b.size()) b[m] = junk[rnd() % strlen(junk)];
      }
    }
    String err;
    std::vector<uint8_t> exact(b);  // its own allocation: ASan sees reads past the end
    if (face.load(exact.data(), exact.size(), err) != faces::pkg::LoadError::NONE) continue;
    loaded++;
    for (int side : {0, 1}) {
      ui::setOrientation(false, side);
      now_ += 40;
      face.kick(world::Kick::MERGED, now_);
      face.draw(d, w, side != 0, now_);
    }
  }
  printf("fuzz: %d broken copies, %d still loaded and were drawn, no crash\n", n, loaded);
  return 0;
}

// The remote modes' screens (air_ui.cpp), portrait and landscape -> <dir>/air-*.png + a sheet
static void airShots(const std::string& dir) {
  mkdir(dir.c_str(), 0755);
  size_t first = shots.size();
  struct Case { const char* name; airui::View v; };
  std::vector<Case> cases;
  auto add = [&](const char* name, airui::View v) { cases.push_back({name, v}); };
  airui::View m;
  m.battery = 78;
  add("mouse", m);
  m.state = airui::SCROLLING; add("mouse-scroll", m);
  m.state = airui::PAUSED; m.flash = "Paused"; m.flashT = 0.2f; add("mouse-paused", m);
  m.flash = ""; m.flashT = 1;
  m.state = airui::RESTING; add("mouse-resting", m);
  m.state = airui::POINTING; m.mouseUnknown = true; add("mouse-repair", m);
  airui::View p;
  p.screen = airui::Screen::PRESENTER; p.battery = 64; p.targetMin = 20;
  add("presenter-idle", p);
  p.phase = 1; p.timerRunning = true; p.elapsedMs = 7 * 60000 + 42000; p.slide = 9; add("presenter", p);
  p.phase = 2; p.elapsedMs = 18 * 60000 + 5000; p.laser = true; p.slide = 23; add("presenter-warn-laser", p);
  p.laser = false; p.phase = 3; p.elapsedMs = 21 * 60000 + 30000; p.black = true; add("presenter-over-black", p);
  airui::View d;
  d.screen = airui::Screen::MEDIA; d.battery = 51;
  add("media", d);
  d.lastAction = airui::VOL_UP; d.knob = 0.6f; d.flashT = 0.3f; add("media-volume", d);
  d.lastAction = airui::NEXT; d.knob = 0; add("media-next", d);
  d.connected = false; add("media-offline", d);
  for (int side : {0, 1}) {
    ui::setOrientation(false, side);
    for (auto& c : cases) {
      airui::draw(c.v);
      savePng(dir + "/air-" + c.name + (side ? "-wide" : "") + ".png");
    }
  }
  sheet(dir + "/air-sheet.png", first, shots.size(), 7);
}

int main(int argc, char** argv) {
  ui::begin();
  std::string mode = argc > 1 ? argv[1] : "demo";
  std::string dir = argc > 2 ? argv[2] : "out";
  if (mode == "pkg" || mode == "pkgpreview") return argc > 3 ? pkg(argv[2], argv[3], mode == "pkgpreview") : 2;
  if (mode == "fuzz") return argc > 2 ? fuzz(argv[2], argc > 3 ? atoi(argv[3]) : 5000) : 2;
  if (mode == "check" && argc > 2) {  // shots check <face.sfb>: would the stick take it? (as an install does)
    std::vector<uint8_t> blob;
    if (!readFile(argv[2], blob)) return 1;
    static faces::pkg::Engine face;
    String err;
    const bool ok = face.load(blob.data(), blob.size(), err) == faces::pkg::LoadError::NONE;
    printf("%s\n", ok ? "ok" : (String("invalid: ") + err).c_str());
    return ok ? 0 : 3;
  }
  if (mode == "air") { airShots(dir); return 0; }
  if (mode == "previews") previews(dir);
  else demo(dir, argc > 3 ? argv[3] : "all");
  return 0;
}
