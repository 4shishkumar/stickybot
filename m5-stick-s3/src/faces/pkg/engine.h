// Face packages (docs/faces/FORMAT.md): the engine that draws a compiled face (.sfb).
//
// load() checks the file as if it came from a stranger (it does): the container and every
// image (logic/sfb.cpp), then the manifest, parsed once with a nesting limit into compact
// structs: elements with their colours resolved to RGB565, bindings interned, templates
// compiled, conditions as a node array, tweens (logic/facefmt.cpp). Unknown types or fields,
// a value out of range or a missing reference make the face invalid: it is never half-drawn.
// Everything lives in one arena (PSRAM on the stick) that unload() frees whole.
//
// draw() is the whole Home screen: the face, then what the firmware always draws over it (the
// status icons and the urgent card). Used by the stick (package.cpp: PackageFace) and by
// face_shots on the Mac, so the pictures the catalog shows are what the stick draws.
#pragma once
#include <Arduino.h>
#include <new>
#include "../../ui.h"
#include "../../logic/facefmt.h"
#include "../../logic/world.h"

namespace faces {
namespace pkg {

// Memory for one face, in blocks, freed whole (PSRAM on the stick when there is some).
class Arena {
 public:
  ~Arena() { clear(); }
  void* alloc(size_t n);   // 4-aligned, zeroed; nullptr when out of memory
  template <typename T>
  T* make(size_t n = 1) {
    T* p = (T*)alloc(sizeof(T) * (n ? n : 1));
    if (p)
      for (size_t i = 0; i < (n ? n : 1); i++) new (p + i) T();
    return p;
  }
  char* copy(const char* s, size_t n);
  void  clear();
  size_t bytes() const { return bytes_; }

 private:
  struct Block {
    Block* next;
    size_t used, cap;
  };
  Block* head_ = nullptr;
  size_t bytes_ = 0;
};

struct Image {
  uint16_t w = 0, h = 0, fw = 0, frames = 1, colors = 0;
  int16_t  clear = -1;           // the transparent palette index, -1 none
  uint16_t* pal = nullptr;       // RGB565
  uint8_t*  px = nullptr;        // one index per pixel, frame by frame (fw x h each)
};

struct Elem;

struct Layout {
  bool     grad = false, gradH = false;
  uint16_t bg = 0, bg2 = 0;           // a colour, or the gradient's two ends
  uint8_t  status = 0;                // 0 light, 1 dark, 2 pill
  int16_t  card[4] = {0, 0, 0, 0};    // the urgent card
  Elem*    elems = nullptr;
  uint8_t  n = 0;
};

// What a face says about itself (the pickers, @faces), read without loading it.
struct Info {
  char id[13] = {0}, name[25] = {0}, about[81] = {0}, version[16] = {0}, minFirmware[16] = {0};
  uint8_t format = 0;
  int8_t  motion = 0;                 // facefmt::Tier
};

// Why load() refused a face, for the install replies (FORMAT.md section 18).
enum class LoadError : uint8_t { NONE, INVALID, FORMAT, FIRMWARE, MEMORY };

// Reads a compiled face's header and the manifest's id / name / about / version /
// minFirmware / format / motion (a filtered parse: cheap). false and why if it isn't a face.
bool peek(const uint8_t* p, size_t n, Info& info, String& err);

class Engine {
 public:
  ~Engine() { unload(); }
  // Parses and checks a compiled face (the bytes can go afterwards: everything is copied).
  LoadError load(const uint8_t* p, size_t n, String& err);
  void unload();
  bool loaded() const { return loaded_; }
  const Info& info() const { return info_; }
  size_t memory() const { return arena_.bytes(); }

  // The whole Home screen on ui::canvas (wide: the 240 x 135 layout).
  void draw(const ui::Dash& d, const world::Snapshot& w, bool wide, uint32_t now);
  void kick(world::Kick k, uint32_t now);
  void setTilt(float x, float y) { tiltX_ = x; tiltY_ = y; }
  void reset();  // forget the events (a fresh start: after a demo)
  // How often it wants a frame (ms), 0 = only when something changes (FORMAT.md section 11).
  uint32_t frameMs(bool onBattery, uint32_t idleMs, uint32_t now) const;
  // The guard rail: a frame took us; true if that lowered the tier (it was too slow).
  bool frameTook(uint32_t us, uint32_t now);
  int  tier() const { return tier_; }

 private:
  struct Ctx;
  struct Tw;
  void element(const Elem& e, const Ctx& c, int depth);
  void drawOne(const Elem& e, const Ctx& c, int x, int y, const Tw& t);
  void drawImage(const Image& im, int frame, int x, int y, int scale, bool flip, float mix, uint16_t mixTo, bool tile);
  void events(const ui::Dash& d, bool urgentOn, uint32_t now);
  void fire(int ev, uint32_t now);

  Arena    arena_;
  Info     info_;
  bool     loaded_ = false;
  Layout   layouts_[2];               // portrait, landscape
  Image*   images_ = nullptr;
  int      nImages_ = 0;
  facefmt::Cond* conds_ = nullptr;
  uint8_t* row_ = nullptr;            // a scaled image row, for pushImage
  uint32_t eventAt_[facefmt::EV_COUNT] = {0};
  uint32_t reactMs_[facefmt::EV_COUNT] = {0};
  uint32_t busyMs_[facefmt::EV_COUNT] = {0};  // how long a reaction keeps a static face moving (0: unused)
  float    tiltX_ = 0, tiltY_ = 0;
  int      lastMinute_ = -2, lastHour_ = -2, lastNight_ = -1, lastUrgent_ = -1;
  bool     seconds_ = false;
  int8_t   tier_ = 0;
  facefmt::Guard guard_;
};

}  // namespace pkg
}  // namespace faces
