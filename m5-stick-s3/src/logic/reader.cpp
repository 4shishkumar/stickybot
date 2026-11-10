#include "reader.h"
#include <math.h>

namespace reader {

static bool cont(unsigned char c) { return (c & 0xC0) == 0x80; }

void capUtf8(std::string& s, size_t maxBytes) {
  if (s.size() <= maxBytes) return;
  size_t n = maxBytes;
  while (n > 0 && cont((unsigned char)s[n])) n--;  // s[n] starts the first character dropped
  s.erase(n);
}

// Breaks one word wider than maxW into pieces that fit (at least one character each).
static void breakWord(const std::string& word, int maxW, text::Width w, void* ctx, std::vector<std::string>& out,
                      std::string& line) {
  size_t i = 0;
  while (i < word.size()) {
    size_t j = i + 1;
    while (j < word.size() && cont((unsigned char)word[j])) j++;
    std::string trial = line + word.substr(i, j - i);
    if (!line.empty() && w(trial, ctx) > maxW) {
      out.push_back(line);
      line.clear();
      continue;  // the same character on a new line
    }
    line = trial;
    i = j;
  }
}

static void wrapParagraph(const std::string& p, int maxW, text::Width w, void* ctx, std::vector<std::string>& out) {
  std::string line;
  size_t start = 0;
  while (start <= p.size()) {
    size_t sp = p.find(' ', start);
    std::string word = p.substr(start, sp == std::string::npos ? std::string::npos : sp - start);
    if (!word.empty()) {
      std::string trial = line.empty() ? word : line + " " + word;
      if (w(trial, ctx) <= maxW) {
        line = trial;
      } else {
        if (!line.empty()) out.push_back(line);
        line.clear();
        if (w(word, ctx) <= maxW) line = word;
        else breakWord(word, maxW, w, ctx, out, line);
      }
    }
    if (sp == std::string::npos) break;
    start = sp + 1;
  }
  if (!line.empty()) out.push_back(line);
}

std::vector<std::string> wrapAll(const std::string& s, int maxW, text::Width w, void* ctx) {
  std::vector<std::string> out;
  size_t start = 0;
  bool blank = false;  // a blank line since the last paragraph
  while (start <= s.size()) {
    size_t nl = s.find('\n', start);
    std::string p = s.substr(start, nl == std::string::npos ? std::string::npos : nl - start);
    for (char& c : p)
      if (c == '\t' || c == '\r') c = ' ';
    bool empty = p.find_first_not_of(' ') == std::string::npos;
    if (empty) {
      blank = !out.empty();
    } else {
      if (blank) out.push_back("");
      blank = false;
      wrapParagraph(p, maxW, w, ctx, out);
    }
    if (nl == std::string::npos) break;
    start = nl + 1;
  }
  return out;
}

void Assembly::begin(uint32_t rid, int parts) {
  rid_ = rid;
  parts_ = parts > 0 ? parts : 0;
  next_ = 0;
  text_.clear();
}

Assembly::Step Assembly::add(uint32_t rid, int i, const char* t) {
  if (!parts_ || rid != rid_ || next_ >= parts_) return Step::IGNORED;
  if (i != next_) {  // a piece lost on the way: the short text stays
    parts_ = 0;
    text_.clear();
    return Step::BROKEN;
  }
  if (t) text_ += t;
  capUtf8(text_, MAX_BYTES);
  next_++;
  return next_ == parts_ ? Step::COMPLETE : Step::ADDED;
}

void Pager::clamp() {
  if (visible < 1) visible = 1;
  if (top > maxTop()) top = maxTop();
  if (top < 0) top = 0;
}

void Pager::next() {
  if (!paged()) return;
  if (top >= maxTop()) {
    top = 0;
    return;
  }
  top += visible > 2 ? visible - 1 : visible;
  clamp();
}

bool Pager::scroll(int n) {
  int was = top;
  top += n;
  clamp();
  return top != was;
}

int Pager::pages() const {
  if (!paged()) return 1;
  int step = visible > 2 ? visible - 1 : visible;
  return 1 + (maxTop() + step - 1) / step;
}

int Pager::page() const {
  if (!paged()) return 1;
  if (top >= maxTop()) return pages();
  int step = visible > 2 ? visible - 1 : visible;
  return 1 + (top + step - 1) / step;
}

void Pager::thumb(int trackH, int& y, int& h) const {
  if (!paged()) {
    y = 0;
    h = trackH;
    return;
  }
  h = trackH * visible / total;
  if (h < 10) h = 10;
  if (h > trackH) h = trackH;
  y = (trackH - h) * top / maxTop();
}

float pitchDeg(float gy, float az) { return atan2f(gy, az) * 180.0f / (float)M_PI; }

int TiltScroll::feed(float dt, float deg) {
  last_ = deg;
  if (!primed_) {
    primed_ = true;
    base_ = deg;
    acc_ = 0;
    return 0;
  }
  float d = base_ - deg;  // + = the top tipped away (flatter)
  float past = fabsf(d) - DEAD_DEG;
  if (past <= 0) {
    base_ += (deg - base_) * 0.02f;  // follows how it's held, slowly, only while still
    acc_ = 0;
    return 0;
  }
  float rate = past * LINES_PER_DEG;
  if (rate > MAX_RATE) rate = MAX_RATE;
  if (dt > 0.2f) dt = 0.2f;  // a long gap (a redraw, a send) isn't a jump
  acc_ += (d > 0 ? rate : -rate) * dt;
  int n = (int)truncf(acc_);
  acc_ -= n;
  return n;
}

}  // namespace reader
