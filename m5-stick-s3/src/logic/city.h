// StickyWorld's city: one building per merged PR, a tower per release. Every building
// comes from (seed, index) alone, so the skyline is the same on every draw and after a
// restart, and a new PR only adds one at the right. Pure C++ (native-testable).
#pragma once
#include <stdint.h>

namespace city {

enum Roof : uint8_t { FLAT, PEAK, ANTENNA, DOME, ROOF_COUNT };

struct Building {
  uint8_t  w;        // width in cells (3-6)
  uint8_t  h;        // height in floors (3-12; a tower 15-18)
  uint8_t  roof;     // Roof
  uint8_t  color;    // palette index 0-5
  uint16_t windows;  // which windows are lit at night (one bit per window)
  bool     tower;    // a release
};

Building building(uint32_t seed, int index);
Building tower(uint32_t seed, int index);

// A building placed on the skyline: its left edge and width in px (narrower when squeezed).
struct Placed {
  Building b;
  int16_t  x;
  int16_t  w;
};

constexpr int MIN_W = 2;  // cells: squeezed buildings stay at least this wide

// Lays out `count` buildings, with tower i standing after building towersAt[i], across
// `width` px with `cell` px per cell. While they fit they keep their width and a 1-cell gap;
// then they squeeze (down to MIN_W, no gap); after that the oldest scroll off the left so
// the newest always show. Fills out[] left to right, returns how many.
int layout(uint32_t seed, int count, const int* towersAt, int nTowers, int width, int cell, Placed out[], int max);

}  // namespace city
