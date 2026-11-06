#include "clockface.h"

namespace clockface {

static const char* const HOURS[] = {"twelve", "one", "two",   "three", "four", "five",
                                    "six",    "seven", "eight", "nine", "ten",  "eleven"};
static const char* const FIVES[] = {"", "five", "ten", "quarter", "twenty", "twenty-five", "half"};

// The hour said with "past" / "to": midnight for 0, else 1-12 ("quarter past twelve" at noon)
static std::string hourWord(int h) { return h % 24 == 0 ? "midnight" : HOURS[h % 12]; }

Words words(int hour, int minute) {
  hour = ((hour % 24) + 24) % 24;
  minute = ((minute % 60) + 60) % 60;
  Words w;
  int m = minute - minute % 5;
  if (minute % 5 >= 3) {
    w.lead = "nearly";
    m += 5;
  } else if (minute % 5) {
    w.lead = "just after";
  }
  if (m == 60) {
    m = 0;
    hour = (hour + 1) % 24;
  }
  if (m == 0) {
    w.hour = hour == 0 ? "midnight" : hour == 12 ? "noon" : HOURS[hour % 12];
    w.text = hour == 0 || hour == 12 ? w.hour : w.hour + " o'clock";
  } else if (m <= 30) {
    w.hour = hourWord(hour);
    w.text = std::string(FIVES[m / 5]) + " past " + w.hour;
  } else {
    w.hour = hourWord(hour + 1);
    w.text = std::string(FIVES[(60 - m) / 5]) + " to " + w.hour;
  }
  return w;
}

Hands hands(int minuteOfDay, float seconds) {
  minuteOfDay = ((minuteOfDay % 1440) + 1440) % 1440;
  if (seconds < 0) seconds = 0;
  if (seconds >= 60) seconds = 59.999f;
  const float minute = minuteOfDay % 60 + seconds / 60.0f;
  return {((minuteOfDay / 60) % 12 * 60 + minute) * 0.5f, minute * 6.0f, seconds * 6.0f};
}

std::vector<std::string> stack(const std::string& s, int maxW, text::Width w, void* ctx) {
  // the pieces: the words, a word too wide on its own broken after its hyphens
  std::vector<std::string> pieces;
  size_t i = 0;
  while (i < s.size()) {
    while (i < s.size() && s[i] == ' ') i++;
    size_t j = s.find(' ', i);
    if (j == std::string::npos) j = s.size();
    if (j > i) {
      std::string word = s.substr(i, j - i);
      if (w(word, ctx) <= maxW || word.find('-') == std::string::npos) {
        pieces.push_back(word);
      } else {
        size_t a = 0, h;
        while ((h = word.find('-', a)) != std::string::npos && h + 1 < word.size()) {
          pieces.push_back(word.substr(a, h + 1 - a));
          a = h + 1;
        }
        pieces.push_back(word.substr(a));
      }
    }
    i = j;
  }
  // greedy lines; the rest of a broken word starts a new one
  std::vector<std::string> lines;
  for (const std::string& p : pieces) {
    if (!lines.empty() && lines.back().back() != '-') {
      std::string joined = lines.back() + " " + p;
      if (w(joined, ctx) <= maxW) {
        lines.back() = joined;
        continue;
      }
    }
    lines.push_back(p);
  }
  return lines;
}

}  // namespace clockface
