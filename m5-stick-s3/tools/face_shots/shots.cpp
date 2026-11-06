// face_shots: draws the Home faces on the Mac, with the stick's own drawing code (ui.cpp,
// src/faces/, LovyanGFX into a sprite), and saves PNGs. No stick needed.
//
//   make -C tools/face_shots            demo frames -> tools/face_shots/out/
//   make -C tools/face_shots previews   the dashboard's face pictures -> mac-agent/.../static/faces/
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
static void demo(const std::string& dir) {
  mkdir(dir.c_str(), 0755);
  faces::demoStart("all");
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

int main(int argc, char** argv) {
  ui::begin();
  std::string mode = argc > 1 ? argv[1] : "demo";
  std::string dir = argc > 2 ? argv[2] : "out";
  if (mode == "previews") previews(dir);
  else demo(dir);
  return 0;
}
