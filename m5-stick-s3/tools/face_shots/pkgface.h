// pkgface: the reference renderer for face packages (docs/faces/FORMAT.md). It reads a
// compiled face (.sfb, made by tools/faces/validate.py --compile) and draws it with the
// stick's own drawing kit (ui_draw.h, faces/common.h: the same fonts, text fitting, icons,
// status icons and urgent card), so what face_shots saves is what the stick will show.
//
// Host-only for now (face_shots). Written to move into src/faces/ as the firmware engine:
// no file system, no exceptions, every index checked, the manifest walked as ArduinoJson
// (the firmware version would parse it once into structs: see docs/faces/PLAN.md).
#pragma once
#include <Arduino.h>
#include <ArduinoJson.h>
#include <map>
#include <string>
#include <vector>
#include "ui.h"
#include "logic/world.h"

namespace pkgface {

struct Image {
  int w = 0, h = 0, fw = 0, frames = 1;
  int clear = -1;                 // the transparent palette index, -1 none
  std::vector<uint16_t> pal;      // RGB565
  std::vector<uint8_t> px;        // one index per pixel
};

class Face {
 public:
  // Parses and checks a compiled face; false (and why) if it isn't one this renderer takes.
  bool load(const uint8_t* data, size_t n, String& err);
  const char* id() const { return id_.c_str(); }
  // Draws Home: the face, then what the firmware always draws over it (status, urgent card).
  void draw(const ui::Dash& d, const world::Snapshot& w, bool wide, uint32_t now);
  void kick(world::Kick k, uint32_t now);
  void setTilt(float x, float y) { tiltX_ = x; tiltY_ = y; }
  // The frame-rate tier (FORMAT.md, "Frame rate"): 0 = redraw only when something changes.
  uint32_t frameMs(bool onBattery, uint32_t idleMs, uint32_t now) const;

 private:
  struct Val {
    bool known = false, num = false;
    float n = 0;
    String s;
    bool truthy() const { return known && (num ? n != 0 : s.length() > 0); }
  };
  struct Ctx {
    const ui::Dash* d;
    const world::Snapshot* w;
    uint32_t now;
    bool wide;
    uint16_t bg;          // the layout's background (mix target)
    int ox = 0, oy = 0;   // the group's offset
    int inst = 0;         // the repeat's instance
  };

  Val get(const char* name, const Ctx& c) const;
  float numOf(JsonVariantConst v, float dflt, const Ctx& c) const;
  bool cond(JsonVariantConst v, const Ctx& c, int depth = 0) const;
  String expand(const char* tmpl, const Ctx& c) const;
  uint16_t color(JsonVariantConst v, uint16_t dflt) const;
  bool colorOf(const char* s, uint16_t& out) const;
  // the tweens' sums for an element: x / y offsets, mix, colour, frame
  struct Tw {
    float dx = 0, dy = 0, mix = -1;
    int frame = -1;
    bool hasColor = false;
    uint16_t color = 0, mixTo = 0;
  };
  Tw tweens(JsonVariantConst anims, const Ctx& c, uint16_t base) const;
  void element(JsonVariantConst e, Ctx c, int depth);
  void drawOne(JsonVariantConst e, const Ctx& c, int x, int y, const Tw& t);
  void drawImage(const Image& img, int frame, int x, int y, int scale, bool flip, float mix, uint16_t mixTo,
                 bool tile);
  void events(const ui::Dash& d, bool urgentOn, uint32_t now);
  void fire(int ev, uint32_t now);

  JsonDocument doc_;
  std::string id_;
  std::vector<Image> images_;
  std::map<std::string, int> imageIndex_;
  std::map<std::string, uint16_t> colors_;
  uint32_t eventAt_[32] = {0};    // when each event last fired (0 never)
  uint32_t reactMs_[32] = {0};
  float tiltX_ = 0, tiltY_ = 0;
  int lastMinute_ = -2, lastHour_ = -2;
  int lastNight_ = -1, lastUrgent_ = -1;
  bool animated_ = false, seconds_ = false, onAnims_ = false;
  int motion_ = 0;                // 0 static, 1 calm, 2 lively
};

// The event names (FORMAT.md, "Events"), index = the bit in eventAt_.
extern const char* const EVENTS[];
int eventIndex(const char* name);   // -1 unknown

}  // namespace pkgface
