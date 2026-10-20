#include "text.h"

namespace text {

static std::string rtrim(std::string s) {
  while (!s.empty() && s.back() == ' ') s.pop_back();
  return s;
}

// Drops a trailing partial UTF-8 sequence so cutting never leaves a broken character.
static void popChar(std::string& s) {
  if (s.empty()) return;
  size_t i = s.size() - 1;
  while (i > 0 && ((unsigned char)s[i] & 0xC0) == 0x80) i--;
  s.erase(i);
}

std::string ellipsize(const std::string& s, int maxW, Width w, void* ctx) {
  if (w(s, ctx) <= maxW) return s;
  std::string t = s;
  while (t.size() > 1 && w(t + "...", ctx) > maxW) popChar(t);
  return rtrim(t) + "...";
}

std::vector<std::string> wrap(const std::string& s, int maxLines, int maxW, Width w, void* ctx) {
  std::vector<std::string> out;
  if (maxLines <= 0) return out;
  std::string line;
  size_t start = 0;
  bool more = false;
  while (start < s.size()) {
    size_t sp = s.find(' ', start);
    std::string word = s.substr(start, sp == std::string::npos ? std::string::npos : sp - start);
    std::string trial = line.empty() ? word : line + " " + word;
    if (!line.empty() && w(trial, ctx) > maxW) {
      out.push_back(line);
      line = word;
      if ((int)out.size() == maxLines) { more = true; break; }
    } else {
      line = trial;
    }
    if (sp == std::string::npos) break;
    start = sp + 1;
  }
  if (!more && !line.empty()) out.push_back(line);
  if (more) {  // more text than fits: the last line ends in "..."
    std::string& l = out.back();
    while (l.size() > 1 && w(l + "...", ctx) > maxW) popChar(l);
    l = rtrim(l) + "...";
  }
  return out;
}

}  // namespace text
