#include "facefmt.h"
#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <string.h>
#include "clockface.h"
#include "pet.h"

namespace facefmt {

const char* const EVENT_NAMES[EV_COUNT] = {
    "merged", "ci_red", "ci_green", "claude_start", "claude_done", "release", "level_up", "review", "shake",
    "flick", "upside_down", "lift", "minute", "hour", "dusk", "dawn", "urgent"};

int eventIndex(const char* name) {
  for (int i = 0; i < EV_COUNT; i++)
    if (!strcmp(EVENT_NAMES[i], name)) return i;
  return -1;
}

int eventForKick(world::Kick k) {
  switch (k) {
    case world::Kick::MERGED:       return EV_MERGED;
    case world::Kick::CI_RED:       return EV_CI_RED;
    case world::Kick::CI_GREEN:     return EV_CI_GREEN;
    case world::Kick::CLAUDE_START: return EV_CLAUDE_START;
    case world::Kick::CLAUDE_DONE:  return EV_CLAUDE_DONE;
    case world::Kick::RELEASE:      return EV_RELEASE;
    case world::Kick::LEVEL_UP:     return EV_LEVEL_UP;
    case world::Kick::REVIEW:       return EV_REVIEW;
    case world::Kick::SHAKE:        return EV_SHAKE;
    case world::Kick::FLICK:        return EV_FLICK;
    case world::Kick::UPSIDE_DOWN:  return EV_UPSIDE_DOWN;
    case world::Kick::LIFT:         return EV_LIFT;
    default:                        return -1;
  }
}

// --- Bindings ----------------------------------------------------------------------------------

static const char* const NAMES[REACT] = {
    "time.known", "time.hm", "time.hm24", "time.h12", "time.hh12", "time.h24", "time.hh24", "time.mm", "time.ss",
    "time.ampm", "time.hour", "time.minute", "time.second", "time.minuteOfDay", "time.words", "time.wordsLead",
    "time.wordsHour",
    "date.weekday", "date.dow", "date.day", "date.dd", "date.month", "date.mon", "date.mm", "date.short", "date.long",
    "wx.valid", "wx.temp", "wx.hi", "wx.lo", "wx.icon", "wx.cond", "wx.day",
    "dev.battery", "dev.charging", "dev.bt", "dev.name", "agent.linked",
    "card.kind", "card.a", "card.b", "card.urgent", "urgent.on",
    "work.known", "work.asks", "work.reviews", "work.sessions", "work.ci", "work.merged", "work.mergedToday",
    "buddy.total", "buddy.running", "buddy.waiting", "buddy.tokens",
    "pet.level", "pet.xp", "pet.into", "pet.need", "pet.streak", "pet.greens", "pet.hat", "pet.shades", "pet.crown",
    "sky.night", "sky.day", "motion.tiltX", "motion.tiltY"};

int bindingIndex(const char* name) {
  if (!name) return -1;
  for (int i = 0; i < REACT; i++)
    if (!strcmp(NAMES[i], name)) return i;
  if (!strncmp(name, "react.", 6)) {
    int ev = eventIndex(name + 6);
    if (ev >= 0) return REACT + ev;
  }
  return -1;
}

const char* bindingName(int id) {
  static char buf[24];
  if (id >= 0 && id < REACT) return NAMES[id];
  if (id >= REACT && id < BIND_COUNT) {
    snprintf(buf, sizeof buf, "react.%s", EVENT_NAMES[id - REACT]);
    return buf;
  }
  return "?";
}

bool secondsBinding(int id) { return id == TIME_SS || id == TIME_SECOND; }

static const char* const DAYS[] = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"};
static const char* const MONTHS[] = {"January", "February", "March",     "April",   "May",      "June",
                                     "July",    "August",   "September", "October", "November", "December"};

static void str(Value& v, const char* s, bool known = true) {
  v.known = known;
  v.num = false;
  v.n = 0;
  snprintf(v.s, sizeof v.s, "%s", known && s ? s : "");
}

static void strf(Value& v, bool known, const char* fmt, int a, int b = 0) {
  v.known = known;
  v.num = false;
  v.n = 0;
  if (known) snprintf(v.s, sizeof v.s, fmt, a, b);
  else v.s[0] = 0;
}

static void num(Value& v, float n, bool known = true) {
  v.known = known;
  v.num = true;
  v.n = known ? n : 0;
  if (known) snprintf(v.s, sizeof v.s, "%ld", lroundf(n));
  else v.s[0] = 0;
}

static void abbrev(Value& v, const char* s) {  // "Fri", "Nov"; nullptr = unknown
  str(v, "", s != nullptr);
  if (s) snprintf(v.s, sizeof v.s, "%.3s", s);
}

static void flag(Value& v, bool b) { num(v, b ? 1 : 0); }

static bool nightMinute(int m) { return m >= 0 && (m >= 22 * 60 || m < 6 * 60); }

void value(int id, const Inputs& in, Value& v) {
  const bool tk = in.minute >= 0, sk = tk && in.second >= 0;
  const int hour = tk ? in.minute / 60 : 0, minute = tk ? in.minute % 60 : 0, sec = in.second >= 0 ? in.second : 0;
  const int h12 = hour % 12 ? hour % 12 : 12;
  const bool dk = tk && in.wday >= 0 && in.wday < 7 && in.mon >= 0 && in.mon < 12;
  static const world::Snapshot none;
  const world::Snapshot& w = in.world ? *in.world : none;
  const bool wk = in.agentLinked && w.valid;
  switch (id) {
    case TIME_KNOWN:  flag(v, tk); return;
    case TIME_HM:
      if (in.clock24) strf(v, tk, "%d:%02d", hour, minute);
      else strf(v, tk, "%d:%02d", h12, minute);
      return;
    case TIME_HM24:   strf(v, tk, "%d:%02d", hour, minute); return;
    case TIME_H12:    strf(v, tk, "%d", h12); return;
    case TIME_HH12:   strf(v, tk, "%02d", h12); return;
    case TIME_H24:    strf(v, tk, "%d", hour); return;
    case TIME_HH24:   strf(v, tk, "%02d", hour); return;
    case TIME_MM:     strf(v, tk, "%02d", minute); return;
    case TIME_SS:     strf(v, sk, "%02d", sec); return;
    case TIME_AMPM:   str(v, hour < 12 ? "AM" : "PM", tk); return;
    case TIME_HOUR:   num(v, hour, tk); return;
    case TIME_MINUTE: num(v, minute, tk); return;
    case TIME_SECOND: num(v, sec, sk); return;
    case TIME_MINUTE_OF_DAY: num(v, in.minute, tk); return;
    case TIME_WORDS: case TIME_WORDS_LEAD: case TIME_WORDS_HOUR: {
      if (!tk) { str(v, "", false); return; }
      clockface::Words cw = clockface::words(hour, minute);
      str(v, id == TIME_WORDS ? cw.text.c_str() : id == TIME_WORDS_LEAD ? cw.lead.c_str() : cw.hour.c_str());
      return;
    }
    case DATE_WEEKDAY: str(v, dk ? DAYS[in.wday] : "", dk); return;
    case DATE_DOW:     abbrev(v, dk ? DAYS[in.wday] : nullptr); return;
    case DATE_DAY:     strf(v, dk, "%d", in.mday); return;
    case DATE_DD:      strf(v, dk, "%02d", in.mday); return;
    case DATE_MONTH:   str(v, dk ? MONTHS[in.mon] : "", dk); return;
    case DATE_MON:     abbrev(v, dk ? MONTHS[in.mon] : nullptr); return;
    case DATE_MM:      strf(v, dk, "%02d", in.mon + 1); return;
    case DATE_SHORT:   str(v, in.dateShort, in.dateShort && *in.dateShort); return;
    case DATE_LONG:    str(v, in.dateLong, in.dateLong && *in.dateLong); return;
    case WX_VALID:     flag(v, in.wxValid); return;
    case WX_TEMP:      num(v, in.wxTemp, in.wxValid); return;
    case WX_HI:        num(v, in.wxHi, in.wxValid); return;
    case WX_LO:        num(v, in.wxLo, in.wxValid); return;
    case WX_ICON:      str(v, in.wxIcon, in.wxValid); return;
    case WX_COND:      str(v, in.wxCond, in.wxValid); return;
    case WX_DAY:       flag(v, in.wxValid ? in.wxDay : !nightMinute(in.minute)); return;
    case DEV_BATTERY:  num(v, in.battery, in.battery >= 0); return;
    case DEV_CHARGING: flag(v, in.charging); return;
    case DEV_BT:       str(v, in.bt == 0 ? "connected" : in.bt == 1 ? "waiting" : "off"); return;
    case DEV_NAME:     str(v, in.name); return;
    case AGENT_LINKED: flag(v, in.agentLinked); return;
    case CARD_KIND:    str(v, in.cardKind, in.agentLinked); return;
    case CARD_A:       str(v, in.cardA, in.agentLinked); return;
    case CARD_B:       str(v, in.cardB, in.agentLinked); return;
    case CARD_URGENT:  flag(v, in.agentLinked && in.cardUrgent); return;
    case URGENT_ON:    flag(v, in.urgentOn); return;
    case WORK_KNOWN:   flag(v, wk); return;
    case WORK_ASKS:    num(v, w.asks, wk); return;
    case WORK_REVIEWS: num(v, w.reviews, wk); return;
    case WORK_SESSIONS: num(v, w.nSessions, wk); return;
    case WORK_CI:
      str(v, w.ci == world::Ci::GREEN ? "green" : w.ci == world::Ci::RED ? "red" : "none", w.valid);
      return;
    case WORK_MERGED:  num(v, w.merged, w.valid); return;
    case WORK_MERGED_TODAY: num(v, w.mergedToday, w.valid); return;
    case BUDDY_TOTAL:   num(v, in.buddyTotal, in.buddyLinked); return;
    case BUDDY_RUNNING: num(v, in.buddyRunning, in.buddyLinked); return;
    case BUDDY_WAITING: num(v, in.buddyWaiting, in.buddyLinked); return;
    case BUDDY_TOKENS:  num(v, (float)in.buddyTokens, in.buddyLinked); return;
    case PET_LEVEL: case PET_INTO: case PET_NEED: {
      pet::Level lv = pet::levelFor(w.xp);
      num(v, id == PET_LEVEL ? lv.level : id == PET_INTO ? lv.into : lv.need, w.valid);
      return;
    }
    case PET_XP:     num(v, (float)w.xp, w.valid); return;
    case PET_STREAK: num(v, w.streak, w.valid); return;
    case PET_GREENS: num(v, w.greens, w.valid); return;
    case PET_HAT:    flag(v, w.acc & pet::HAT); return;
    case PET_SHADES: flag(v, w.acc & pet::SHADES); return;
    case PET_CROWN:  flag(v, w.acc & pet::CROWN); return;
    case SKY_NIGHT:  flag(v, nightMinute(in.minute)); return;
    case SKY_DAY:    num(v, tk ? in.minute * 100 / 1440 : 50, tk); return;  // 0..99 through the day
    case MOTION_TILT_X: num(v, in.tiltX); return;
    case MOTION_TILT_Y: num(v, in.tiltY); return;
  }
  if (id >= REACT && id < BIND_COUNT) {
    const int ev = id - REACT;
    const uint32_t at = in.eventAt ? in.eventAt[ev] : 0, len = in.reactMs ? in.reactMs[ev] : 3000;
    flag(v, at && in.now - at < len);
    return;
  }
  v = Value{};  // unknown: not known (the loader refuses these)
}

// --- Templates -----------------------------------------------------------------------------------

constexpr char MARK = 0x01;

bool compileTemplate(const char* src, char* out, size_t cap, const char** err) {
  const char* why = nullptr;
  size_t o = 0;
  for (const char* p = src; *p && !why;) {
    if (*p == MARK) { why = "control character"; break; }
    if (*p != '{') {
      if (o + 1 >= cap) { why = "too long"; break; }
      out[o++] = *p++;
      continue;
    }
    const char* e = strchr(p, '}');
    if (!e || e - p - 1 >= 40) { why = "unclosed {"; break; }
    char inner[40];
    memcpy(inner, p + 1, e - p - 1);
    inner[e - p - 1] = 0;
    uint8_t fmt = FMT_NONE;
    if (char* c = strchr(inner, ':')) {
      *c = 0;
      const char* f = c + 1;
      if (!strcmp(f, "02")) fmt = FMT_PAD2;
      else if (!strcmp(f, "03")) fmt = FMT_PAD3;
      else if (!strcmp(f, "04")) fmt = FMT_PAD4;
      else if (!strcmp(f, "upper")) fmt = FMT_UPPER;
      else if (!strcmp(f, "lower")) fmt = FMT_LOWER;
      else { why = "unknown format"; break; }
    }
    const int id = bindingIndex(inner);
    if (id < 0) { why = "unknown binding"; break; }
    if (o + 3 >= cap) { why = "too long"; break; }
    out[o++] = MARK;
    out[o++] = (char)(id + 1);
    out[o++] = (char)(fmt + 1);
    p = e + 1;
  }
  if (cap) out[o < cap ? o : cap - 1] = 0;
  if (err) *err = why;
  return !why;
}

void applyCase(char* s, bool upper) {
  for (; *s; s++)
    if (upper && *s >= 'a' && *s <= 'z') *s -= 32;
    else if (!upper && *s >= 'A' && *s <= 'Z') *s += 32;
}

size_t expand(const char* t, const Inputs& in, char* out, size_t cap) {
  if (!cap) return 0;
  size_t o = 0;
  Value v;
  for (const char* p = t; *p;) {
    if (*p != MARK) {
      if (o + 1 < cap) out[o++] = *p;
      p++;
      continue;
    }
    if (!p[1] || !p[2]) break;
    const int id = (uint8_t)p[1] - 1, fmt = (uint8_t)p[2] - 1;
    p += 3;
    value(id, in, v);
    if (fmt == FMT_UPPER || fmt == FMT_LOWER) applyCase(v.s, fmt == FMT_UPPER);
    if (fmt >= FMT_PAD2 && fmt <= FMT_PAD4 && v.known)
      for (size_t pad = fmt - FMT_PAD2 + 2, n = strlen(v.s); n < pad && o + 1 < cap; n++) out[o++] = '0';
    for (const char* s = v.s; *s && o + 1 < cap; s++) out[o++] = *s;
  }
  out[o] = 0;
  return o;
}

bool templateUses(const char* t, bool (*pred)(int id)) {
  for (const char* p = t; *p; p++)
    if (*p == MARK && p[1] && p[2]) {
      if (pred((uint8_t)p[1] - 1)) return true;
      p += 2;
    }
  return false;
}

// --- Numbers ---------------------------------------------------------------------------------------

float numValue(const Num& n, const Inputs& in) {
  if (n.bind < 0) return n.k;
  Value v;
  value(n.bind, in, v);
  float x = v.number() * n.mul + n.add;
  if (n.hasLo && x < n.lo) x = n.lo;
  if (n.hasHi && x > n.hi) x = n.hi;
  return x;
}

float fracValue(const Frac& f, const Inputs& in) {
  float x = f.k;
  if (f.bind >= 0) {
    Value v;
    value(f.bind, in, v);
    x = v.known ? v.n / (f.of < 1 ? 1 : f.of) : 0;
  }
  return x < 0 ? 0 : x > 1 ? 1 : x;
}

// --- Conditions ------------------------------------------------------------------------------------

bool eval(const Cond* nodes, int at, const Inputs& in, int depth) {
  if (depth > MAX_COND_DEPTH) return false;
  const Cond& c = nodes[at];
  switch (c.op) {
    case Op::ALL:
      for (int i = 0; i < c.n; i++)
        if (!eval(nodes, c.first + i, in, depth + 1)) return false;
      return true;
    case Op::ANY:
      for (int i = 0; i < c.n; i++)
        if (eval(nodes, c.first + i, in, depth + 1)) return true;
      return false;
    case Op::NOT:
      return !eval(nodes, c.first, in, depth + 1);
    default:
      break;
  }
  Value v;
  value(c.bind, in, v);
  if (c.op == Op::TRUTHY) return v.truthy();
  if (c.op == Op::FALSY) return !v.truthy();
  if (!v.known) return c.op == Op::NE;  // unknown: equals nothing
  if (c.isStr) {
    const bool same = !strcmp(v.s, c.str);
    return c.op == Op::EQ ? same : c.op == Op::NE ? !same : false;
  }
  const float l = v.number(), r = c.num;
  switch (c.op) {
    case Op::EQ: return l == r;
    case Op::NE: return l != r;
    case Op::GT: return l > r;
    case Op::GE: return l >= r;
    case Op::LT: return l < r;
    case Op::LE: return l <= r;
    default:     return false;
  }
}

// --- Tweens --------------------------------------------------------------------------------------

float ease(Ease e, float t) {
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  switch (e) {
    case Ease::IN:    return t * t;
    case Ease::OUT:   return 1 - (1 - t) * (1 - t);
    case Ease::INOUT: return t * t * (3 - 2 * t);  // smoothstep, as ui::smooth
    case Ease::SINE:  return 0.5f - 0.5f * cosf(t * 3.14159265358979f);
    case Ease::BACK: {  // overshoot, as ui::easeOutBack
      const float c1 = 1.70158f, c3 = c1 + 1;
      return 1 + c3 * powf(t - 1, 3) + c1 * powf(t - 1, 2);
    }
    default:          return t;
  }
}

bool progress(const Tween& tw, uint32_t now, uint32_t eventAt, int inst, float& f) {
  const uint32_t ms = tw.ms < 50 ? 50 : tw.ms;
  int32_t t = (int32_t)now - (int32_t)tw.delay - (int32_t)(tw.phase * inst);
  if (tw.on >= 0) {  // plays `times` cycles from the event, else isn't there
    if (!eventAt) return false;
    t = (int32_t)(now - eventAt) - (int32_t)tw.delay - (int32_t)(tw.phase * inst);
    const uint32_t len = ms * tw.times * (tw.loop == Loop::PINGPONG ? 2 : 1);
    if (t < 0 || (uint32_t)t >= len) return false;
  }
  if (t < 0) t = 0;
  if (tw.loop == Loop::ONCE) {
    f = (float)t / ms;
    if (f > 1) f = 1;
  } else if (tw.loop == Loop::PINGPONG) {
    const uint32_t k = (uint32_t)t % (2 * ms);
    f = k < ms ? (float)k / ms : 2.0f - (float)k / ms;
  } else {
    f = (float)((uint32_t)t % ms) / ms;
  }
  f = ease(tw.ease, f);
  return true;
}

// --- Colours -----------------------------------------------------------------------------------------

bool parseHex(const char* s, uint16_t& out) {
  if (!s || s[0] != '#' || strlen(s) != 7) return false;
  for (int i = 1; i < 7; i++)
    if (!isxdigit((unsigned char)s[i])) return false;
  const long v = strtol(s + 1, nullptr, 16);
  out = rgb565(v >> 16, (v >> 8) & 255, v & 255);
  return true;
}

uint16_t blend(uint16_t a, uint16_t b, float t) {
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  int ar = a >> 11, ag = (a >> 5) & 63, ab = a & 31, br = b >> 11, bg = (b >> 5) & 63, bb = b & 31;
  return ((int)(ar + (br - ar) * t) << 11) | ((int)(ag + (bg - ag) * t) << 5) | (int)(ab + (bb - ab) * t);
}

// --- Frame rate ----------------------------------------------------------------------------------------

uint32_t frameMs(int tier, bool seconds, bool onBattery, uint32_t idleMs) {
  if (tier >= LIVELY) return !onBattery ? 40 : idleMs < 20000 ? 66 : 150;
  if (tier == CALM) return !onBattery ? 100 : idleMs < 20000 ? 200 : 500;
  if (tier == STATIC && seconds) return onBattery ? 1000 : 200;
  return 0;
}

bool Guard::feed(uint32_t frameUs, uint32_t now) {
  if (frameUs <= SLOW_US) {
    slow = false;
    return false;
  }
  if (!slow) {
    slow = true;
    slowSince = now;
    return false;
  }
  if (now - slowSince < SLOW_FOR_MS) return false;
  slow = false;  // lowered: the next tier gets its own 2 s
  return true;
}

}  // namespace facefmt
