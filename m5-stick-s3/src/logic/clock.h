// Local time from the Mac agent's clock (epoch + UTC offset), formatted for the
// screens. Pure C++ (native-testable).
#pragma once
#include <stdint.h>
#include <string>

namespace clockfmt {

constexpr int64_t VALID_AFTER = 1700000000;  // an epoch before this = "not set yet"

struct Local { int hour, min, wday, mday, mon; };  // wday 0 = Sunday, mon 0 = January
bool        local(int64_t epoch, long tzOffset, Local& out);  // false if the clock isn't set
std::string time12(const Local& t);    // "3:05", "12:00"
std::string time24(const Local& t);    // "15:05", "0:00" (the 24-hour setting)
std::string ampm(const Local& t);      // "AM" / "PM"
std::string shortDate(const Local& t); // "Tue 20 Oct"
std::string longDate(const Local& t);  // "Tuesday 20 October"
bool        isNight(int hour);          // 22:00-06:00 (hour < 0 = unknown: false)

}  // namespace clockfmt
