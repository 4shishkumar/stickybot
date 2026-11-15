// Native substitutes for Arduino's byte-oriented Print sinks; no hardware or home I/O.
#pragma once
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string>
#include <cstdarg>
#include <cstdio>

class String {
 public:
  String() = default;
  String(const char* value) : value_(value ? value : "") {}
  String(const std::string& value) : value_(value) {}
  size_t length() const { return value_.size(); }
  const char* c_str() const { return value_.c_str(); }
  String& operator+=(char c) { value_ += c; return *this; }
  bool operator==(const char* s) const { return value_ == s; }
 private:
  std::string value_;
};

class Print {
 public:
  virtual ~Print() = default;
  virtual size_t write(uint8_t) = 0;
  virtual size_t write(const uint8_t* bytes, size_t size) {
    for (size_t i = 0; i < size; ++i) write(bytes[i]);
    return size;
  }
  size_t print(char c) { return write((uint8_t)c); }
  size_t print(const char* s) { return write((const uint8_t*)s, std::strlen(s)); }
  size_t print(const String& s) { return write((const uint8_t*)s.c_str(), s.length()); }
  size_t println() { return print("\r\n"); }
  size_t println(const char* s) { size_t n = print(s); return n + println(); }
  size_t println(const String& s) { size_t n = print(s); return n + println(); }
  size_t printf(const char* format, ...) {
    char buffer[2048];
    va_list args;
    va_start(args, format);
    int n = std::vsnprintf(buffer, sizeof buffer, format, args);
    va_end(args);
    size_t len = n > 0 ? (size_t)n : 0;
    if (len >= sizeof buffer) len = sizeof buffer - 1;
    return write((const uint8_t*)buffer, len);
  }
};

class FakeSerial : public Print {
 public:
  std::string output;
  using Print::write;
  size_t write(uint8_t c) override { output += (char)c; return 1; }
};
inline FakeSerial Serial;
