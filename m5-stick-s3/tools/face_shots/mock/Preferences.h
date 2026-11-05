// Preferences (NVS) for face_shots: remembers nothing.
#pragma once
#include <Arduino.h>

class Preferences {
 public:
  bool    begin(const char*, bool = false) { return true; }
  void    end() {}
  uint8_t getUChar(const char*, uint8_t def = 0) { return def; }
  size_t  putUChar(const char*, uint8_t) { return 1; }
  size_t  getBytesLength(const char*) { return 0; }
  size_t  getBytes(const char*, void*, size_t) { return 0; }
  size_t  putBytes(const char*, const void*, size_t n) { return n; }
};
