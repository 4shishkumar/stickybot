#include "engine.h"
#include <ArduinoJson.h>
#include <new>
#include <vector>
#include "../common.h"
#include "../../config.h"
#include "../../logic/clockface.h"
#include "../../logic/ota_rx.h"
#include "../../logic/sfb.h"
#if defined(ESP_PLATFORM)
#include <esp_heap_caps.h>
#endif

namespace faces {
namespace pkg {

using namespace ui;
using facefmt::Cond;
using facefmt::Op;
using facefmt::Tween;

// --- Memory: PSRAM on the stick (8 MB) when there is some ----------------------------------

void* bigAlloc(size_t n) {
#if defined(ESP_PLATFORM)
  void* p = heap_caps_malloc(n, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
  return p ? p : malloc(n);
#else
  return malloc(n);
#endif
}

namespace {
struct BigAllocator : ArduinoJson::Allocator {
  void* allocate(size_t n) override { return bigAlloc(n); }
  void deallocate(void* p) override { free(p); }
  void* reallocate(void* p, size_t n) override {
#if defined(ESP_PLATFORM)
    void* q = heap_caps_realloc(p, n, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    return q ? q : realloc(p, n);
#else
    return realloc(p, n);
#endif
  }
};
BigAllocator bigJson;
}  // namespace

constexpr size_t BLOCK = 8 * 1024;

void* Arena::alloc(size_t n) {
  n = (n + 7) & ~(size_t)7;  // 8: pointers and floats in the structs, on any host
  if (!head_ || head_->used + n > head_->cap) {
    const size_t cap = n > BLOCK / 2 ? n : BLOCK;  // a big one (an image) gets its own block
    Block* b = (Block*)bigAlloc(sizeof(Block) + cap);
    if (!b) return nullptr;
    b->used = 0;
    b->cap = cap;
    if (n > BLOCK / 2 && head_) {  // keep filling the current small block
      b->next = head_->next;
      head_->next = b;
      b->used = n;
      bytes_ += sizeof(Block) + cap;
      void* p = (uint8_t*)(b + 1);
      memset(p, 0, n);
      return p;
    }
    b->next = head_;
    head_ = b;
    bytes_ += sizeof(Block) + cap;
  }
  void* p = (uint8_t*)(head_ + 1) + head_->used;
  head_->used += n;
  memset(p, 0, n);
  return p;
}

char* Arena::copy(const char* s, size_t n) {
  char* p = (char*)alloc(n + 1);
  if (p) {
    memcpy(p, s, n);
    p[n] = 0;
  }
  return p;
}

void Arena::clear() {
  while (head_) {
    Block* n = head_->next;
    free(head_);
    head_ = n;
  }
  bytes_ = 0;
}

// --- What a manifest compiles to ---------------------------------------------------------------

enum class Type : uint8_t { RECT, LINE, CIRCLE, ARC, BAR, TEXT, IMAGE, HAND, TICKS, WEATHER, SCATTER, GROUP };

enum : uint8_t { HAS_C0 = 1, HAS_C1 = 2, HAS_C2 = 4, HAS_MIX_TO = 8, OUTLINE = 16, SHADOW = 32 };

struct Elem {
  Type     type = Type::RECT;
  uint8_t  anchor = 0;            // tl tc tr ml mc mr bl bc br
  uint8_t  has = 0;               // HAS_* bits
  uint8_t  nTweens = 0, nKids = 0;
  int16_t  when = -1;             // the condition's first node, -1 always
  facefmt::Num x, y;
  Tween*   tweens = nullptr;
  // repeat: count copies (or the binding's value, at most max), each offset by dx, dy
  int8_t   repBind = -1;
  uint8_t  repCount = 1, repMax = 16;
  int16_t  repDx = 0, repDy = 0;
  // colours: c0 the main one (fill / color: what a colour tween changes), c1 the second
  // (stroke / track / outline / shadow / majorColor), c2 firstColor
  uint16_t c0 = 0, c1 = 0, c2 = 0, mixTo = 0;
  float    mix = 0;
  // shapes
  int16_t  w = 0, h = 0, r = 0, t = 0, x2 = 0, y2 = 0, a0 = 0, a1 = 360;
  float    width = 1, w0 = 2, w1 = 1, majorWidth = 1;
  bool     up = false;            // bar: fills upwards
  bool     hasValue = false, hasTrack = false;
  facefmt::Frac value;
  // text
  const char* text = nullptr;     // compiled (facefmt::compileTemplate)
  uint8_t  font = 12, fit[4] = {0}, nFit = 0, lines = 1;
  int8_t   textCase = 0;          // 1 upper, -1 lower
  // image
  int8_t   image = -1;
  facefmt::Num frame;
  uint16_t fpsMilli = 0;          // play: frames per 1000 s (0: no play)
  uint8_t* seq = nullptr;
  uint8_t  nSeq = 0, scale = 1;
  bool     tile = false, flip = false;
  // hand / ticks / scatter
  uint8_t  hand = 1;              // 0 hour, 1 minute, 2 second
  int16_t  length = 30, tail = 0;
  int16_t  count = 12, every = 0, len = 3, majorLen = 3;
  int16_t  n = 1, seed = 1, size = 1, twinkle = 0, fall = 0, drift = 0;
  Elem*    kids = nullptr;
};

// --- Loading ------------------------------------------------------------------------------------

namespace {

constexpr int MAX_ELEMS = 48, MAX_DRAWN = 96, MAX_KIDS = 16, MAX_PARTS = 8, MAX_TWEENS = 4, MAX_COLORS = 16,
              MAX_CONDS = 256, MAX_TEXT = 64, MAX_TERMS = 8;

const char* const ANCHORS[] = {"tl", "tc", "tr", "ml", "mc", "mr", "bl", "bc", "br"};
const int FONTS[] = {10, 12, 14, 18, 24, 32, 44, 56, 96};

struct TypeDef {
  const char* name;
  Type type;
  const char* const* fields;    // besides the common ones
  const char* const* required;
};

const char* const COMMON[] = {"type", "name", "x", "y", "when", "anim", "repeat", "mix", "mixTo", nullptr};
const char* const F_RECT[] = {"anchor", "w", "h", "r", "fill", "stroke", nullptr};
const char* const R_RECT[] = {"w", "h", nullptr};
const char* const F_LINE[] = {"x2", "y2", "width", "color", nullptr};
const char* const R_LINE[] = {"x2", "y2", "color", nullptr};
const char* const F_CIRCLE[] = {"r", "fill", "stroke", nullptr};
const char* const R_CIRCLE[] = {"r", nullptr};
const char* const F_ARC[] = {"r", "t", "start", "end", "value", "color", "track", nullptr};
const char* const R_ARC[] = {"r", "t", "color", nullptr};
const char* const F_BAR[] = {"anchor", "w", "h", "r", "dir", "value", "color", "track", nullptr};
const char* const R_BAR[] = {"w", "h", "value", "color", nullptr};
const char* const F_TEXT[] = {"anchor", "text", "font", "fit", "w", "lines", "case", "color", "outline", "shadow", nullptr};
const char* const R_TEXT[] = {"text", "color", nullptr};
const char* const F_IMAGE[] = {"anchor", "image", "frame", "play", "scale", "tile", "flip", nullptr};
const char* const R_IMAGE[] = {"image", nullptr};
const char* const F_HAND[] = {"hand", "length", "tail", "w0", "w1", "color", "shadow", nullptr};
const char* const R_HAND[] = {"hand", "length", "color", nullptr};
const char* const F_TICKS[] = {"r", "count", "len", "width", "color", "every", "majorLen", "majorWidth", "majorColor",
                               "firstColor", nullptr};
const char* const R_TICKS[] = {"r", "count", "color", nullptr};
const char* const F_WEATHER[] = {"anchor", "size", nullptr};
const char* const F_SCATTER[] = {"w", "h", "n", "seed", "size", "color", "twinkle", "fall", "drift", nullptr};
const char* const R_SCATTER[] = {"w", "h", "n", "color", nullptr};
const char* const F_GROUP[] = {"part", "children", nullptr};
const char* const NONE[] = {nullptr};

const TypeDef TYPES[] = {
    {"rect", Type::RECT, F_RECT, R_RECT},       {"line", Type::LINE, F_LINE, R_LINE},
    {"circle", Type::CIRCLE, F_CIRCLE, R_CIRCLE}, {"arc", Type::ARC, F_ARC, R_ARC},
    {"bar", Type::BAR, F_BAR, R_BAR},           {"text", Type::TEXT, F_TEXT, R_TEXT},
    {"image", Type::IMAGE, F_IMAGE, R_IMAGE},   {"hand", Type::HAND, F_HAND, R_HAND},
    {"ticks", Type::TICKS, F_TICKS, R_TICKS},   {"weather", Type::WEATHER, F_WEATHER, NONE},
    {"scatter", Type::SCATTER, F_SCATTER, R_SCATTER}, {"group", Type::GROUP, F_GROUP, NONE},
};

const char* const TOP[] = {"$schema", "format", "id", "name", "version", "author", "license", "about", "tags",
                           "minFirmware", "motion", "colors", "images", "parts", "reactions", "layouts", nullptr};
const char* const LAYOUT[] = {"bg", "status", "card", "elements", nullptr};
const char* const TWEEN[] = {"prop", "from", "to", "ms", "ease", "loop", "delay", "phase", "on", "times", "mixTo",
                             nullptr};
const char* const REPEAT[] = {"count", "max", "dx", "dy", nullptr};
const char* const NUM[] = {"var", "mul", "add", "lo", "hi", nullptr};
const char* const FRAC[] = {"var", "of", nullptr};
const char* const PLAY[] = {"fps", "seq", nullptr};
const char* const LAYOUTS[] = {"portrait", "landscape", nullptr};
const char* const GRAD[] = {"grad", "dir", nullptr};
const char* const COND_OPS[] = {"eq", "ne", "gt", "ge", "lt", "le"};

bool in(const char* const* list, const char* s) {
  for (; *list; list++)
    if (!strcmp(*list, s)) return true;
  return false;
}

bool nameOk(const char* s, size_t maxLen) {  // [a-z][a-z0-9_]*
  size_t n = strlen(s);
  if (!n || n > maxLen || s[0] < 'a' || s[0] > 'z') return false;
  for (size_t i = 0; i < n; i++)
    if (!((s[i] >= 'a' && s[i] <= 'z') || (s[i] >= '0' && s[i] <= '9') || s[i] == '_')) return false;
  return true;
}

bool timeBinding(int id) {
  return id >= facefmt::TIME_HM && id <= facefmt::TIME_WORDS_HOUR && id != facefmt::TIME_AMPM;
}

struct Builder {
  Arena& a;
  String& err;
  String where;
  const Image* images;
  int nImages;
  struct Named { const char* name; uint16_t c; } colors[MAX_COLORS];
  int nColors = 0;
  struct ImageName { const char* name; int i; } imageNames[sfb::MAX_IMAGES];
  int nImageNames = 0;
  struct Part { const char* name; Elem* e; uint8_t n; int elems, drawn; bool time, seconds; } parts[MAX_PARTS];
  int nParts = 0;
  std::vector<Cond> conds;
  // the layout being built
  int  elems = 0, drawn = 0;
  bool time = false, seconds = false;
  uint32_t* busyMs;    // per event: how long its reactions and tweens run

  Builder(Arena& ar, String& e, const Image* im, int ni, uint32_t* busy)
      : a(ar), err(e), images(im), nImages(ni), busyMs(busy) {}

  bool fail(const String& why) {
    if (!err.length()) err = where.length() ? where + ": " + why : why;
    return false;
  }

  bool only(JsonObjectConst o, const char* const* allowed, const char* const* more = nullptr) {
    for (JsonPairConst kv : o) {
      const char* k = kv.key().c_str();
      if (!in(allowed, k) && !(more && in(more, k))) return fail(String("unknown field '") + k + "'");
    }
    return true;
  }

  // A number in lo..hi (integer: whole numbers only); absent keeps out.
  bool number(JsonVariantConst v, const char* what, float lo, float hi, float& out, bool integer = false) {
    if (v.isNull()) return true;
    if (!v.is<float>() || v.is<bool>()) return fail(String(what) + ": not a number");
    if (integer && !v.is<long>()) return fail(String(what) + ": not a whole number");
    const float x = v.as<float>();
    if (!(x >= lo && x <= hi)) return fail(String(what) + " out of range");
    out = x;
    return true;
  }
  bool integer(JsonVariantConst v, const char* what, int lo, int hi, int16_t& out) {
    float f = out;
    if (!number(v, what, lo, hi, f, true)) return false;
    out = (int16_t)f;
    return true;
  }

  bool binding(JsonVariantConst v, const char* what, int8_t& out) {
    const int id = facefmt::bindingIndex(v | "");
    if (!v.is<const char*>() || id < 0) return fail(String(what) + ": unknown binding '" + (v | "") + "'");
    if (timeBinding(id)) time = true;
    if (facefmt::secondsBinding(id)) seconds = true;
    out = (int8_t)id;
    return true;
  }

  bool color(JsonVariantConst v, const char* what, uint16_t& out) {
    const char* s = v | "";
    if (!v.is<const char*>()) return fail(String(what) + ": not a colour");
    if (facefmt::parseHex(s, out)) return true;
    for (int i = 0; i < nColors; i++)
      if (!strcmp(colors[i].name, s)) {
        out = colors[i].c;
        return true;
      }
    return fail(String(what) + ": unknown colour '" + s + "'");
  }
  // An optional colour: sets the bit in has when present
  bool colorIf(JsonObjectConst o, const char* key, uint16_t& out, uint8_t& has, uint8_t bit) {
    if (o[key].isNull()) return true;
    has |= bit;
    return color(o[key], key, out);
  }

  bool pos(JsonVariantConst v, const char* what, facefmt::Num& out) {
    if (v.isNull()) return true;
    if (v.is<JsonObjectConst>()) {
      JsonObjectConst o = v;
      if (!only(o, NUM) || !binding(o["var"], what, out.bind)) return false;
      if (!number(o["mul"], "mul", -1000, 1000, out.mul) || !number(o["add"], "add", -1000, 1000, out.add)) return false;
      out.hasLo = !o["lo"].isNull();
      out.hasHi = !o["hi"].isNull();
      return number(o["lo"], "lo", -1e6f, 1e6f, out.lo) && number(o["hi"], "hi", -1e6f, 1e6f, out.hi);
    }
    return number(v, what, -240, 480, out.k, true);
  }

  bool frac(JsonVariantConst v, facefmt::Frac& out) {
    if (v.is<JsonObjectConst>()) {
      JsonObjectConst o = v;
      if (o["of"].isNull()) return fail("value: needs 'of'");
      return only(o, FRAC) && binding(o["var"], "value", out.bind) && number(o["of"], "of", 1, 100000, out.of);
    }
    if (v.isNull()) return fail("value: missing");
    return number(v, "value", 0, 1, out.k);
  }

  // A condition into conds[slot] (its children appended after)
  bool cond(JsonVariantConst v, int slot, int depth) {
    if (depth > facefmt::MAX_COND_DEPTH) return fail("condition nested too deep");
    if ((int)conds.size() > MAX_CONDS) return fail("too many conditions");
    if (v.is<const char*>()) {  // "sky.night" or "!sky.night"
      const char* s = v.as<const char*>();
      const bool neg = s[0] == '!';
      const int id = facefmt::bindingIndex(s + (neg ? 1 : 0));
      if (id < 0) return fail(String("when: unknown binding '") + s + "'");
      if (timeBinding(id)) time = true;
      conds[slot].op = neg ? Op::FALSY : Op::TRUTHY;
      conds[slot].bind = (uint8_t)id;
      return true;
    }
    if (!v.is<JsonObjectConst>()) return fail("when: not a condition");
    JsonObjectConst o = v;
    for (const char* group : {"all", "any"}) {
      if (o[group].isNull()) continue;
      JsonArrayConst list = o[group];
      if (o.size() != 1 || list.isNull() || !list.size() || list.size() > MAX_TERMS)
        return fail(String("when: '") + group + "' needs 1-8 conditions");
      const int first = conds.size();
      conds.resize(first + list.size());
      conds[slot].op = !strcmp(group, "all") ? Op::ALL : Op::ANY;
      conds[slot].n = list.size();
      conds[slot].first = first;
      int i = 0;
      for (JsonVariantConst x : list)
        if (!cond(x, first + i++, depth + 1)) return false;
      return true;
    }
    if (!o["not"].isNull()) {
      if (o.size() != 1) return fail("when: 'not' stands alone");
      const int first = conds.size();
      conds.resize(first + 1);
      conds[slot].op = Op::NOT;
      conds[slot].n = 1;
      conds[slot].first = first;
      return cond(o["not"], first, depth + 1);
    }
    if (o.size() != 2) return fail("when: a comparison is {\"var\", one operator}");
    int8_t id;
    if (!binding(o["var"], "when", id)) return false;
    Cond& c = conds[slot];
    c.bind = (uint8_t)id;
    for (JsonPairConst kv : o) {
      const char* k = kv.key().c_str();
      if (!strcmp(k, "var")) continue;
      int op = -1;
      for (int i = 0; i < 6; i++)
        if (!strcmp(COND_OPS[i], k)) op = i;
      if (op < 0) return fail(String("when: unknown operator '") + k + "'");
      c.op = (Op)((int)Op::EQ + op);
      JsonVariantConst x = kv.value();
      if (x.is<const char*>()) {
        if (op > 1) return fail(String("when: '") + k + "' needs a number");
        c.isStr = true;
        c.str = a.copy(x.as<const char*>(), strlen(x.as<const char*>()));
        if (!c.str) return fail("out of memory");
      } else if (x.is<bool>()) {
        if (op > 1) return fail(String("when: '") + k + "' needs a number");
        c.num = x.as<bool>() ? 1 : 0;
      } else if (x.is<float>()) {
        c.num = x.as<float>();
      } else {
        return fail("when: bad value");
      }
    }
    return true;
  }

  bool when(JsonVariantConst v, int16_t& out) {
    if (v.isNull()) return true;
    const int slot = conds.size();
    conds.resize(slot + 1);
    out = slot;
    return cond(v, slot, 0);
  }

  bool tween(JsonObjectConst o, Tween& t, bool imageElem) {
    if (!only(o, TWEEN)) return false;
    static const char* const PROPS[] = {"x", "y", "mix", "color", "frame"};
    const char* prop = o["prop"] | "";
    int p = -1;
    for (int i = 0; i < 5; i++)
      if (!strcmp(PROPS[i], prop)) p = i;
    if (p < 0) return fail(String("anim: unknown prop '") + prop + "'");
    t.prop = (facefmt::Prop)p;
    if (o["ms"].isNull() || o["to"].isNull()) return fail("anim: needs 'ms' and 'to'");
    float ms = 1000, delay = 0, phase = 0, times = 1;
    if (!number(o["ms"], "ms", 50, 120000, ms, true) || !number(o["delay"], "delay", 0, 60000, delay, true) ||
        !number(o["phase"], "phase", 0, 60000, phase, true) || !number(o["times"], "times", 1, 20, times, true))
      return false;
    t.ms = ms;
    t.delay = delay;
    t.phase = phase;
    t.times = times;
    if ((t.prop == facefmt::Prop::COLOR || t.prop == facefmt::Prop::MIX) && ms < 500)
      return fail("anim: colour changes take at least 500 ms (no flashing)");
    if (t.prop == facefmt::Prop::FRAME && !imageElem) return fail("anim: 'frame' is for images");
    static const char* const EASES[] = {"linear", "in", "out", "inout", "sine", "back"};
    static const char* const LOOPS[] = {"repeat", "pingpong", "once"};
    const char* ease = o["ease"] | "linear";
    const char* loop = o["loop"] | "repeat";
    int e = -1, l = -1;
    for (int i = 0; i < 6; i++)
      if (!strcmp(EASES[i], ease)) e = i;
    for (int i = 0; i < 3; i++)
      if (!strcmp(LOOPS[i], loop)) l = i;
    if (e < 0 || l < 0) return fail("anim: unknown ease or loop");
    t.ease = (facefmt::Ease)e;
    t.loop = (facefmt::Loop)l;
    if (!o["on"].isNull()) {
      const int ev = facefmt::eventIndex(o["on"] | "");
      if (ev < 0) return fail(String("anim: unknown event '") + (o["on"] | "") + "'");
      t.on = ev;
      const uint32_t len = t.delay + t.phase * 15 + t.ms * t.times * (t.loop == facefmt::Loop::PINGPONG ? 2 : 1);
      if (len > busyMs[ev]) busyMs[ev] = len;
    }
    if (t.prop == facefmt::Prop::COLOR) {
      if (!o["from"].isNull()) {
        t.hasFrom = true;
        if (!color(o["from"], "from", t.cFrom)) return false;
      }
      t.hasTo = true;
      if (!color(o["to"], "to", t.cTo)) return false;
    } else {
      if (!number(o["from"], "from", -1000, 1000, t.from) || !number(o["to"], "to", -1000, 1000, t.to)) return false;
    }
    if (!o["mixTo"].isNull()) {
      t.hasMixTo = true;
      if (!color(o["mixTo"], "mixTo", t.mixTo)) return false;
    }
    return true;
  }

  bool font(JsonVariantConst v, uint8_t& out) {
    float f = 0;
    if (!number(v, "font", 10, 96, f, true)) return false;
    for (int x : FONTS)
      if (x == (int)f) {
        out = (uint8_t)f;
        return true;
      }
    return fail("font: not one of the stick's sizes");
  }

  int imageIndex(const char* name) {
    for (int i = 0; i < nImageNames; i++)
      if (!strcmp(imageNames[i].name, name)) return imageNames[i].i;
    return -1;
  }

  // One element into e; depth 1 inside a group or part. drawn: its copies, times mult.
  bool elem(JsonVariantConst v, Elem& e, int depth, int mult) {
    if (!v.is<JsonObjectConst>()) return fail("not an element");
    JsonObjectConst o = v;
    const char* type = o["type"] | "";
    const TypeDef* td = nullptr;
    for (const TypeDef& t : TYPES)
      if (!strcmp(t.name, type)) td = &t;
    if (!td) return fail(String("unknown type '") + type + "'");
    e.type = td->type;
    if (!only(o, COMMON, td->fields)) return false;
    for (const char* const* r = td->required; *r; r++)
      if (o[*r].isNull()) return fail(String(type) + " needs '" + *r + "'");
    if (o["name"].is<const char*>() == false && !o["name"].isNull()) return fail("name: not text");
    if (!pos(o["x"], "x", e.x) || !pos(o["y"], "y", e.y) || !when(o["when"], e.when)) return false;
    if (!o["anchor"].isNull()) {
      const char* an = o["anchor"] | "";
      int i = -1;
      for (int k = 0; k < 9; k++)
        if (!strcmp(ANCHORS[k], an)) i = k;
      if (i < 0) return fail(String("unknown anchor '") + an + "'");
      e.anchor = i;
    }
    // repeat
    int copies = 1;
    if (!o["repeat"].isNull()) {
      JsonObjectConst rp = o["repeat"];
      if (rp.isNull() || !only(rp, REPEAT) || rp["count"].isNull()) return fail("repeat: needs 'count'");
      int16_t mx = 16, cnt = 1;
      if (!integer(rp["max"], "max", 1, 16, mx) || !integer(rp["dx"], "dx", -240, 240, e.repDx) ||
          !integer(rp["dy"], "dy", -240, 240, e.repDy))
        return false;
      e.repMax = mx;
      if (rp["count"].is<const char*>()) {
        if (rp["max"].isNull()) return fail("repeat: a bound count needs 'max'");
        if (!binding(rp["count"], "repeat", e.repBind)) return false;
        copies = mx;
      } else {
        if (!integer(rp["count"], "count", 0, 16, cnt)) return false;
        e.repCount = cnt;
        copies = cnt < mx ? cnt : mx;
      }
    }
    // mix
    if (!number(o["mix"], "mix", 0, 1, e.mix) || !colorIf(o, "mixTo", e.mixTo, e.has, HAS_MIX_TO)) return false;
    // tweens
    if (!o["anim"].isNull()) {
      JsonArrayConst an = o["anim"];
      if (an.isNull() || an.size() > MAX_TWEENS) return fail("anim: at most 4 tweens");
      e.nTweens = an.size();
      e.tweens = a.make<Tween>(e.nTweens);
      if (!e.tweens) return fail("out of memory");
      int i = 0;
      for (JsonVariantConst t : an) {
        if (!t.is<JsonObjectConst>()) return fail("anim: not a tween");
        if (!tween(t, e.tweens[i++], e.type == Type::IMAGE)) return false;
      }
    }
    elems++;
    drawn += copies * mult;

    switch (e.type) {
      case Type::RECT:
        return integer(o["w"], "w", 1, 240, e.w) && integer(o["h"], "h", 1, 240, e.h) &&
               integer(o["r"], "r", 0, 60, e.r) && colorIf(o, "fill", e.c0, e.has, HAS_C0) &&
               colorIf(o, "stroke", e.c1, e.has, HAS_C1);
      case Type::LINE:
        return integer(o["x2"], "x2", -240, 480, e.x2) && integer(o["y2"], "y2", -240, 480, e.y2) &&
               number(o["width"], "width", 0.5f, 8, e.width) && colorIf(o, "color", e.c0, e.has, HAS_C0);
      case Type::CIRCLE:
        return integer(o["r"], "r", 1, 120, e.r) && colorIf(o, "fill", e.c0, e.has, HAS_C0) &&
               colorIf(o, "stroke", e.c1, e.has, HAS_C1);
      case Type::ARC:
        e.r = 10;
        e.t = 2;
        e.hasValue = !o["value"].isNull();
        e.hasTrack = !o["track"].isNull();
        return integer(o["r"], "r", 2, 120, e.r) && integer(o["t"], "t", 1, 60, e.t) &&
               integer(o["start"], "start", -360, 360, e.a0) && integer(o["end"], "end", -360, 720, e.a1) &&
               (!e.hasValue || frac(o["value"], e.value)) && colorIf(o, "color", e.c0, e.has, HAS_C0) &&
               colorIf(o, "track", e.c1, e.has, HAS_C1);
      case Type::BAR: {
        e.w = 10;
        e.h = 2;
        e.hasTrack = !o["track"].isNull();
        const char* dir = o["dir"] | "right";
        if (strcmp(dir, "right") && strcmp(dir, "up")) return fail("bar: dir is right or up");
        e.up = !strcmp(dir, "up");
        return integer(o["w"], "w", 2, 240, e.w) && integer(o["h"], "h", 1, 240, e.h) &&
               integer(o["r"], "r", 0, 30, e.r) && frac(o["value"], e.value) &&
               colorIf(o, "color", e.c0, e.has, HAS_C0) && colorIf(o, "track", e.c1, e.has, HAS_C1);
      }
      case Type::TEXT: {
        const char* src = o["text"] | "";
        const size_t n = strlen(src);
        if (!o["text"].is<const char*>() || !n || n > MAX_TEXT) return fail("text: 1-64 characters");
        char* t = (char*)a.alloc(n + 1);
        const char* why = nullptr;
        if (!t) return fail("out of memory");
        if (!facefmt::compileTemplate(src, t, n + 1, &why)) return fail(String("text: ") + why);
        if (facefmt::templateUses(t, timeBinding)) time = true;
        if (facefmt::templateUses(t, facefmt::secondsBinding)) seconds = true;
        e.text = t;
        int16_t lines = 1;
        if (!integer(o["w"], "w", 8, 240, e.w) || !integer(o["lines"], "lines", 1, 3, lines)) return false;
        e.lines = lines;
        if (!o["font"].isNull() && !font(o["font"], e.font)) return false;
        if (!o["fit"].isNull()) {
          JsonArrayConst fit = o["fit"];
          if (fit.isNull() || !fit.size() || fit.size() > 4) return fail("fit: 1-4 fonts");
          if (!e.w) return fail("fit needs 'w'");
          for (JsonVariantConst f : fit)
            if (!font(f, e.fit[e.nFit++])) return false;
        }
        if (!o["case"].isNull()) {
          const char* cs = o["case"] | "";
          if (strcmp(cs, "upper") && strcmp(cs, "lower")) return fail("case: upper or lower");
          e.textCase = !strcmp(cs, "upper") ? 1 : -1;
        }
        if (!o["outline"].isNull()) {
          e.has |= OUTLINE;
          if (!colorIf(o, "outline", e.c1, e.has, HAS_C1)) return false;
        } else if (!o["shadow"].isNull()) {
          e.has |= SHADOW;
          if (!colorIf(o, "shadow", e.c1, e.has, HAS_C1)) return false;
        }
        return colorIf(o, "color", e.c0, e.has, HAS_C0);
      }
      case Type::IMAGE: {
        const int i = imageIndex(o["image"] | "");
        if (i < 0) return fail(String("unknown image '") + (o["image"] | "") + "'");
        e.image = i;
        const Image& im = images[i];
        int16_t scale = 1;
        if (!integer(o["scale"], "scale", 1, 4, scale)) return false;
        e.scale = scale;
        if (o["frame"].is<JsonObjectConst>()) {
          if (!pos(o["frame"], "frame", e.frame)) return false;
        } else if (!number(o["frame"], "frame", 0, im.frames - 1, e.frame.k, true)) {
          return false;
        }
        for (const char* k : {"tile", "flip"})
          if (!o[k].isNull() && strcmp(o[k] | "", "x")) return fail(String(k) + ": only \"x\"");
        e.tile = !o["tile"].isNull();
        e.flip = !o["flip"].isNull();
        if (!o["play"].isNull()) {
          JsonObjectConst pl = o["play"];
          float fps = 4;
          if (pl.isNull() || !only(pl, PLAY) || pl["fps"].isNull() || !number(pl["fps"], "fps", 0.2f, 12, fps))
            return fail("play: needs 'fps' (0.2-12)");
          e.fpsMilli = (uint16_t)lroundf(fps * 1000);
          if (!pl["seq"].isNull()) {
            JsonArrayConst seq = pl["seq"];
            if (seq.isNull() || !seq.size() || seq.size() > 32) return fail("play: seq has 1-32 frames");
            e.nSeq = seq.size();
            e.seq = a.make<uint8_t>(e.nSeq);
            if (!e.seq) return fail("out of memory");
            int k = 0;
            for (JsonVariantConst f : seq) {
              float x = 0;
              if (!number(f, "seq", 0, im.frames - 1, x, true)) return false;
              e.seq[k++] = (uint8_t)x;
            }
          }
        }
        return true;
      }
      case Type::HAND: {
        static const char* const HANDS[] = {"hour", "minute", "second"};
        const char* hd = o["hand"] | "";
        int i = -1;
        for (int k = 0; k < 3; k++)
          if (!strcmp(HANDS[k], hd)) i = k;
        if (i < 0) return fail("hand: hour, minute or second");
        e.hand = i;
        time = true;
        if (i == 2) seconds = true;
        return integer(o["length"], "length", 2, 120, e.length) && integer(o["tail"], "tail", 0, 60, e.tail) &&
               number(o["w0"], "w0", 0.5f, 8, e.w0) && number(o["w1"], "w1", 0.5f, 8, e.w1) &&
               colorIf(o, "color", e.c0, e.has, HAS_C0) && colorIf(o, "shadow", e.c1, e.has, HAS_C1);
      }
      case Type::TICKS:
        e.r = 50;
        if (!integer(o["r"], "r", 4, 120, e.r) || !integer(o["count"], "count", 2, 120, e.count) ||
            !integer(o["len"], "len", 1, 40, e.len) || !number(o["width"], "width", 0.5f, 6, e.width) ||
            !integer(o["every"], "every", 1, 60, e.every))
          return false;
        e.majorLen = e.len;
        e.majorWidth = e.width;
        return integer(o["majorLen"], "majorLen", 1, 40, e.majorLen) &&
               number(o["majorWidth"], "majorWidth", 0.5f, 6, e.majorWidth) &&
               colorIf(o, "color", e.c0, e.has, HAS_C0) && colorIf(o, "majorColor", e.c1, e.has, HAS_C1) &&
               colorIf(o, "firstColor", e.c2, e.has, HAS_C2);
      case Type::WEATHER:
        e.size = 20;
        return integer(o["size"], "size", 12, 48, e.size);
      case Type::SCATTER:
        return integer(o["w"], "w", 1, 240, e.w) && integer(o["h"], "h", 1, 240, e.h) &&
               integer(o["n"], "n", 1, 48, e.n) && integer(o["seed"], "seed", 0, 32767, e.seed) &&
               integer(o["size"], "size", 1, 3, e.size) && integer(o["twinkle"], "twinkle", 0, 20000, e.twinkle) &&
               integer(o["fall"], "fall", -120, 120, e.fall) && integer(o["drift"], "drift", -120, 120, e.drift) &&
               colorIf(o, "color", e.c0, e.has, HAS_C0);
      case Type::GROUP: {
        if (depth > 0) return fail("groups don't nest");
        const bool part = !o["part"].isNull(), kids = !o["children"].isNull();
        if (part == kids) return fail("group: 'part' or 'children'");
        if (part) {
          const char* name = o["part"] | "";
          for (int i = 0; i < nParts; i++)
            if (!strcmp(parts[i].name, name)) {
              e.kids = parts[i].e;
              e.nKids = parts[i].n;
              elems += parts[i].elems;
              drawn += parts[i].drawn * copies * mult;
              if (parts[i].time) time = true;
              if (parts[i].seconds) seconds = true;
              return true;
            }
          return fail(String("unknown part '") + name + "'");
        }
        JsonArrayConst list = o["children"];
        if (list.isNull() || list.size() > MAX_KIDS) return fail("group: at most 16 children");
        return children(list, e.kids, e.nKids, copies * mult);
      }
    }
    return true;
  }

  bool children(JsonArrayConst list, Elem*& out, uint8_t& n, int mult) {
    n = list.size();
    out = a.make<Elem>(n);
    if (!out) return fail("out of memory");
    const String outer = where;
    int i = 0;
    for (JsonVariantConst k : list) {
      where = outer + ", child " + String(i);
      if (!elem(k, out[i++], 1, mult)) return false;
    }
    where = outer;
    return true;
  }
};

const lgfx::IFont* fontFor(int px) {
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

const textdatum_t DATUMS[] = {top_left,    top_center,    top_right,    middle_left, middle_center,
                              middle_right, bottom_left, bottom_center, bottom_right};

// The top-left of a w x h box whose anchor point is x, y.
void anchorBox(uint8_t a, int& x, int& y, int w, int h) {
  const int col = a % 3, row = a / 3;
  if (col == 1) x -= w / 2;
  else if (col == 2) x -= w;
  if (row == 1) y -= h / 2;
  else if (row == 2) y -= h;
}

void copyField(char* out, size_t cap, const char* s) {
  snprintf(out, cap, "%s", s ? s : "");
}

}  // namespace

// --- peek / load -------------------------------------------------------------------------------

bool peek(const uint8_t* p, size_t n, Info& info, String& err) {
  info = Info{};
  sfb::File f;
  sfb::Error e = sfb::parse(p, n, f);
  if (e != sfb::Error::NONE) {
    err = sfb::errorText(e);
    if (e == sfb::Error::FORMAT) info.format = f.format;
    return false;
  }
  JsonDocument filter;
  for (const char* k : {"id", "name", "about", "version", "minFirmware", "format", "motion"}) filter[k] = true;
  JsonDocument doc(&bigJson);
  DeserializationError je = deserializeJson(doc, f.manifest, f.manifestLen, DeserializationOption::Filter(filter),
                                            DeserializationOption::NestingLimit(12));
  if (je) {
    err = String("manifest: ") + je.c_str();
    return false;
  }
  copyField(info.id, sizeof info.id, doc["id"] | "");
  copyField(info.name, sizeof info.name, doc["name"] | "");
  copyField(info.about, sizeof info.about, doc["about"] | "");
  copyField(info.version, sizeof info.version, doc["version"] | "");
  copyField(info.minFirmware, sizeof info.minFirmware, doc["minFirmware"] | "");
  info.format = doc["format"] | 0;
  const char* motion = doc["motion"] | "static";
  info.motion = !strcmp(motion, "lively") ? facefmt::LIVELY : !strcmp(motion, "calm") ? facefmt::CALM : facefmt::STATIC;
  return true;
}

void Engine::unload() {
  arena_.clear();
  loaded_ = false;
  images_ = nullptr;
  conds_ = nullptr;
  row_ = nullptr;
  nImages_ = 0;
  for (Layout& l : layouts_) l = Layout{};
  info_ = Info{};
}

void Engine::reset() {
  memset(eventAt_, 0, sizeof eventAt_);
  lastMinute_ = lastHour_ = -2;
  lastNight_ = lastUrgent_ = -1;
}

LoadError Engine::load(const uint8_t* p, size_t n, String& err) {
  unload();
  reset();
  err = "";
  sfb::File f;
  const sfb::Error se = sfb::parse(p, n, f);
  if (se != sfb::Error::NONE) {
    err = sfb::errorText(se);
    return se == sfb::Error::FORMAT ? LoadError::FORMAT : LoadError::INVALID;
  }
  // The images: indices one byte each, frame by frame
  images_ = arena_.make<Image>(f.nImages);
  nImages_ = f.nImages;
  int widest = 1;
  for (int i = 0; i < f.nImages; i++) {
    const sfb::Image& s = f.images[i];
    Image& im = images_[i];
    im.w = s.w;
    im.h = s.h;
    im.fw = s.fw;
    im.frames = s.frames;
    im.colors = s.colors;
    im.clear = s.clear;
    im.pal = arena_.make<uint16_t>(s.colors);
    im.px = arena_.make<uint8_t>((size_t)s.w * s.h);
    if (!im.pal || !im.px) {
      err = "out of memory";
      unload();
      return LoadError::MEMORY;
    }
    for (int c = 0; c < s.colors; c++) im.pal[c] = s.paletteAt(c);
    const char* why = nullptr;
    if (!sfb::unpack(s, im.px, &why)) {
      err = String("image ") + i + ": " + why;
      unload();
      return LoadError::INVALID;
    }
    if (s.fw > widest) widest = s.fw;
  }
  row_ = arena_.make<uint8_t>((size_t)widest * 4 * 4);  // a row at scale 4, four times (pushImage of s rows)

  // The manifest, parsed once
  JsonDocument doc(&bigJson);
  DeserializationError je =
      deserializeJson(doc, f.manifest, f.manifestLen, DeserializationOption::NestingLimit(12));
  if (je || !doc.is<JsonObjectConst>()) {
    err = String("manifest: ") + (je ? je.c_str() : "not an object");
    unload();
    return LoadError::INVALID;
  }
  JsonObjectConst m = doc.as<JsonObjectConst>();
  memset(busyMs_, 0, sizeof busyMs_);
  Builder b(arena_, err, images_, nImages_, busyMs_);
  auto bad = [&](LoadError e = LoadError::INVALID) {
    unload();
    if (!err.length()) err = "invalid";
    return e;
  };
  if (!b.only(m, TOP)) return bad();
  if ((m["format"] | 0) != sfb::FORMAT) {
    err = "unknown face format";
    return bad(LoadError::FORMAT);
  }
  const char* id = m["id"] | "";
  const size_t idLen = strlen(id);
  bool idOk = idLen >= 3 && idLen <= 12 && id[0] >= 'a' && id[0] <= 'z';
  for (size_t i = 0; i < idLen; i++) idOk = idOk && ((id[i] >= 'a' && id[i] <= 'z') || (id[i] >= '0' && id[i] <= '9'));
  if (!idOk) {
    err = "id: 3-12 of a-z, 0-9";
    return bad();
  }
  for (const char* k : {"name", "version", "minFirmware", "about", "license"})
    if (!m[k].is<const char*>()) {
      err = String(k) + ": missing";
      return bad();
    }
  if (otarx::compareVersions(m["minFirmware"] | "", FW_VERSION) > 0) {
    err = String("needs firmware ") + (m["minFirmware"] | "");
    return bad(LoadError::FIRMWARE);
  }
  copyField(info_.id, sizeof info_.id, id);
  copyField(info_.name, sizeof info_.name, m["name"] | "");
  copyField(info_.about, sizeof info_.about, m["about"] | "");
  copyField(info_.version, sizeof info_.version, m["version"] | "");
  copyField(info_.minFirmware, sizeof info_.minFirmware, m["minFirmware"] | "");
  info_.format = sfb::FORMAT;
  const char* motion = m["motion"] | "static";
  if (strcmp(motion, "static") && strcmp(motion, "calm") && strcmp(motion, "lively")) {
    err = "motion: static, calm or lively";
    return bad();
  }
  info_.motion = !strcmp(motion, "lively") ? facefmt::LIVELY : !strcmp(motion, "calm") ? facefmt::CALM : facefmt::STATIC;
  const Info keep = info_;

  // colours, images, reactions
  JsonObjectConst cols = m["colors"];
  if (!m["colors"].isNull() && (cols.isNull() || cols.size() > MAX_COLORS)) {
    err = "colors: at most 16";
    return bad();
  }
  for (JsonPairConst kv : cols) {
    const char* name = kv.key().c_str();
    uint16_t c;
    if (!nameOk(name, 16) || !facefmt::parseHex(kv.value() | "", c)) {
      err = String("colors: bad '") + name + "'";
      return bad();
    }
    b.colors[b.nColors++] = {arena_.copy(name, strlen(name)), c};
  }
  JsonObjectConst imgs = m["images"];
  if (!m["images"].isNull() && (imgs.isNull() || (int)imgs.size() != f.nImages)) {
    err = "images: don't match the file";
    return bad();
  }
  if (m["images"].isNull() && f.nImages) {
    err = "images: don't match the file";
    return bad();
  }
  for (JsonPairConst kv : imgs) {
    const char* name = kv.key().c_str();
    const int i = kv.value()["i"] | -1;
    if (!nameOk(name, 16) || i < 0 || i >= f.nImages) {
      err = String("images: bad '") + name + "'";
      return bad();
    }
    for (int k = 0; k < b.nImageNames; k++)
      if (b.imageNames[k].i == i) {
        err = "images: one image twice";
        return bad();
      }
    b.imageNames[b.nImageNames++] = {arena_.copy(name, strlen(name)), i};
  }
  for (int i = 0; i < facefmt::EV_COUNT; i++) reactMs_[i] = 3000;
  for (JsonPairConst kv : m["reactions"].as<JsonObjectConst>()) {
    const int ev = facefmt::eventIndex(kv.key().c_str());
    const float ms = kv.value().is<float>() ? kv.value().as<float>() : -1;
    if (ev < 0 || !(ms >= 500 && ms <= 15000)) {
      err = String("reactions: bad '") + kv.key().c_str() + "'";
      return bad();
    }
    reactMs_[ev] = ms;
  }
  // parts (built once, shared by the layouts)
  JsonObjectConst parts = m["parts"];
  if (!m["parts"].isNull() && (parts.isNull() || parts.size() > MAX_PARTS)) {
    err = "parts: at most 8";
    return bad();
  }
  for (JsonPairConst kv : parts) {
    const char* name = kv.key().c_str();
    JsonArrayConst list = kv.value();
    if (!nameOk(name, 16) || list.isNull() || list.size() > MAX_KIDS) {
      err = String("parts: bad '") + name + "'";
      return bad();
    }
    Builder::Part& p = b.parts[b.nParts];
    p.name = arena_.copy(name, strlen(name));
    b.where = String("part ") + name;
    b.elems = b.drawn = 0;
    b.time = b.seconds = false;
    if (!b.children(list, p.e, p.n, 1)) return bad();
    p.elems = b.elems;
    p.drawn = b.drawn;
    p.time = b.time;
    p.seconds = b.seconds;
    b.nParts++;
  }
  // the layouts
  JsonObjectConst lays = m["layouts"];
  if (lays.isNull() || !b.only(lays, LAYOUTS)) return bad();
  bool seconds = false;
  for (int side = 0; side < 2; side++) {
    const char* name = side ? "landscape" : "portrait";
    JsonObjectConst lo = lays[name];
    b.where = name;
    if (lo.isNull()) {
      err = String("layouts: needs '") + name + "'";
      return bad();
    }
    if (!b.only(lo, LAYOUT)) return bad();
    Layout& l = layouts_[side];
    // background: a colour or a gradient
    l.bg = GROUND;
    JsonVariantConst bg = lo["bg"];
    if (bg.is<JsonObjectConst>()) {
      JsonObjectConst g = bg;
      JsonArrayConst ends = g["grad"];
      const char* dir = g["dir"] | "v";
      if (!b.only(g, GRAD) || ends.size() != 2 ||
          (strcmp(dir, "v") && strcmp(dir, "h"))) {
        b.fail("bg: {\"grad\": [two colours], \"dir\": v or h}");
        return bad();
      }
      l.grad = true;
      l.gradH = !strcmp(dir, "h");
      if (!b.color(ends[0], "bg", l.bg) || !b.color(ends[1], "bg", l.bg2)) return bad();
    } else if (!bg.isNull() && !b.color(bg, "bg", l.bg)) {
      return bad();
    }
    const char* status = lo["status"] | "light";
    l.status = !strcmp(status, "dark") ? 1 : !strcmp(status, "pill") ? 2 : 0;
    if (strcmp(status, "light") && strcmp(status, "dark") && strcmp(status, "pill")) {
      b.fail("status: light, dark or pill");
      return bad();
    }
    // the urgent card: the defaults, and the smallest the text fits in
    const int16_t def[2][4] = {{6, 184, 123, 48}, {10, 84, 220, 44}}, least[2][2] = {{110, 38}, {96, 34}};
    memcpy(l.card, def[side], sizeof l.card);
    if (!lo["card"].isNull()) {
      JsonArrayConst card = lo["card"];
      if (card.size() != 4) {
        b.fail("card: [x, y, w, h]");
        return bad();
      }
      for (int i = 0; i < 4; i++)
        if (!b.integer(card[i], "card", 0, 240, l.card[i])) return bad();
      const int sw = side ? 240 : 135, sh = side ? 135 : 240;
      if (l.card[2] < least[side][0] || l.card[3] < least[side][1] || l.card[0] + l.card[2] > sw ||
          l.card[1] + l.card[3] > sh) {
        b.fail("card: too small, or off the screen");
        return bad();
      }
    }
    JsonArrayConst list = lo["elements"];
    if (list.isNull() || !list.size() || list.size() > MAX_ELEMS) {
      b.fail("elements: 1-48");
      return bad();
    }
    b.elems = b.drawn = 0;
    b.time = b.seconds = false;
    l.elems = arena_.make<Elem>(list.size());
    if (!l.elems) {
      err = "out of memory";
      return bad(LoadError::MEMORY);
    }
    l.n = list.size();
    int i = 0;
    for (JsonVariantConst e : list) {
      b.where = String(name) + " element " + String(i);
      if (!b.elem(e, l.elems[i++], 0, 1)) return bad();
    }
    b.where = name;
    if (b.elems > MAX_ELEMS) {
      b.fail("more than 48 elements");
      return bad();
    }
    if (b.drawn > MAX_DRAWN) {
      b.fail("more than 96 things drawn");
      return bad();
    }
    if (!b.time) {
      b.fail("doesn't show the time");
      return bad();
    }
    seconds = seconds || b.seconds;
  }
  if (b.conds.size() > MAX_CONDS) {
    err = "too many conditions";
    return bad();
  }
  if (b.conds.size()) {
    conds_ = arena_.make<Cond>(b.conds.size());
    if (!conds_) {
      err = "out of memory";
      return bad(LoadError::MEMORY);
    }
    memcpy((void*)conds_, b.conds.data(), sizeof(Cond) * b.conds.size());
  }
  // a reaction keeps a static face moving while it lasts (react.<event> or its tweens)
  for (int i = 0; i < facefmt::EV_COUNT; i++)
    if (busyMs_[i] && busyMs_[i] < reactMs_[i]) busyMs_[i] = reactMs_[i];
  for (const Cond* c = b.conds.data(); c < b.conds.data() + b.conds.size(); c++)
    if (c->op <= Op::LE && c->bind >= facefmt::REACT) {
      const int ev = c->bind - facefmt::REACT;
      if (busyMs_[ev] < reactMs_[ev]) busyMs_[ev] = reactMs_[ev];
    }
  info_ = keep;
  seconds_ = seconds;
  tier_ = info_.motion;
  guard_ = facefmt::Guard{};
  loaded_ = true;
  return LoadError::NONE;
}

// --- Frame rate ----------------------------------------------------------------------------------

uint32_t Engine::frameMs(bool onBattery, uint32_t idleMs, uint32_t now) const {
  if (!loaded_) return 0;
  int tier = tier_;
  if (tier == facefmt::STATIC)  // a static face moves while it reacts
    for (int i = 0; i < facefmt::EV_COUNT; i++)
      if (busyMs_[i] && eventAt_[i] && now - eventAt_[i] < busyMs_[i] + 500) tier = facefmt::CALM;
  return facefmt::frameMs(tier, seconds_, onBattery, idleMs);
}

bool Engine::frameTook(uint32_t us, uint32_t now) {
  if (!loaded_ || tier_ <= facefmt::FROZEN || !guard_.feed(us, now)) return false;
  tier_--;
  return true;
}

// --- Events --------------------------------------------------------------------------------------

void Engine::fire(int ev, uint32_t now) {
  if (ev >= 0 && ev < facefmt::EV_COUNT) eventAt_[ev] = now ? now : 1;
}

void Engine::kick(world::Kick k, uint32_t now) { fire(facefmt::eventForKick(k), now); }

void Engine::events(const Dash& d, bool urgentOn, uint32_t now) {
  if (d.minute >= 0) {
    if (lastMinute_ != -2 && d.minute != lastMinute_) fire(facefmt::EV_MINUTE, now);
    const int hour = d.minute / 60;
    if (lastHour_ != -2 && hour != lastHour_) fire(facefmt::EV_HOUR, now);
    lastMinute_ = d.minute;
    lastHour_ = hour;
  }
  const int night = faces::isNightMinute(d.minute) ? 1 : 0;
  if (lastNight_ >= 0 && night != lastNight_) fire(night ? facefmt::EV_DUSK : facefmt::EV_DAWN, now);
  lastNight_ = night;
  if (lastUrgent_ >= 0 && urgentOn && !lastUrgent_) fire(facefmt::EV_URGENT, now);
  lastUrgent_ = urgentOn;
}

// --- Drawing -------------------------------------------------------------------------------------

struct Engine::Ctx {
  const Dash* d;
  const facefmt::Inputs* in;
  uint32_t now;
  uint16_t bg;          // the layout's background (the mix target)
  int ox = 0, oy = 0;   // the group's offset
  int inst = 0;         // the repeat's copy
};

// the tweens' sums for an element: x / y offsets, mix, colour, frame
struct Engine::Tw {
  float dx = 0, dy = 0, mix = -1;
  int frame = -1;
  bool hasColor = false;
  uint16_t color = 0, mixTo = 0;
};

void Engine::drawImage(const Image& im, int frame, int x, int y, int s, bool flip, float mix, uint16_t mixTo,
                       bool tile) {
  frame = constrain(frame, 0, im.frames - 1);
  uint16_t pal[256];
  for (int i = 0; i < im.colors; i++) pal[i] = mix > 0 ? blend(im.pal[i], mixTo, mix) : im.pal[i];
  const lgfx::rgb565_t* p565 = reinterpret_cast<const lgfx::rgb565_t*>(pal);
  const uint32_t clear = im.clear >= 0 ? (uint32_t)im.clear : lgfx::pixelcopy_t::NON_TRANSP;
  const uint8_t* fp = im.px + (size_t)frame * im.fw * im.h;
  const int fw = im.fw, tw = fw * s;
  int from = x, copies = 1;
  if (tile) {  // repeat across the screen (parallax layers)
    from = ((x % tw) + tw) % tw - tw;
    copies = (W - from + tw - 1) / tw;
  }
  for (int k = 0; k < copies; k++) {
    const int bx = from + k * tw;
    if (bx >= W || bx + tw <= 0 || y >= H || y + im.h * s <= 0) continue;
    if (s == 1 && !flip) {  // the frame as it is: one push through its palette
      canvas.pushImage(bx, y, fw, im.h, fp, clear, lgfx::palette_8bit, p565);
      continue;
    }
    for (int yy = 0; yy < im.h; yy++) {  // a row scaled / mirrored, pushed s rows high
      const int py = y + yy * s;
      if (py + s <= 0 || py >= H) continue;
      const uint8_t* src = fp + yy * fw;
      for (int xx = 0; xx < fw; xx++) {
        const uint8_t v = src[flip ? fw - 1 - xx : xx];
        for (int j = 0; j < s; j++) row_[xx * s + j] = v;
      }
      for (int j = 1; j < s; j++) memcpy(row_ + j * tw, row_, tw);
      canvas.pushImage(bx, py, tw, s, row_, clear, lgfx::palette_8bit, p565);
    }
  }
}

void Engine::drawOne(const Elem& e, const Ctx& c, int x, int y, const Tw& t) {
  const float mix = t.mix >= 0 ? t.mix : e.mix;
  const uint16_t mixTo = t.mix >= 0 ? t.mixTo : (e.has & HAS_MIX_TO) ? e.mixTo : c.bg;
  auto mixed = [&](uint16_t v) { return mix > 0 ? blend(v, mixTo, mix) : v; };
  auto main = [&](uint16_t dflt) { return mixed(t.hasColor ? t.color : (e.has & HAS_C0) ? e.c0 : dflt); };
  auto second = [&](uint16_t dflt) { return mixed((e.has & HAS_C1) ? e.c1 : dflt); };
  const bool hasFill = e.has & HAS_C0, hasStroke = e.has & HAS_C1;

  switch (e.type) {
    case Type::RECT: {
      int w = e.w, h = e.h;
      anchorBox(e.anchor, x, y, w, h);
      if (hasFill || t.hasColor) {
        if (e.r) canvas.fillSmoothRoundRect(x, y, w, h, e.r, main(INK));
        else canvas.fillRect(x, y, w, h, main(INK));
      }
      if (hasStroke) {
        if (e.r) canvas.drawRoundRect(x, y, w, h, e.r, second(INK));
        else canvas.drawRect(x, y, w, h, second(INK));
      }
      return;
    }
    case Type::LINE: {
      const int x2 = e.x2 + c.ox + (int)t.dx, y2 = e.y2 + c.oy + (int)t.dy;
      canvas.drawWideLine(x, y, x2, y2, e.width / 2, main(INK));
      return;
    }
    case Type::CIRCLE:
      if (hasFill || t.hasColor) canvas.fillSmoothCircle(x, y, e.r, main(INK));
      if (hasStroke) canvas.drawCircle(x, y, e.r, second(INK));
      return;
    case Type::ARC: {
      const float frac = e.hasValue ? facefmt::fracValue(e.value, *c.in) : 1;
      if (e.hasTrack) canvas.fillArc(x, y, e.r - e.t + 1, e.r, e.a0 - 90, e.a1 - 90, second(LINE));
      if (frac > 0) canvas.fillArc(x, y, e.r - e.t + 1, e.r, e.a0 - 90, e.a0 - 90 + (e.a1 - e.a0) * frac, main(INK));
      return;
    }
    case Type::BAR: {
      int w = e.w, h = e.h;
      anchorBox(e.anchor, x, y, w, h);
      const float frac = facefmt::fracValue(e.value, *c.in);
      if (e.hasTrack) canvas.fillRoundRect(x, y, w, h, e.r, second(LINE));
      if (e.up) {
        const int fh = lroundf(h * frac);
        if (fh > 0) canvas.fillRoundRect(x, y + h - fh, w, fh, min((int)e.r, fh / 2), main(INK));
      } else {
        const int fw = lroundf(w * frac);
        if (fw > 0) canvas.fillRoundRect(x, y, fw, h, min((int)e.r, fw / 2), main(INK));
      }
      return;
    }
    case Type::TEXT: {
      char buf[200];
      facefmt::expand(e.text, *c.in, buf, sizeof buf);
      if (e.textCase) facefmt::applyCase(buf, e.textCase > 0);
      String s(buf);
      s.trim();
      if (!s.length()) return;
      const lgfx::IFont* f = fontFor(e.font);
      for (int i = 0; i < e.nFit; i++) {  // the first that fits (biggest first)
        f = fontFor(e.fit[i]);
        canvas.setFont(f);
        if (canvas.textWidth(s) <= e.w) break;
      }
      const uint16_t ink = main(INK), shade = second(PUPIL);
      String ls[3];
      int n = 1;
      if (e.lines > 1 && e.w) n = wrap(s, f, ls, e.lines, e.w);
      else ls[0] = e.w ? ellipsize(s, f, e.w) : s;
      const int lh = lineH(f), row = e.anchor / 3;
      const textdatum_t dt = DATUMS[e.anchor];
      int by = y;  // a block of n lines: the anchor applies to the block
      if (n > 1) {
        if (row == 1) by = y - (n - 1) * lh / 2;
        else if (row == 2) by = y - (n - 1) * lh;
      }
      for (int i = 0; i < n; i++) {
        const int ly = by + i * lh;
        if (e.has & OUTLINE) faces::outlineTxt(ls[i], x, ly, f, ink, shade, dt, f == F56 || f == F96 ? 2 : 1);
        else if (e.has & SHADOW) faces::shadowTxt(ls[i], x, ly, f, ink, shade, dt);
        else txt(ls[i], x, ly, f, ink, dt);
      }
      return;
    }
    case Type::IMAGE: {
      const Image& im = images_[e.image];
      int frame = (int)facefmt::numValue(e.frame, *c.in);
      if (e.fpsMilli) {  // frames per 1000 s, in whole numbers: steady for weeks of millis()
        const uint32_t k = (uint32_t)((uint64_t)c.now * e.fpsMilli / 1000000ull) + c.inst;
        frame = e.nSeq ? e.seq[k % e.nSeq] : (int)(k % im.frames);
      }
      if (t.frame >= 0) frame = t.frame;
      anchorBox(e.anchor, x, y, im.fw * e.scale, im.h * e.scale);
      drawImage(im, frame, x, y, e.scale, e.flip, mix, mixTo, e.tile);
      return;
    }
    case Type::HAND: {
      const Dash& d = *c.d;
      if (e.hand == 2 && (d.minute < 0 || d.second < 0)) return;  // no second hand before the time is known
      clockface::Hands hs = clockface::hands(d.minute >= 0 ? d.minute : 10 * 60 + 10, d.second >= 0 ? d.second : 0);
      const float deg = e.hand == 0 ? hs.hour : e.hand == 2 ? hs.second : hs.minute;
      const float a = deg * PI / 180, sn = sinf(a), cs = cosf(a), len = e.length, tail = e.tail;
      if (hasStroke)
        canvas.drawWedgeLine(x + 1 - sn * tail, y + 2 + cs * tail, x + 1 + sn * len, y + 2 - cs * len, e.w0, e.w1,
                             second(PUPIL));
      canvas.drawWedgeLine(x - sn * tail, y + cs * tail, x + sn * len, y - cs * len, e.w0, e.w1, main(INK));
      return;
    }
    case Type::TICKS:
      for (int i = 0; i < e.count; i++) {
        const bool major = e.every && i % e.every == 0;
        const float a = i * 2 * PI / e.count, sn = sinf(a), cs = -cosf(a);
        const int len = major ? e.majorLen : e.len;
        const float wd = major ? e.majorWidth : e.width;
        uint16_t cc = major && hasStroke ? second(INK) : main(INK);
        if (i == 0 && (e.has & HAS_C2)) cc = mixed(e.c2);
        if (len <= 1 && wd <= 1) canvas.fillRect(x + sn * (e.r - 1), y + cs * (e.r - 1), 1, 1, cc);
        else canvas.drawWideLine(x + sn * e.r, y + cs * e.r, x + sn * (e.r - len), y + cs * (e.r - len), wd / 2, cc);
      }
      return;
    case Type::WEATHER: {
      if (!c.d->wxValid) return;
      anchorBox(e.anchor, x, y, e.size, e.size);
      weatherIcon(c.d->wxIcon, c.d->wxDay, x, y, e.size);
      return;
    }
    case Type::SCATTER: {
      const int w = max(1, (int)e.w), h = max(1, (int)e.h);
      const uint32_t seed = (uint32_t)e.seed * 7919u;
      const uint16_t base = main(INK);
      for (int i = 0; i < e.n; i++) {
        const uint32_t hx = faces::hash32(seed + i * 2), hy = faces::hash32(seed + i * 2 + 1);
        int px = (int)((hx % w + (int64_t)e.drift * c.now / 1000) % w);
        int py = (int)((hy % h + (int64_t)e.fall * c.now / 1000) % h);
        if (px < 0) px += w;
        if (py < 0) py += h;
        uint16_t cc = base;
        if (e.twinkle) {
          const float ph =
              sinf(2 * PI * ((float)(c.now % (uint32_t)e.twinkle) / e.twinkle + faces::frac01(hx ^ 0x9e37)));
          cc = blend(base, c.bg, 0.4f + 0.4f * ph);
        }
        canvas.fillRect(x + px, y + py, e.size, e.size, cc);
      }
      return;
    }
    case Type::GROUP:
      return;  // (element() draws its children)
  }
}

void Engine::element(const Elem& e, const Ctx& c, int depth) {
  if (e.when >= 0 && !facefmt::eval(conds_, e.when, *c.in)) return;
  int count = e.repCount;
  if (e.repBind >= 0) {
    facefmt::Value v;
    facefmt::value(e.repBind, *c.in, v);
    count = (int)v.n;
  }
  count = constrain(count, 0, min(16, (int)e.repMax));
  for (int i = 0; i < count; i++) {
    Ctx ci = c;
    ci.inst = i;
    Tw t;
    t.mixTo = c.bg;
    const uint16_t base = (e.has & HAS_C0) ? e.c0 : INK;
    for (int k = 0; k < e.nTweens; k++) {
      const Tween& tw = e.tweens[k];
      float f;
      if (!facefmt::progress(tw, c.now, tw.on >= 0 ? eventAt_[tw.on] : 0, i, f)) continue;
      switch (tw.prop) {
        case facefmt::Prop::COLOR:
          t.color = blend(tw.hasFrom ? tw.cFrom : base, tw.hasTo ? tw.cTo : base, f);
          t.hasColor = true;
          break;
        case facefmt::Prop::X: t.dx += tw.from + (tw.to - tw.from) * f; break;
        case facefmt::Prop::Y: t.dy += tw.from + (tw.to - tw.from) * f; break;
        case facefmt::Prop::MIX:
          t.mix = constrain(tw.from + (tw.to - tw.from) * f, 0.0f, 1.0f);
          t.mixTo = tw.hasMixTo ? tw.mixTo : c.bg;
          break;
        case facefmt::Prop::FRAME: t.frame = (int)lroundf(tw.from + (tw.to - tw.from) * f); break;
      }
    }
    const int x = (int)lroundf(facefmt::numValue(e.x, *c.in) + t.dx) + c.ox + i * e.repDx;
    const int y = (int)lroundf(facefmt::numValue(e.y, *c.in) + t.dy) + c.oy + i * e.repDy;
    if (e.type == Type::GROUP) {
      if (depth > 0) continue;
      ci.ox = x;
      ci.oy = y;
      for (int k = 0; k < e.nKids; k++) element(e.kids[k], ci, depth + 1);
      continue;
    }
    drawOne(e, ci, x, y, t);
  }
}

void Engine::draw(const Dash& d, const world::Snapshot& w, bool wide, uint32_t now) {
  if (!loaded_) return;
  const faces::Urgent u = faces::urgent(d);
  events(d, u.on, now);
  facefmt::Inputs in;
  in.minute = d.minute;
  in.second = d.second;
  in.clock24 = d.clock24;
  in.wday = d.wday;
  in.mday = d.mday;
  in.mon = d.mon;
  in.dateShort = d.date.c_str();
  in.dateLong = d.longDate.c_str();
  in.wxValid = d.wxValid;
  in.wxDay = d.wxDay;
  in.wxTemp = d.wxTemp;
  in.wxHi = d.wxHi;
  in.wxLo = d.wxLo;
  in.wxIcon = d.wxIcon.c_str();
  in.wxCond = d.wxCond.c_str();
  in.battery = d.battery;
  in.charging = d.charging;
  in.bt = d.bt;
  in.name = d.name.c_str();
  in.agentLinked = d.agentLinked;
  in.cardKind = d.cardKind.c_str();
  in.cardA = d.cardA.c_str();
  in.cardB = d.cardB.c_str();
  in.cardUrgent = d.cardUrgent;
  in.urgentOn = u.on;
  in.world = &w;
  in.buddyLinked = d.linked;
  in.buddyTotal = d.sessions;
  in.buddyRunning = d.running;
  in.buddyWaiting = d.waiting;
  in.buddyTokens = d.tokens;
  in.tiltX = tiltX_;
  in.tiltY = tiltY_;
  in.now = now;
  in.eventAt = eventAt_;
  in.reactMs = reactMs_;

  const Layout& l = layouts_[wide ? 1 : 0];
  Ctx c{&d, &in, now, l.bg};
  if (l.grad) {
    const int n = l.gradH ? W : H;
    for (int i = 0; i < n; i += 2) {
      const uint16_t cc = blend(l.bg, l.bg2, (float)i / (n - 1));
      if (l.gradH) canvas.fillRect(i, 0, 2, H, cc);
      else canvas.fillRect(0, i, W, 2, cc);
    }
  } else {
    canvas.fillScreen(l.bg);
  }
  for (int i = 0; i < l.n; i++) element(l.elems[i], c, 0);

  // What the firmware always draws over a face (FORMAT.md, "What the stick always shows")
  if (l.status == 2) canvas.fillRoundRect(W - 76, 1, 75, 17, 8, rgb(0x11, 0x13, 0x16));
  faces::statusIcons(d, W - 4, 9, l.status == 1 ? rgb(0x1a, 0x1d, 0x21) : INK);
  if (u.on) faces::signBoard(l.card[0], l.card[1], l.card[2], l.card[3], u.a, u.b, u.bg, u.ink);
}

}  // namespace pkg
}  // namespace faces
