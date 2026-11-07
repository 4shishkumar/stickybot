#include "clock.h"
#include <time.h>

namespace clockfmt {

bool local(int64_t epoch, long tzOffset, Local& out) {
  if (epoch < VALID_AFTER) return false;
  time_t t = (time_t)(epoch + tzOffset);
  tm r;
  gmtime_r(&t, &r);
  out = {r.tm_hour, r.tm_min, r.tm_wday, r.tm_mday, r.tm_mon};
  return true;
}

std::string time12(const Local& t) {
  int h = t.hour % 12;
  char buf[8];
  snprintf(buf, sizeof buf, "%d:%02d", h ? h : 12, t.min);
  return buf;
}

std::string time24(const Local& t) {
  char buf[8];
  snprintf(buf, sizeof buf, "%d:%02d", t.hour, t.min);
  return buf;
}

std::string ampm(const Local& t) { return t.hour < 12 ? "AM" : "PM"; }

std::string shortDate(const Local& t) {
  static const char* D[] = {"Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"};
  static const char* M[] = {"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"};
  return std::string(D[t.wday]) + " " + std::to_string(t.mday) + " " + M[t.mon];
}

std::string longDate(const Local& t) {
  static const char* D[] = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"};
  static const char* M[] = {"January", "February", "March", "April", "May", "June", "July", "August",
                            "September", "October", "November", "December"};
  return std::string(D[t.wday]) + " " + std::to_string(t.mday) + " " + M[t.mon];
}

bool isNight(int hour) { return hour >= 22 || (hour >= 0 && hour < 6); }

}  // namespace clockfmt
