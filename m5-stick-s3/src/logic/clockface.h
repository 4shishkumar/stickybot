// The clock faces' arithmetic: the time in words ("ten past nine", the Words face), the
// hands of a dial (the Analog face) and words stacked into lines that fit a narrow screen.
// Pure C++ (native-testable).
#pragma once
#include <string>
#include <vector>
#include "text.h"

namespace clockface {

// The time in words, to the nearest five minutes: lead "" (on the five), "just after"
// (1-2 minutes past it) or "nearly" (3-4 minutes before the next one).
//   9:00 "nine o'clock"   9:15 "quarter past nine"   9:30 "half past nine"   9:40 "twenty to ten"
//   9:41 "just after" + "twenty to ten"   9:58 "nearly" + "ten o'clock"
//   0:00 "midnight"  12:00 "noon"  0:10 "ten past midnight"  23:45 "quarter to midnight"
struct Words {
  std::string lead;  // "", "just after", "nearly"
  std::string text;  // "twenty-five to eleven"
  std::string hour;  // the hour's word in it ("eleven", "midnight"), to set it apart
};
Words words(int hour, int minute);  // hour 0-23, minute 0-59

// Degrees clockwise from 12 o'clock. The hour and minute hands move smoothly (the minute
// hand with the seconds, the hour hand with the minutes).
struct Hands {
  float hour, minute, second;
};
Hands hands(int minuteOfDay, float seconds);

// The words in lines no wider than maxW, filled greedily; a word that is too wide on its
// own breaks after a hyphen ("twenty-" / "five"), else it stands alone (cut by the caller).
std::vector<std::string> stack(const std::string& s, int maxW, text::Width w, void* ctx);

}  // namespace clockface
