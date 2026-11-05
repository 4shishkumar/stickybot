// Just enough of Arduino for the screens on a Mac (face_shots): String, Print, millis()...
#pragma once
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdarg.h>
#include <algorithm>
#include <string>

using std::max;
using std::min;

#ifndef PI
#define PI 3.14159265358979323846f
#endif
#define constrain(x, lo, hi) ((x) < (lo) ? (lo) : ((x) > (hi) ? (hi) : (x)))

uint32_t millis();
uint32_t esp_random();
inline void delay(uint32_t) {}

class String {
 public:
  String() {}
  String(const char* s) : s_(s ? s : "") {}
  String(const std::string& s) : s_(s) {}
  String(char c) : s_(1, c) {}
  String(int v) : s_(std::to_string(v)) {}
  String(unsigned v) : s_(std::to_string(v)) {}
  String(long v) : s_(std::to_string(v)) {}
  String(unsigned long v) : s_(std::to_string(v)) {}
  String(float v, int digits = 2) { char b[32]; snprintf(b, sizeof b, "%.*f", digits, v); s_ = b; }
  String(double v, int digits = 2) { char b[32]; snprintf(b, sizeof b, "%.*f", digits, v); s_ = b; }
  unsigned length() const { return (unsigned)s_.size(); }
  bool isEmpty() const { return s_.empty(); }
  const char* c_str() const { return s_.c_str(); }
  char operator[](unsigned i) const { return i < s_.size() ? s_[i] : 0; }
  char operator[](int i) const { return (*this)[(unsigned)i]; }
  operator const char*() const { return s_.c_str(); }  // for LovyanGFX (its String overloads need ARDUINO)
  String& operator+=(const String& o) { s_ += o.s_; return *this; }
  String& operator+=(const char* o) { s_ += o ? o : ""; return *this; }
  String& operator+=(char c) { s_ += c; return *this; }
  bool operator==(const String& o) const { return s_ == o.s_; }
  bool operator==(const char* o) const { return s_ == (o ? o : ""); }
  bool operator!=(const String& o) const { return s_ != o.s_; }
  bool operator!=(const char* o) const { return !(*this == o); }
  String substring(unsigned a) const { return a < s_.size() ? String(s_.substr(a)) : String(); }
  String substring(unsigned a, unsigned b) const {
    if (b > s_.size()) b = s_.size();
    return a < b ? String(s_.substr(a, b - a)) : String();
  }
  int indexOf(char c, unsigned from = 0) const { auto p = s_.find(c, from); return p == std::string::npos ? -1 : (int)p; }
  int indexOf(const String& t, unsigned from = 0) const { auto p = s_.find(t.s_, from); return p == std::string::npos ? -1 : (int)p; }
  bool startsWith(const String& t) const { return s_.compare(0, t.s_.size(), t.s_) == 0; }
  bool endsWith(const String& t) const {
    return s_.size() >= t.s_.size() && s_.compare(s_.size() - t.s_.size(), t.s_.size(), t.s_) == 0;
  }
  long toInt() const { return atol(s_.c_str()); }
  void trim() {
    size_t a = s_.find_first_not_of(" \t\r\n"), b = s_.find_last_not_of(" \t\r\n");
    s_ = a == std::string::npos ? "" : s_.substr(a, b - a + 1);
  }
  void replace(const String& a, const String& b) {
    for (size_t p = 0; (p = s_.find(a.s_, p)) != std::string::npos; p += b.s_.size()) s_.replace(p, a.s_.size(), b.s_);
  }
  std::string::const_iterator begin() const { return s_.begin(); }
  std::string::const_iterator end() const { return s_.end(); }
  friend String operator+(const String& a, const String& b) { return String(a.s_ + b.s_); }
  friend String operator+(const String& a, const char* b) { return String(a.s_ + (b ? b : "")); }
  friend String operator+(const char* a, const String& b) { return String(std::string(a ? a : "") + b.s_); }
  friend String operator+(const String& a, char b) { return String(a.s_ + b); }
  friend String operator+(const String& a, int b) { return a + String(b); }
  friend String operator+(const String& a, unsigned b) { return a + String(b); }
  friend String operator+(const String& a, long b) { return a + String(b); }

 private:
  std::string s_;
};

class Print {
 public:
  virtual ~Print() {}
  virtual size_t write(uint8_t c) = 0;
  virtual size_t write(const uint8_t* buf, size_t len) {
    for (size_t i = 0; i < len; i++) write(buf[i]);
    return len;
  }
  size_t printf(const char* fmt, ...) {
    char b[512];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(b, sizeof b, fmt, ap);
    va_end(ap);
    return write((const uint8_t*)b, n < 0 ? 0 : std::min((size_t)n, sizeof b - 1));
  }
  size_t print(const String& s) { return write((const uint8_t*)s.c_str(), s.length()); }
  size_t println(const String& s = "") { return print(s) + write('\n'); }
};
