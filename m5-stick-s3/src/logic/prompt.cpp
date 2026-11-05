#include "prompt.h"

namespace prompt {

const char* decision(bool ask, Input in) {
  if (in == Input::A) return "once";
  if (in == Input::B) return "deny";
  return ask ? "dismiss" : "deny";
}

std::string button(char key, const std::string& label, const char* fallback, size_t maxChars) {
  std::string out(1, key);
  out += "  ";
  const std::string& s = label.empty() ? std::string(fallback) : label;
  size_t chars = 0, i = 0;
  while (i < s.size() && chars < maxChars) {
    size_t n = 1;
    unsigned char c = s[i];
    if (c >= 0xF0) n = 4;
    else if (c >= 0xE0) n = 3;
    else if (c >= 0xC0) n = 2;
    if (i + n > s.size()) break;  // a cut-off character at the end
    out.append(s, i, n);
    i += n;
    chars++;
  }
  return out;
}

}  // namespace prompt
