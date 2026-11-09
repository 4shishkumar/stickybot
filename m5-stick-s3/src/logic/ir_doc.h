// One remote's file on the stick (firmware 2.5.0): /ir/<device>.json in LittleFS (src/ir/store.cpp).
//
//   {"v":1,"dev":"tv","buttons":[{"n":"power","c":"nec:4:8","f":1},
//                                {"n":"cool 24","c":"raw:38:3400,1700,420,1300:0123..."}]}
//
// v: the format (1); dev: the device id (= the file name); buttons in the user's order: n the
// name (irnames::buttonName), c the code string (ircodec::parse), f 1 = a favourite. The stick
// never trusts the file (or the Mac): every name and code is checked when it is read, and the
// limits (irnames::MAX_BUTTONS, MAX_FILE) hold. Pure C++ with ArduinoJson (native-testable).
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <string>
#include <vector>
#include "ir_names.h"

namespace irdoc {

constexpr int FORMAT = 1;

struct Button {
  std::string name;
  std::string code;
  bool fav = false;
};

struct Device {
  std::string id;
  std::vector<Button> buttons;
  int find(const std::string& name) const;     // index, -1 if missing
  // adds or replaces (same name) a button; false + why when the device is full or a value is bad
  bool put(const std::string& name, const std::string& code, bool fav, const char*& why);
  bool rename(const std::string& from, const std::string& to, const char*& why);
  bool remove(const std::string& name);
  std::string nextName() const;                // "button 3"
};

// The file's text -> device (checked). why: a short reason when it's refused
bool read(const char* json, size_t len, Device& out, const char*& why);
std::string write(const Device& d);

// A code string the stick can send (ircodec::parse; needs scratch room for a raw code)
bool validCode(const std::string& code);

}  // namespace irdoc
