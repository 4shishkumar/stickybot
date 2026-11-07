// Face packages' arithmetic (docs/faces/FORMAT.md): the data bindings and their text, text
// templates, conditions, tweens, the frame-rate tiers and the guard rail. The engine
// (faces/pkg/engine.cpp) parses a manifest once into these compact forms and draws with
// them every frame. Pure C++ (native-testable).
#pragma once
#include <stddef.h>
#include <stdint.h>
#include "world.h"

namespace facefmt {

// --- Events (FORMAT.md section 10): the index is the bit in the engine's event table -----
enum Event : uint8_t {
  EV_MERGED, EV_CI_RED, EV_CI_GREEN, EV_CLAUDE_START, EV_CLAUDE_DONE, EV_RELEASE, EV_LEVEL_UP,
  EV_REVIEW, EV_SHAKE, EV_FLICK, EV_UPSIDE_DOWN, EV_LIFT, EV_MINUTE, EV_HOUR, EV_DUSK, EV_DAWN,
  EV_URGENT, EV_COUNT
};
extern const char* const EVENT_NAMES[EV_COUNT];
int eventIndex(const char* name);      // -1 unknown
int eventForKick(world::Kick k);       // -1 for NONE

// --- Bindings (section 7) ------------------------------------------------------------------
enum Bind : uint8_t {
  TIME_KNOWN, TIME_HM, TIME_HM24, TIME_H12, TIME_HH12, TIME_H24, TIME_HH24, TIME_MM, TIME_SS,
  TIME_AMPM, TIME_HOUR, TIME_MINUTE, TIME_SECOND, TIME_MINUTE_OF_DAY, TIME_WORDS, TIME_WORDS_LEAD,
  TIME_WORDS_HOUR,
  DATE_WEEKDAY, DATE_DOW, DATE_DAY, DATE_DD, DATE_MONTH, DATE_MON, DATE_MM, DATE_SHORT, DATE_LONG,
  WX_VALID, WX_TEMP, WX_HI, WX_LO, WX_ICON, WX_COND, WX_DAY,
  DEV_BATTERY, DEV_CHARGING, DEV_BT, DEV_NAME, AGENT_LINKED,
  CARD_KIND, CARD_A, CARD_B, CARD_URGENT, URGENT_ON,
  WORK_KNOWN, WORK_ASKS, WORK_REVIEWS, WORK_SESSIONS, WORK_CI, WORK_MERGED, WORK_MERGED_TODAY,
  BUDDY_TOTAL, BUDDY_RUNNING, BUDDY_WAITING, BUDDY_TOKENS,
  PET_LEVEL, PET_XP, PET_INTO, PET_NEED, PET_STREAK, PET_GREENS, PET_HAT, PET_SHADES, PET_CROWN,
  SKY_NIGHT, SKY_DAY, MOTION_TILT_X, MOTION_TILT_Y,
  REACT,                       // react.<event>: REACT + the event's index
  BIND_COUNT = REACT + EV_COUNT
};
int         bindingIndex(const char* name);  // "time.hm" -> TIME_HM, -1 unknown
const char* bindingName(int id);
bool        secondsBinding(int id);          // changes every second (the static tier redraws with it)

// What the bindings read: the engine fills it from ui::Dash, the world and the clock each
// frame. Strings are borrowed (they must outlive the call).
struct Inputs {
  // the clock: minute < 0 until the agent has sent the time
  int  minute = -1, second = -1;
  bool clock24 = false;               // the 12 / 24-hour setting: time.hm follows it
  int  wday = -1, mday = 0, mon = 0;  // the date (wday 0 Sunday, mon 0 January), wday -1 unknown
  const char *dateShort = "", *dateLong = "";
  bool wxValid = false, wxDay = true;
  int  wxTemp = 0, wxHi = 0, wxLo = 0;
  const char *wxIcon = "", *wxCond = "";
  int  battery = -1;                  // -1 unknown
  bool charging = false;
  int  bt = 0;                        // 0 connected, 1 waiting, 2 off
  const char* name = "";
  bool agentLinked = false;
  const char *cardKind = "", *cardA = "", *cardB = "";
  bool cardUrgent = false, urgentOn = false;
  const world::Snapshot* world = nullptr;
  bool buddyLinked = false;           // the Claude desktop app
  int  buddyTotal = 0, buddyRunning = 0, buddyWaiting = 0;
  long buddyTokens = 0;
  float tiltX = 0, tiltY = 0;
  uint32_t now = 0;
  const uint32_t* eventAt = nullptr;  // [EV_COUNT] millis() of each event, 0 never
  const uint32_t* reactMs = nullptr;  // [EV_COUNT] how long react.<event> holds
};

struct Value {
  bool  known = false, num = false;
  float n = 0;
  char  s[96] = {0};
  bool truthy() const { return known && (num ? n != 0 : s[0] != 0); }
  float number() const { return num ? n : truthy() ? 1 : 0; }
};
void value(int id, const Inputs& in, Value& out);

// --- Text templates: "{time.hm}", "{date.dd:02}", "{wx.cond:upper}" ------------------------
// Compiled once into the same text with each binding as 3 bytes (0x01, id + 1, fmt + 1), so
// drawing is a walk with no lookups. out needs strlen(src) + 1 bytes (never longer).
enum Fmt : uint8_t { FMT_NONE, FMT_PAD2, FMT_PAD3, FMT_PAD4, FMT_UPPER, FMT_LOWER };
bool   compileTemplate(const char* src, char* out, size_t cap, const char** err = nullptr);
size_t expand(const char* compiled, const Inputs& in, char* out, size_t cap);
bool   templateUses(const char* compiled, bool (*pred)(int id));
void   applyCase(char* s, bool upper);   // ASCII letters only

// --- Numbers from bindings: var * mul + add, clamped ----------------------------------------
struct Num {
  int8_t bind = -1;      // -1: the constant k
  float  k = 0, mul = 1, add = 0, lo = 0, hi = 0;
  bool   hasLo = false, hasHi = false;
};
float numValue(const Num& n, const Inputs& in);
// A 0..1 fill: a constant, or var / of (clamped); unknown = 0
struct Frac {
  int8_t bind = -1;
  float  k = 1, of = 1;
};
float fracValue(const Frac& f, const Inputs& in);

// --- Conditions (section 8): nodes in one array, children of all / any contiguous -----------
enum class Op : uint8_t { TRUTHY, FALSY, EQ, NE, GT, GE, LT, LE, ALL, ANY, NOT };
struct Cond {
  Op       op = Op::TRUTHY;
  uint8_t  bind = 0;      // TRUTHY..LE
  uint8_t  n = 0;         // ALL / ANY: children; NOT: 1
  uint16_t first = 0;     // ALL / ANY / NOT: the first child's index
  bool     isStr = false; // EQ / NE against a string
  float    num = 0;       // the right-hand number (true = 1)
  const char* str = "";   // the right-hand string
};
constexpr int MAX_COND_DEPTH = 3;
bool eval(const Cond* nodes, int at, const Inputs& in, int depth = 0);

// --- Tweens (section 9) -----------------------------------------------------------------------
enum class Prop : uint8_t { X, Y, MIX, COLOR, FRAME };
enum class Ease : uint8_t { LINEAR, IN, OUT, INOUT, SINE, BACK };
enum class Loop : uint8_t { REPEAT, PINGPONG, ONCE };
struct Tween {
  Prop     prop = Prop::X;
  Ease     ease = Ease::LINEAR;
  Loop     loop = Loop::REPEAT;
  int8_t   on = -1;        // the event that starts it, -1 always running
  uint8_t  times = 1;      // cycles to play after `on`
  bool     hasFrom = false, hasTo = false, hasMixTo = false;  // COLOR / MIX: the colours given
  uint32_t ms = 1000, delay = 0, phase = 0;
  float    from = 0, to = 0;
  uint16_t cFrom = 0, cTo = 0, mixTo = 0;  // RGB565
};
float ease(Ease e, float t);
// The eased progress 0..1 of a tween now (inst: the repeat copy, for `phase`). false: an `on`
// tween that isn't playing (before its event, or done), so it adds nothing.
bool  progress(const Tween& t, uint32_t now, uint32_t eventAt, int inst, float& f);

// --- Colours ---------------------------------------------------------------------------------
constexpr uint16_t rgb565(uint8_t r, uint8_t g, uint8_t b) {
  return ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3);
}
bool     parseHex(const char* s, uint16_t& out);   // "#rrggbb"
uint16_t blend(uint16_t a, uint16_t b, float t);   // as ui::blend: t 0 -> a, 1 -> b

// --- Frame rate (section 11) -----------------------------------------------------------------
enum Tier : int8_t { FROZEN = -1, STATIC = 0, CALM = 1, LIVELY = 2 };
// ms between frames; 0 = only when something changes. seconds: the face shows seconds.
uint32_t frameMs(int tier, bool seconds, bool onBattery, uint32_t idleMs);

// The guard rail: frames slower than SLOW_US for SLOW_FOR_MS in a row lower the tier.
struct Guard {
  static constexpr uint32_t SLOW_US = 30000, SLOW_FOR_MS = 2000;
  uint32_t slowSince = 0;
  bool     slow = false;
  // One frame drawn (its time in us, at now ms); true = lower the tier now.
  bool feed(uint32_t frameUs, uint32_t now);
};

}  // namespace facefmt
