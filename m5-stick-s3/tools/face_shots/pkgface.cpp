#include "pkgface.h"
#include "faces/common.h"
#include "logic/clockface.h"
#include "logic/pet.h"

namespace pkgface {

using namespace ui;

const char* const EVENTS[] = {"merged", "ci_red", "ci_green", "claude_start", "claude_done", "release",
                              "level_up", "review", "shake", "flick", "upside_down", "lift", "minute",
                              "hour", "dusk", "dawn", "urgent", nullptr};

int eventIndex(const char* name) {
  for (int i = 0; EVENTS[i]; i++)
    if (!strcmp(EVENTS[i], name)) return i;
  return -1;
}

static int eventForKick(world::Kick k) {
  switch (k) {
    case world::Kick::MERGED:       return 0;
    case world::Kick::CI_RED:       return 1;
    case world::Kick::CI_GREEN:     return 2;
    case world::Kick::CLAUDE_START: return 3;
    case world::Kick::CLAUDE_DONE:  return 4;
    case world::Kick::RELEASE:      return 5;
    case world::Kick::LEVEL_UP:     return 6;
    case world::Kick::REVIEW:       return 7;
    case world::Kick::SHAKE:        return 8;
    case world::Kick::FLICK:        return 9;
    case world::Kick::UPSIDE_DOWN:  return 10;
    case world::Kick::LIFT:         return 11;
    default:                        return -1;
  }
}

// --- Loading: the .sfb container (tools/faces/facepkg.py compile_face) --------------------

namespace {
constexpr uint32_t MAX_SFB = 64 * 1024;
constexpr int MAX_IMAGES = 16, MAX_SIDE = 240;
constexpr uint32_t MAX_PIXELS = 96 * 1024;

uint16_t u16(const uint8_t* p) { return p[0] | p[1] << 8; }
uint32_t u32(const uint8_t* p) { return p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }

bool unpackBits(const uint8_t* in, size_t n, std::vector<uint8_t>& out, size_t size) {
  size_t i = 0;
  while (out.size() < size && i < n) {
    uint8_t c = in[i++];
    if (c < 128) {
      if (i + c + 1 > n) return false;
      out.insert(out.end(), in + i, in + i + c + 1);
      i += c + 1;
    } else if (c > 128) {
      if (i >= n) return false;
      out.insert(out.end(), 257 - c, in[i++]);
    }
  }
  out.resize(size);
  return i <= n;
}
}  // namespace

bool Face::load(const uint8_t* p, size_t n, String& err) {
  images_.clear();
  imageIndex_.clear();
  colors_.clear();
  if (n < 16 || memcmp(p, "SFB1", 4) || n > MAX_SFB) { err = "not a compiled face"; return false; }
  if (u16(p + 4) != 1 || u16(p + 6) != 1) { err = "unsupported container / format version"; return false; }
  const uint32_t mlen = u32(p + 8);
  const int nimg = u16(p + 12);
  if (16 + mlen > n || nimg > MAX_IMAGES) { err = "bad sizes"; return false; }
  DeserializationError je = deserializeJson(doc_, (const char*)p + 16, mlen, DeserializationOption::NestingLimit(12));
  if (je) { err = String("manifest: ") + je.c_str(); return false; }
  size_t pos = 16 + mlen + ((4 - mlen % 4) % 4);
  uint32_t pixels = 0;
  for (int i = 0; i < nimg; i++) {
    if (pos + 16 > n) { err = "truncated image"; return false; }
    const uint8_t* h = p + pos;
    Image im;
    im.w = u16(h);
    im.h = u16(h + 2);
    im.fw = u16(h + 4);
    const int bits = h[6], nc = h[7] + 1, clear = h[8], enc = h[9];
    const uint32_t dlen = u32(h + 12);
    pos += 16;
    if ((bits != 1 && bits != 2 && bits != 4 && bits != 8) || im.w < 1 || im.h < 1 || im.w > MAX_SIDE ||
        im.h > MAX_SIDE || (im.fw && im.w % im.fw) || pos + nc * 2 + dlen > n || (1 << bits) < nc) {
      err = "bad image header";
      return false;
    }
    pixels += im.w * im.h;
    if (pixels > MAX_PIXELS) { err = "images too big"; return false; }
    im.frames = im.fw ? im.w / im.fw : 1;
    if (!im.fw) im.fw = im.w;
    im.clear = clear == 255 ? -1 : clear;
    for (int c = 0; c < nc; c++) im.pal.push_back(u16(p + pos + c * 2));
    pos += nc * 2 + nc % 2 * 2;  // the palette, padded to 4
    const size_t stride = (im.w * bits + 7) / 8;
    std::vector<uint8_t> packed;
    if (enc == 1) {
      if (!unpackBits(p + pos, dlen, packed, stride * im.h)) { err = "bad image data"; return false; }
    } else {
      if (dlen < stride * im.h) { err = "short image data"; return false; }
      packed.assign(p + pos, p + pos + stride * im.h);
    }
    pos += dlen + ((4 - dlen % 4) % 4);
    const int per = 8 / bits;
    im.px.resize(im.w * im.h);
    for (int y = 0; y < im.h; y++)
      for (int x = 0; x < im.w; x++) {
        uint8_t v = (packed[y * stride + x / per] >> (8 - bits * (x % per + 1))) & ((1 << bits) - 1);
        im.px[y * im.w + x] = v < nc ? v : 0;
      }
    images_.push_back(std::move(im));
  }
  JsonObjectConst m = doc_.as<JsonObjectConst>();
  id_ = m["id"] | "";
  for (JsonPairConst kv : m["images"].as<JsonObjectConst>()) {
    int i = kv.value()["i"] | -1;
    if (i < 0 || i >= (int)images_.size()) { err = "image index"; return false; }
    imageIndex_[kv.key().c_str()] = i;
  }
  for (JsonPairConst kv : m["colors"].as<JsonObjectConst>()) {
    const char* s = kv.value() | "";
    if (strlen(s) != 7 || s[0] != '#') { err = "colour"; return false; }
    long v = strtol(s + 1, nullptr, 16);
    colors_[kv.key().c_str()] = rgb(v >> 16, (v >> 8) & 255, v & 255);
  }
  for (int i = 0; EVENTS[i]; i++) reactMs_[i] = m["reactions"][EVENTS[i]] | 3000;
  const char* motion = m["motion"] | "static";
  motion_ = !strcmp(motion, "lively") ? 2 : !strcmp(motion, "calm") ? 1 : 0;
  // Does it show seconds / start anims on events? (the frame rate)
  // (the firmware engine would note these while parsing into structs)
  std::string all;
  serializeJson(doc_["layouts"], all);
  serializeJson(doc_["parts"], all);
  auto has = [&](const char* s) { return all.find(s) != std::string::npos; };
  seconds_ = has("time.ss") || has("time.second") || has("\"second\"");
  onAnims_ = has("\"on\":") || has("react.");
  memset(eventAt_, 0, sizeof eventAt_);
  return true;
}

// --- Frame rate -------------------------------------------------------------------------

uint32_t Face::frameMs(bool onBattery, uint32_t idleMs, uint32_t now) const {
  bool reacting = false;
  for (int i = 0; EVENTS[i]; i++)
    if (eventAt_[i] && now - eventAt_[i] < reactMs_[i] + 500) reacting = true;
  int tier = motion_;
  if (reacting && tier == 0) tier = 1;  // a static face moves while it reacts
  if (tier == 2) return !onBattery ? 40 : idleMs < 20000 ? 66 : 150;
  if (tier == 1) return !onBattery ? 100 : idleMs < 20000 ? 200 : 500;
  return seconds_ ? (onBattery ? 1000 : 200) : 0;
}

// --- Events -----------------------------------------------------------------------------

void Face::fire(int ev, uint32_t now) {
  if (ev >= 0 && ev < 32) eventAt_[ev] = now ? now : 1;
}

void Face::kick(world::Kick k, uint32_t now) { fire(eventForKick(k), now); }

void Face::events(const Dash& d, bool urgentOn, uint32_t now) {
  if (d.minute >= 0) {
    if (lastMinute_ != -2 && d.minute != lastMinute_) fire(eventIndex("minute"), now);
    const int hour = d.minute / 60;
    if (lastHour_ != -2 && hour != lastHour_) fire(eventIndex("hour"), now);
    lastMinute_ = d.minute;
    lastHour_ = hour;
  }
  const int night = faces::isNightMinute(d.minute) ? 1 : 0;
  if (lastNight_ >= 0 && night != lastNight_) fire(eventIndex(night ? "dusk" : "dawn"), now);
  lastNight_ = night;
  if (lastUrgent_ >= 0 && urgentOn && !lastUrgent_) fire(eventIndex("urgent"), now);
  lastUrgent_ = urgentOn;
}

// --- Bindings (FORMAT.md, "Data bindings") -------------------------------------------------

static const char* const MONTHS[] = {"January", "February", "March", "April", "May", "June", "July",
                                     "August", "September", "October", "November", "December"};

static String two(int v) { return v < 10 ? "0" + String(v) : String(v); }

Face::Val Face::get(const char* name, const Ctx& c) const {
  const Dash& d = *c.d;
  const world::Snapshot& w = *c.w;
  Val v;
  auto str = [&](const String& s, bool known = true) { v.known = known; v.s = known ? s : String(); return v; };
  auto num = [&](float n, bool known = true) {
    v.known = known;
    v.num = true;
    v.n = known ? n : 0;
    v.s = known ? String((long)lroundf(n)) : String();
    return v;
  };
  auto flag = [&](bool b) { return num(b ? 1 : 0); };
  const bool tk = d.minute >= 0;
  const int hour = tk ? d.minute / 60 : 0, minute = tk ? d.minute % 60 : 0, sec = d.second >= 0 ? d.second : 0;
  const String& ld = d.longDate;  // "Friday 6 November"
  const int sp1 = ld.indexOf(' '), sp2 = sp1 >= 0 ? ld.indexOf(' ', sp1 + 1) : -1;
  const String weekday = sp1 > 0 ? ld.substring(0, sp1) : String();
  const String day = sp2 > sp1 ? ld.substring(sp1 + 1, sp2) : String();
  const String month = sp2 > 0 ? ld.substring(sp2 + 1) : String();
  int monthNo = 0;
  for (int i = 0; i < 12; i++)
    if (month == MONTHS[i]) monthNo = i + 1;
  const bool dk = tk && weekday.length();
  const bool wk = d.agentLinked && w.valid;
  const String n(name);

  if (n == "time.known") return flag(tk);
  if (n == "time.hm") return str(d.clock, tk && d.clock.length());
  if (n == "time.hm24") return str(String(hour) + ":" + two(minute), tk);
  if (n == "time.h12") return str(String(hour % 12 ? hour % 12 : 12), tk);
  if (n == "time.h24") return str(String(hour), tk);
  if (n == "time.hh12") return str(two(hour % 12 ? hour % 12 : 12), tk);
  if (n == "time.hh24") return str(two(hour), tk);
  if (n == "time.mm") return str(two(minute), tk);
  if (n == "time.ss") return str(two(sec), tk && d.second >= 0);
  if (n == "time.ampm") return str(d.ampm, tk);
  if (n == "time.hour") return num(hour, tk);
  if (n == "time.minute") return num(minute, tk);
  if (n == "time.second") return num(sec, tk && d.second >= 0);
  if (n == "time.minuteOfDay") return num(d.minute, tk);
  if (n.startsWith("time.words")) {
    if (!tk) return str("", false);
    clockface::Words cw = clockface::words(hour, minute);
    if (n == "time.words") return str(cw.text.c_str());
    if (n == "time.wordsLead") return str(cw.lead.c_str());
    if (n == "time.wordsHour") return str(cw.hour.c_str());
  }
  if (n == "date.weekday") return str(weekday, dk);
  if (n == "date.dow") return str(weekday.substring(0, 3), dk);
  if (n == "date.day") return str(day, dk);
  if (n == "date.dd") return str(two(day.toInt()), dk);
  if (n == "date.month") return str(month, dk);
  if (n == "date.mon") return str(month.substring(0, 3), dk);
  if (n == "date.mm") return str(two(monthNo), dk && monthNo);
  if (n == "date.short") return str(d.date, d.date.length() > 0);
  if (n == "date.long") return str(d.longDate, d.longDate.length() > 0);
  if (n == "wx.valid") return flag(d.wxValid);
  if (n == "wx.temp") return num(d.wxTemp, d.wxValid);
  if (n == "wx.hi") return num(d.wxHi, d.wxValid);
  if (n == "wx.lo") return num(d.wxLo, d.wxValid);
  if (n == "wx.icon") return str(d.wxIcon, d.wxValid);
  if (n == "wx.cond") return str(d.wxCond, d.wxValid);
  if (n == "wx.day") return flag(d.wxValid ? d.wxDay : !faces::isNightMinute(d.minute));
  if (n == "dev.battery") return num(d.battery, d.battery >= 0);
  if (n == "dev.charging") return flag(d.charging);
  if (n == "dev.bt") return str(d.bt == 0 ? "connected" : d.bt == 1 ? "waiting" : "off");
  if (n == "dev.name") return str(d.name);
  if (n == "agent.linked") return flag(d.agentLinked);
  if (n == "card.kind") return str(d.cardKind, d.agentLinked);
  if (n == "card.a") return str(d.cardA, d.agentLinked);
  if (n == "card.b") return str(d.cardB, d.agentLinked);
  if (n == "card.urgent") return flag(d.agentLinked && d.cardUrgent);
  if (n == "urgent.on") return flag(faces::urgent(d).on);
  if (n == "work.known") return flag(wk);
  if (n == "work.asks") return num(w.asks, wk);
  if (n == "work.reviews") return num(w.reviews, wk);
  if (n == "work.sessions") return num(w.nSessions, wk);
  if (n == "work.ci")
    return str(w.ci == world::Ci::GREEN ? "green" : w.ci == world::Ci::RED ? "red" : "none", w.valid);
  if (n == "work.merged") return num(w.merged, w.valid);
  if (n == "work.mergedToday") return num(w.mergedToday, w.valid);
  if (n == "buddy.total") return num(d.sessions, d.linked);
  if (n == "buddy.running") return num(d.running, d.linked);
  if (n == "buddy.waiting") return num(d.waiting, d.linked);
  if (n == "buddy.tokens") return num(d.tokens, d.linked);
  if (n.startsWith("pet.")) {
    pet::Level lv = pet::levelFor(w.xp);
    if (n == "pet.level") return num(lv.level, w.valid);
    if (n == "pet.xp") return num(w.xp, w.valid);
    if (n == "pet.into") return num(lv.into, w.valid);
    if (n == "pet.need") return num(lv.need, w.valid);
    if (n == "pet.streak") return num(w.streak, w.valid);
    if (n == "pet.greens") return num(w.greens, w.valid);
    if (n == "pet.hat") return flag(w.acc & pet::HAT);
    if (n == "pet.shades") return flag(w.acc & pet::SHADES);
    if (n == "pet.crown") return flag(w.acc & pet::CROWN);
  }
  if (n == "sky.night") return flag(faces::isNightMinute(d.minute));
  if (n == "sky.day") return num(tk ? d.minute * 100 / 1440 : 50, tk);  // 0..99 through the day
  if (n == "motion.tiltX") return num(tiltX_);
  if (n == "motion.tiltY") return num(tiltY_);
  if (n.startsWith("react.")) {
    int ev = eventIndex(name + 6);
    return flag(ev >= 0 && eventAt_[ev] && c.now - eventAt_[ev] < reactMs_[ev]);
  }
  return v;  // unknown: not known (the validator refuses these)
}

float Face::numOf(JsonVariantConst v, float dflt, const Ctx& c) const {
  if (v.is<float>()) return v.as<float>();
  if (!v.is<JsonObjectConst>()) return dflt;
  Val b = get(v["var"] | "", c);
  float x = (b.num ? b.n : b.truthy() ? 1 : 0) * (v["mul"] | 1.0f) + (v["add"] | 0.0f);
  if (v["lo"].is<float>()) x = max(x, v["lo"].as<float>());
  if (v["hi"].is<float>()) x = min(x, v["hi"].as<float>());
  return x;
}

bool Face::cond(JsonVariantConst v, const Ctx& c, int depth) const {
  if (v.isNull()) return true;
  if (depth > 3) return false;
  if (v.is<const char*>()) {
    const char* s = v.as<const char*>();
    const bool neg = s[0] == '!';
    return get(s + (neg ? 1 : 0), c).truthy() != neg;
  }
  if (v["all"].is<JsonArrayConst>()) {
    for (JsonVariantConst x : v["all"].as<JsonArrayConst>())
      if (!cond(x, c, depth + 1)) return false;
    return true;
  }
  if (v["any"].is<JsonArrayConst>()) {
    for (JsonVariantConst x : v["any"].as<JsonArrayConst>())
      if (cond(x, c, depth + 1)) return true;
    return false;
  }
  if (!v["not"].isNull()) return !cond(v["not"], c, depth + 1);
  Val b = get(v["var"] | "", c);
  for (JsonPairConst kv : v.as<JsonObjectConst>()) {
    const char* op = kv.key().c_str();
    JsonVariantConst x = kv.value();
    if (!strcmp(op, "var")) continue;
    if (!b.known) return !strcmp(op, "ne");
    if (x.is<const char*>()) {
      const bool same = b.s == x.as<const char*>();
      return !strcmp(op, "eq") ? same : !strcmp(op, "ne") ? !same : false;
    }
    float r = x.is<bool>() ? (x.as<bool>() ? 1 : 0) : x.as<float>();
    float l = b.num ? b.n : b.truthy() ? 1 : 0;
    if (!strcmp(op, "eq")) return l == r;
    if (!strcmp(op, "ne")) return l != r;
    if (!strcmp(op, "gt")) return l > r;
    if (!strcmp(op, "ge")) return l >= r;
    if (!strcmp(op, "lt")) return l < r;
    if (!strcmp(op, "le")) return l <= r;
  }
  return false;
}

String Face::expand(const char* t, const Ctx& c) const {
  String out;
  for (const char* p = t; *p;) {
    if (*p != '{') { out += *p++; continue; }
    const char* e = strchr(p, '}');
    if (!e) break;
    std::string inner(p + 1, e - p - 1), fmt;
    size_t colon = inner.find(':');
    if (colon != std::string::npos) { fmt = inner.substr(colon + 1); inner = inner.substr(0, colon); }
    Val v = get(inner.c_str(), c);
    String s = v.s;
    if (fmt == "upper" || fmt == "lower") {
      std::string x(s.c_str());
      for (auto& ch : x) ch = fmt == "upper" ? toupper(ch) : tolower(ch);
      s = x;
    } else if (fmt.size() == 2 && fmt[0] == '0' && v.known) {
      while ((int)s.length() < fmt[1] - '0') s = "0" + s;
    }
    out += s;
    p = e + 1;
  }
  return out;
}

bool Face::colorOf(const char* s, uint16_t& out) const {
  if (!s) return false;
  if (s[0] == '#' && strlen(s) == 7) {
    long v = strtol(s + 1, nullptr, 16);
    out = rgb(v >> 16, (v >> 8) & 255, v & 255);
    return true;
  }
  auto it = colors_.find(s);
  if (it == colors_.end()) return false;
  out = it->second;
  return true;
}

uint16_t Face::color(JsonVariantConst v, uint16_t dflt) const {
  uint16_t c;
  return colorOf(v | (const char*)nullptr, c) ? c : dflt;
}

// --- Tweens (FORMAT.md, "Animation") ---------------------------------------------------------

static float ease(const char* e, float t) {
  t = constrain(t, 0.0f, 1.0f);
  if (!strcmp(e, "in")) return t * t;
  if (!strcmp(e, "out")) return 1 - (1 - t) * (1 - t);
  if (!strcmp(e, "inout")) return smooth(t);
  if (!strcmp(e, "sine")) return 0.5f - 0.5f * cosf(t * PI);
  if (!strcmp(e, "back")) return easeOutBack(t);
  return t;
}

Face::Tw Face::tweens(JsonVariantConst anims, const Ctx& c, uint16_t base) const {
  Tw out;
  out.mixTo = c.bg;
  for (JsonVariantConst a : anims.as<JsonArrayConst>()) {
    const char* prop = a["prop"] | "";
    const uint32_t ms = max(50, a["ms"] | 1000);
    const char* loop = a["loop"] | "repeat";
    const char* on = a["on"] | (const char*)nullptr;
    int32_t t = (int32_t)c.now - (int32_t)(a["delay"] | 0) - (int32_t)(a["phase"] | 0) * c.inst;
    float f;
    if (on) {  // runs `times` cycles from the event, else isn't there
      int ev = eventIndex(on);
      if (ev < 0 || !eventAt_[ev]) continue;
      t = (int32_t)(c.now - eventAt_[ev]) - (int32_t)(a["delay"] | 0) - (int32_t)(a["phase"] | 0) * c.inst;
      const int cycles = a["times"] | 1;
      const uint32_t len = ms * cycles * (!strcmp(loop, "pingpong") ? 2 : 1);
      if (t < 0 || (uint32_t)t >= len) continue;
    }
    if (t < 0) t = 0;
    if (!strcmp(loop, "once")) f = min(1.0f, (float)t / ms);
    else if (!strcmp(loop, "pingpong")) {
      uint32_t k = t % (2 * ms);
      f = k < ms ? (float)k / ms : 2.0f - (float)k / ms;
    } else f = (float)(t % ms) / ms;
    f = ease(a["ease"] | "linear", f);
    if (!strcmp(prop, "color")) {
      uint16_t from = color(a["from"], base), to = color(a["to"], base);
      out.color = blend(from, to, f);
      out.hasColor = true;
      continue;
    }
    const float from = a["from"] | 0.0f, to = a["to"] | 0.0f, v = from + (to - from) * f;
    if (!strcmp(prop, "x")) out.dx += v;
    else if (!strcmp(prop, "y")) out.dy += v;
    else if (!strcmp(prop, "mix")) { out.mix = constrain(v, 0.0f, 1.0f); out.mixTo = color(a["mixTo"], c.bg); }
    else if (!strcmp(prop, "frame")) out.frame = (int)lroundf(v);
  }
  return out;
}

// --- Drawing --------------------------------------------------------------------------------

static const lgfx::IFont* font(int px) {
  switch (px) {
    case 10: return F9;
    case 14: return F14;
    case 18: return F18;
    case 24: return F24;
    case 32: return F32;
    case 44: return F44;
    case 56: return F56;
    case 96: return F96;
    default: return F12;
  }
}

static textdatum_t datum(const char* a) {
  static const struct { const char* n; textdatum_t d; } T[] = {
      {"tl", top_left}, {"tc", top_center}, {"tr", top_right}, {"ml", middle_left}, {"mc", middle_center},
      {"mr", middle_right}, {"bl", bottom_left}, {"bc", bottom_center}, {"br", bottom_right}};
  for (auto& t : T)
    if (!strcmp(t.n, a)) return t.d;
  return top_left;
}

// The top-left of a w x h box whose anchor point is x, y.
static void anchorBox(const char* a, int& x, int& y, int w, int h) {
  if (a[1] == 'c') x -= w / 2;
  else if (a[1] == 'r') x -= w;
  if (a[0] == 'm') y -= h / 2;
  else if (a[0] == 'b') y -= h;
}

void Face::drawImage(const Image& im, int frame, int x, int y, int s, bool flip, float mix, uint16_t mixTo,
                     bool tile) {
  frame = constrain(frame, 0, im.frames - 1);
  uint16_t pal[256];
  for (size_t i = 0; i < im.pal.size() && i < 256; i++) pal[i] = mix > 0 ? blend(im.pal[i], mixTo, mix) : im.pal[i];
  const int fw = im.fw, x0 = frame * fw, tw = fw * s;
  int from = x, copies = 1;
  if (tile) {  // repeat across the screen (parallax layers)
    from = ((x % tw) + tw) % tw - tw;
    copies = (W - from + tw - 1) / tw;
  }
  // (the firmware engine would push palette rows with pushImage; one pixel at a time is fine here)
  for (int k = 0; k < copies; k++) {
    const int bx = from + k * tw;
    for (int yy = 0; yy < im.h; yy++) {
      const int py = y + yy * s;
      if (py + s <= 0 || py >= H) continue;
      for (int xx = 0; xx < fw; xx++) {
        const int sx = flip ? fw - 1 - xx : xx;
        const uint8_t v = im.px[yy * im.w + x0 + sx];
        if (v == im.clear) continue;
        const int px = bx + xx * s;
        if (px + s <= 0 || px >= W) continue;
        if (s == 1) canvas.drawPixel(px, py, pal[v]);
        else canvas.fillRect(px, py, s, s, pal[v]);
      }
    }
  }
}

void Face::drawOne(JsonVariantConst e, const Ctx& c, int x, int y, const Tw& t) {
  const char* type = e["type"] | "";
  const char* anchor = e["anchor"] | "tl";
  const float mix = t.mix >= 0 ? t.mix : (e["mix"] | 0.0f);
  const uint16_t mixTo = t.mix >= 0 ? t.mixTo : color(e["mixTo"], c.bg);
  auto col = [&](const char* key, uint16_t dflt, bool main = false) {
    uint16_t v = color(e[key], dflt);
    if (main && t.hasColor) v = t.color;
    return mix > 0 ? blend(v, mixTo, mix) : v;
  };
  const bool hasFill = !e["fill"].isNull(), hasStroke = !e["stroke"].isNull();

  if (!strcmp(type, "rect")) {
    int w = e["w"] | 1, h = e["h"] | 1, r = e["r"] | 0;
    anchorBox(anchor, x, y, w, h);
    if (hasFill || t.hasColor) {
      if (r) canvas.fillSmoothRoundRect(x, y, w, h, r, col("fill", INK, true));
      else canvas.fillRect(x, y, w, h, col("fill", INK, true));
    }
    if (hasStroke) {
      if (r) canvas.drawRoundRect(x, y, w, h, r, col("stroke", INK));
      else canvas.drawRect(x, y, w, h, col("stroke", INK));
    }
  } else if (!strcmp(type, "line")) {
    const int x2 = (e["x2"] | 0) + c.ox + (int)t.dx, y2 = (e["y2"] | 0) + c.oy + (int)t.dy;
    canvas.drawWideLine(x, y, x2, y2, (e["width"] | 1.0f) / 2, col("color", INK, true));
  } else if (!strcmp(type, "circle")) {
    const int r = e["r"] | 1;
    if (hasFill || t.hasColor) canvas.fillSmoothCircle(x, y, r, col("fill", INK, true));
    if (hasStroke) canvas.drawCircle(x, y, r, col("stroke", INK));
  } else if (!strcmp(type, "arc")) {
    const int r = e["r"] | 10, th = e["t"] | 2;
    const float a0 = e["start"] | 0, a1 = e["end"] | 360;
    float frac = 1;
    if (!e["value"].isNull()) {
      JsonVariantConst v = e["value"];
      if (v.is<float>()) frac = v.as<float>();
      else {
        Val b = get(v["var"] | "", c);
        frac = b.known ? b.n / max(1.0f, v["of"] | 1.0f) : 0;
      }
      frac = constrain(frac, 0.0f, 1.0f);
    }
    if (!e["track"].isNull()) canvas.fillArc(x, y, r - th + 1, r, a0 - 90, a1 - 90, col("track", LINE));
    if (frac > 0) canvas.fillArc(x, y, r - th + 1, r, a0 - 90, a0 - 90 + (a1 - a0) * frac, col("color", INK, true));
  } else if (!strcmp(type, "bar")) {
    int w = e["w"] | 10, h = e["h"] | 2, r = e["r"] | 0;
    anchorBox(anchor, x, y, w, h);
    JsonVariantConst v = e["value"];
    float frac = 0;
    if (v.is<float>()) frac = v.as<float>();
    else {
      Val b = get(v["var"] | "", c);
      frac = b.known ? b.n / max(1.0f, v["of"] | 1.0f) : 0;
    }
    frac = constrain(frac, 0.0f, 1.0f);
    if (!e["track"].isNull()) canvas.fillRoundRect(x, y, w, h, r, col("track", LINE));
    const bool up = !strcmp(e["dir"] | "right", "up");
    if (up) {
      const int fh = lroundf(h * frac);
      if (fh > 0) canvas.fillRoundRect(x, y + h - fh, w, fh, min(r, fh / 2), col("color", INK, true));
    } else {
      const int fw = lroundf(w * frac);
      if (fw > 0) canvas.fillRoundRect(x, y, fw, h, min(r, fw / 2), col("color", INK, true));
    }
  } else if (!strcmp(type, "text")) {
    String s = expand(e["text"] | "", c);
    const char* cs = e["case"] | "";
    if (*cs) {
      std::string x2(s.c_str());
      for (auto& ch : x2) ch = !strcmp(cs, "upper") ? toupper(ch) : tolower(ch);
      s = x2;
    }
    s.trim();
    if (!s.length()) return;
    const int w = e["w"] | 0;
    const lgfx::IFont* f = font(e["font"] | 12);
    if (e["fit"].is<JsonArrayConst>()) {
      for (JsonVariantConst fv : e["fit"].as<JsonArrayConst>()) {
        f = font(fv | 12);
        canvas.setFont(f);
        if (canvas.textWidth(s) <= w) break;
      }
    }
    const uint16_t ink = col("color", INK, true);
    const bool outline = !e["outline"].isNull(), shadow = !e["shadow"].isNull();
    const uint16_t shade = outline ? col("outline", PUPIL) : col("shadow", PUPIL);
    const int lines = e["lines"] | 1;
    String ls[3];
    int n = 1;
    if (lines > 1 && w) n = wrap(s, f, ls, lines, w);
    else ls[0] = w ? ellipsize(s, f, w) : s;
    const int lh = lineH(f);
    textdatum_t dt = datum(anchor);
    // a block of n lines: the anchor applies to the block
    int by = y;
    if (n > 1) {
      if (anchor[0] == 'm') by = y - (n - 1) * lh / 2;
      else if (anchor[0] == 'b') by = y - (n - 1) * lh;
    }
    for (int i = 0; i < n; i++) {
      const int ly = by + i * lh;
      if (outline) faces::outlineTxt(ls[i], x, ly, f, ink, shade, dt, f == F56 || f == F96 ? 2 : 1);
      else if (shadow) faces::shadowTxt(ls[i], x, ly, f, ink, shade, dt);
      else txt(ls[i], x, ly, f, ink, dt);
    }
  } else if (!strcmp(type, "image")) {
    auto it = imageIndex_.find(e["image"] | "");
    if (it == imageIndex_.end()) return;
    const Image& im = images_[it->second];
    const int s = constrain(e["scale"] | 1, 1, 4);
    int frame = (int)numOf(e["frame"], 0, c);
    if (e["play"].is<JsonObjectConst>()) {
      const float fps = constrain(e["play"]["fps"] | 4.0f, 0.2f, 12.0f);
      const uint32_t k = (uint32_t)(c.now * fps / 1000) + c.inst;
      JsonArrayConst seq = e["play"]["seq"];
      frame = seq.size() ? (int)(seq[k % seq.size()] | 0) : (int)(k % im.frames);
    }
    if (t.frame >= 0) frame = t.frame;
    int w = im.fw * s, h = im.h * s;
    anchorBox(anchor, x, y, w, h);
    drawImage(im, frame, x, y, s, !strcmp(e["flip"] | "", "x"), mix, mixTo, !strcmp(e["tile"] | "", "x"));
  } else if (!strcmp(type, "hand")) {
    const Dash& d = *c.d;
    clockface::Hands hs = clockface::hands(d.minute >= 0 ? d.minute : 10 * 60 + 10, d.second >= 0 ? d.second : 0);
    const char* which = e["hand"] | "minute";
    if (!strcmp(which, "second") && (d.minute < 0 || d.second < 0)) return;
    const float deg = !strcmp(which, "hour") ? hs.hour : !strcmp(which, "second") ? hs.second : hs.minute;
    const float a = deg * PI / 180, sn = sinf(a), cs2 = cosf(a);
    const float len = e["length"] | 30, tail = e["tail"] | 0, w0 = e["w0"] | 2.0f, w1 = e["w1"] | 1.0f;
    if (!e["shadow"].isNull())
      canvas.drawWedgeLine(x + 1 - sn * tail, y + 2 + cs2 * tail, x + 1 + sn * len, y + 2 - cs2 * len, w0, w1,
                           col("shadow", PUPIL));
    canvas.drawWedgeLine(x - sn * tail, y + cs2 * tail, x + sn * len, y - cs2 * len, w0, w1, col("color", INK, true));
  } else if (!strcmp(type, "ticks")) {
    const int r = e["r"] | 50, n = e["count"] | 12, every = e["every"] | 0;
    for (int i = 0; i < n; i++) {
      const bool major = every && i % every == 0;
      const float a = i * 2 * PI / n, sn = sinf(a), cs2 = -cosf(a);
      const int len = major ? (e["majorLen"] | (e["len"] | 3)) : (e["len"] | 3);
      const float wd = major ? (e["majorWidth"] | (e["width"] | 1.0f)) : (e["width"] | 1.0f);
      uint16_t cc = major && !e["majorColor"].isNull() ? col("majorColor", INK) : col("color", INK, true);
      if (i == 0 && !e["firstColor"].isNull()) cc = col("firstColor", INK);
      if (len <= 1 && wd <= 1) canvas.fillRect(x + sn * (r - 1), y + cs2 * (r - 1), 1, 1, cc);
      else canvas.drawWideLine(x + sn * r, y + cs2 * r, x + sn * (r - len), y + cs2 * (r - len), wd / 2, cc);
    }
  } else if (!strcmp(type, "weather")) {
    if (!c.d->wxValid) return;
    const int s = e["size"] | 20;
    anchorBox(anchor, x, y, s, s);
    weatherIcon(c.d->wxIcon, c.d->wxDay, x, y, s);
  } else if (!strcmp(type, "scatter")) {
    const int w = max(1, e["w"] | 1), h = max(1, e["h"] | 1), n = e["n"] | 1, s = e["size"] | 1;
    const uint32_t seed = (e["seed"] | 1) * 7919u;
    const int tw = e["twinkle"] | 0, fall = e["fall"] | 0, drift = e["drift"] | 0;
    const uint16_t base = col("color", INK, true);
    for (int i = 0; i < n; i++) {
      uint32_t hx = faces::hash32(seed + i * 2), hy = faces::hash32(seed + i * 2 + 1);
      int px = (int)((hx % w + (int64_t)drift * c.now / 1000) % w), py = (int)((hy % h + (int64_t)fall * c.now / 1000) % h);
      if (px < 0) px += w;
      if (py < 0) py += h;
      uint16_t cc = base;
      if (tw) {
        const float ph = sinf(2 * PI * ((float)(c.now % (uint32_t)tw) / tw + faces::frac01(hx ^ 0x9e37)));
        cc = blend(base, c.bg, 0.4f + 0.4f * ph);
      }
      canvas.fillRect(x + px, y + py, s, s, cc);
    }
  }
}

void Face::element(JsonVariantConst e, Ctx c, int depth) {
  if (!cond(e["when"], c)) return;
  JsonVariantConst rep = e["repeat"];
  int count = 1, dx = 0, dy = 0;
  if (rep.is<JsonObjectConst>()) {
    JsonVariantConst cv = rep["count"];
    count = cv.is<int>() ? cv.as<int>() : (int)get(cv | "", c).n;
    count = constrain(count, 0, min(16, (int)(rep["max"] | 16)));
    dx = rep["dx"] | 0;
    dy = rep["dy"] | 0;
  }
  const char* type = e["type"] | "";
  for (int i = 0; i < count; i++) {
    Ctx ci = c;
    ci.inst = i;
    const Tw t = tweens(e["anim"], ci, INK);
    const int x = (int)lroundf(numOf(e["x"], 0, ci) + t.dx) + c.ox + i * dx;
    const int y = (int)lroundf(numOf(e["y"], 0, ci) + t.dy) + c.oy + i * dy;
    if (!strcmp(type, "group")) {
      if (depth > 0) continue;
      ci.ox = x;
      ci.oy = y;
      JsonVariantConst kids = e["children"];
      if (e["part"].is<const char*>()) kids = doc_["parts"][e["part"].as<const char*>()];
      for (JsonVariantConst k : kids.as<JsonArrayConst>()) element(k, ci, depth + 1);
      continue;
    }
    drawOne(e, ci, x, y, t);
  }
}

void Face::draw(const Dash& d, const world::Snapshot& w, bool wide, uint32_t now) {
  const faces::Urgent u = faces::urgent(d);
  events(d, u.on, now);
  JsonVariantConst lay = doc_["layouts"][wide ? "landscape" : "portrait"];
  Ctx c{&d, &w, now, wide, GROUND};
  JsonVariantConst bg = lay["bg"];
  if (bg["grad"].is<JsonArrayConst>()) {
    const uint16_t a = color(bg["grad"][0], GROUND), b = color(bg["grad"][1], GROUND);
    const bool hz = !strcmp(bg["dir"] | "v", "h");
    const int n = hz ? W : H;
    for (int i = 0; i < n; i += 2) {
      const uint16_t cc = blend(a, b, (float)i / (n - 1));
      if (hz) canvas.fillRect(i, 0, 2, H, cc);
      else canvas.fillRect(0, i, W, 2, cc);
    }
    c.bg = a;
  } else {
    c.bg = color(bg, GROUND);
    canvas.fillScreen(c.bg);
  }
  for (JsonVariantConst e : lay["elements"].as<JsonArrayConst>()) element(e, c, 0);

  // What the firmware always draws over a face (FORMAT.md, "What the stick always shows")
  const char* status = lay["status"] | "light";
  if (!strcmp(status, "pill")) canvas.fillRoundRect(W - 76, 1, 75, 17, 8, rgb(0x11, 0x13, 0x16));
  faces::statusIcons(d, W - 4, 9, !strcmp(status, "dark") ? rgb(0x1a, 0x1d, 0x21) : INK);
  if (u.on) {
    JsonArrayConst card = lay["card"];
    int cx = wide ? 10 : 6, cy = wide ? 84 : 184, cw = wide ? 220 : 123, ch = wide ? 44 : 48;
    if (card.size() == 4) { cx = card[0]; cy = card[1]; cw = card[2]; ch = card[3]; }
    faces::signBoard(cx, cy, cw, ch, u.a, u.b, u.bg, u.ink);
  }
}

}  // namespace pkgface
