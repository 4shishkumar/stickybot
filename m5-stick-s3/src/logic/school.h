// The Aquarium's fish: one per running Claude Code session plus a couple of ambient ones.
// A new session swims in from the side; a fish grows the longer its session runs; when the
// session finishes its fish leaps out of the water and is gone. Pure C++ (native-testable).
//
// Coordinates are px inside the water: x 0..w, y 0 (the surface) .. h (the sand).
#pragma once
#include <stdint.h>

namespace school {

constexpr int MAX_FISH = 8;
constexpr int AMBIENT = 2;
constexpr uint32_t JUMP_MS = 1500;   // up to the surface, the leap, the splash
constexpr int HUES = 8;              // colour palette size (hueFor)

enum class State : uint8_t { SWIM, JUMP };

struct Fish {
  bool     on = false;
  bool     ambient = false;
  uint16_t id = 0;          // the session's id (ambient: 0)
  State    st = State::SWIM;
  float    x = 0, y = 0;    // centre
  float    vx = 0, vy = 0;  // px/s
  float    tx = 0, ty = 0;  // where it's heading
  float    size = 1;        // 0.6 (ambient) .. 1.6 (a session running for an hour+)
  uint8_t  hue = 0;
  uint32_t born = 0, jumpAt = 0;
  float    jumpX = 0;       // where it left the water
  char     label[13] = {0}; // the project
  bool facingRight() const { return vx >= 0; }
};

struct Session {
  uint16_t    id;
  const char* project;
  uint32_t    secs;
};

float   sizeFor(uint32_t secs);         // 0.8 at the start .. 1.6 after an hour
uint8_t hueFor(const char* project);    // the same colour for the same project, 0..HUES-1

class School {
 public:
  void begin(uint32_t seed, int w, int h, uint32_t now);  // also adds the ambient fish
  void resize(int w, int h);                               // the screen turned
  // The sessions running now: new ones swim in, missing ones jump and leave.
  void sync(const Session* s, int n, uint32_t now);
  // dt in s; drift -1..1 pushes everyone towards that side (the tilt); sleepy: slow, low.
  void step(float dt, float drift, bool sleepy, uint32_t now);
  void scatter();                                          // a shake: everyone darts off
  void clear(uint32_t now);                                // back to just the ambient fish

  int         size() const { return MAX_FISH; }
  const Fish& at(int i) const { return fish[i]; }
  int         count() const;     // fish in the water (or jumping)
  int         sessions() const;  // ... that belong to a session
  // While jumping: 0..1 through the leap (negative = still swimming up to it).
  static float jumpPhase(const Fish& f, uint32_t now);

 private:
  Fish     fish[MAX_FISH];
  int      w = 100, h = 100;
  uint32_t rng = 1;
  uint32_t rand();
  float    frand(float lo, float hi);
  void     pickTarget(Fish& f, bool sleepy);
  int      freeSlot() const;
};

}  // namespace school
