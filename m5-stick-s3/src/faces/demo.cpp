// The faces' demo: each face through its moods and animations with made-up data, portrait
// then landscape, a caption saying what's going on. Started by the serial command
// "screens faces" (every face) or "@face demo=<id>" (the dashboard's Preview buttons).
// It runs inside the main loop (it doesn't block): a press of A or B ends it, and an
// approval or an event screen still comes first.
#include "faces.h"
#include "common.h"
#include "../logic/clock.h"

namespace faces {

namespace {

struct State {
  world::Snapshot w;
  int    minute = 8 * 60 + 30;
  String wx = "clear";
  bool   urgent = false;
  String cardKind = "fact", cardA = "Did you know?", cardB = "Honey never spoils.";
};

typedef void (*Apply)(State&);

struct Step {
  uint16_t    ms;
  int8_t      side;   // 0 portrait, 1 landscape
  const char* label;
  Apply       apply;
  world::Kick toy;
};

static void addSession(State& s, uint16_t id, const char* p, uint32_t secs) {
  if (s.w.nSessions >= world::MAX_SESSIONS) return;
  world::Session& x = s.w.sessions[s.w.nSessions++];
  x.id = id;
  x.secs = secs;
  world::setProject(x, p);
}

static void dropSession(State& s, uint16_t id) {
  int n = 0;
  for (int i = 0; i < s.w.nSessions; i++)
    if (s.w.sessions[i].id != id) s.w.sessions[n++] = s.w.sessions[i];
  s.w.nSessions = n;
}

static void merge(State& s) {
  s.w.merged++;
  s.w.mergedToday++;
  s.w.xp += 10;
}

using world::Kick;
static const Step SCRIPT[] = {
    {3500, 0, "Morning, all good", [](State& s) { s.minute = 8 * 60 + 30; s.wx = "clear"; s.w.ci = world::Ci::GREEN; }, Kick::NONE},
    {3500, 0, "Claude sessions running", [](State& s) {
       addSession(s, 1, "web", 300); addSession(s, 2, "api", 2700); addSession(s, 3, "docs", 40); }, Kick::NONE},
    {4000, 0, "PR merged", merge, Kick::NONE},
    {3200, 0, "Review requested", [](State& s) { s.w.reviews = 1; }, Kick::NONE},
    {3600, 0, "CI failed", [](State& s) { s.w.reviews = 0; s.w.ci = world::Ci::RED; }, Kick::NONE},
    {3000, 0, "CI fixed", [](State& s) { s.w.ci = world::Ci::GREEN; s.w.greens++; s.w.xp += 3; }, Kick::NONE},
    {3500, 0, "A Claude session finished", [](State& s) { dropSession(s, 1); s.w.xp += 2; }, Kick::NONE},
    {3500, 0, "Needs you: an approval", [](State& s) { s.urgent = true; }, Kick::NONE},
    {4500, 0, "Release!", [](State& s) {
       s.urgent = false;
       if (s.w.nTowers < world::MAX_TOWERS) s.w.towers[s.w.nTowers++] = s.w.merged;
       s.w.xp += 25; }, Kick::NONE},
    {3000, 0, "Rain", [](State& s) { s.minute = 15 * 60; s.wx = "rain"; }, Kick::NONE},
    {3000, 0, "Snow at dusk", [](State& s) { s.minute = 18 * 60 + 40; s.wx = "snow"; }, Kick::NONE},
    {3200, 0, "Night", [](State& s) { s.minute = 23 * 60 + 10; s.wx = "clear"; s.w.nSessions = 0; }, Kick::NONE},
    {2800, 0, "Shake", [](State& s) { s.minute = 10 * 60; addSession(s, 4, "web", 900); }, Kick::SHAKE},
    {2000, 0, "Flick", [](State&) {}, Kick::FLICK},
    {3500, 1, "Landscape", [](State& s) { s.minute = 11 * 60; s.wx = "partly"; addSession(s, 5, "infra", 1200); }, Kick::NONE},
    {4000, 1, "PR merged", merge, Kick::NONE},
    {3000, 1, "Needs you", [](State& s) { s.urgent = true; }, Kick::NONE},
    {3000, 1, "Rain", [](State& s) { s.urgent = false; s.wx = "rain"; s.minute = 16 * 60; }, Kick::NONE},
    {3200, 1, "Night", [](State& s) { s.wx = "clear"; s.minute = 22 * 60 + 30; }, Kick::NONE},
};
// The quiet faces (the clocks): a few times of day, something urgent, landscape
static const Step CLOCKS[] = {
    {3000, 0, "Morning", [](State&) {}, Kick::NONE},
    {3000, 0, "Needs you", [](State& s) { s.urgent = true; }, Kick::NONE},
    {3000, 0, "Evening", [](State& s) { s.urgent = false; s.minute = 18 * 60 + 47; s.wx = "partly"; }, Kick::NONE},
    {3000, 1, "Landscape", [](State& s) { s.minute = 14 * 60 + 5; }, Kick::NONE},
    {3000, 1, "Needs you", [](State& s) { s.urgent = true; }, Kick::NONE},
    {3000, 1, "Late", [](State& s) { s.urgent = false; s.minute = 23 * 60 + 58; s.wx = "clear"; }, Kick::NONE},
};

static State    st;
static int      order[16], nOrder = 0, pos = -1, step = -1;
static uint32_t stepAt = 0;
static bool     on = false;

// The clocks get the short script; the faces with a world to show, the whole story.
static const Step* script(int& n) {
  static const char* const QUIET[] = {"classic", "bigtime", "words", "analog"};
  bool quiet = false;
  for (const char* q : QUIET) quiet = quiet || !strcmp(at(order[pos]).id(), q);
  n = quiet ? sizeof CLOCKS / sizeof CLOCKS[0] : sizeof SCRIPT / sizeof SCRIPT[0];
  return quiet ? CLOCKS : SCRIPT;
}

static void freshWorld() {
  st = State{};
  st.w.valid = true;
  st.w.merged = 23;
  st.w.mergedToday = 1;
  st.w.xp = 380;
  st.w.streak = 6;
  st.w.greens = 12;
  st.w.acc = 1 | 2;  // the hat (5-day streak) and the shades (10 green builds)
  st.w.seed = 20261106;
  st.w.towers[0] = 9;
  st.w.nTowers = 1;
}

// Next step (or the next face); false when it's all done.
static bool advance() {
  int n;
  step++;
  if (pos < 0 || step >= (script(n), n)) {
    pos++;
    step = 0;
    if (pos >= nOrder) return false;
    freshWorld();
    setDemo(order[pos], &st.w);
  }
  const Step* s = script(n);
  world::Snapshot before = st.w;
  s[step].apply(st);
  world::Kick k[8];
  int nk = world::diff(before, st.w, k, 8);
  for (int i = 0; i < nk; i++) demoKick(k[i]);
  if (s[step].toy != Kick::NONE) demoKick(s[step].toy);
  stepAt = millis();
  return true;
}

}  // namespace

bool demoStart(const String& id) {
  nOrder = 0;
  if (!id.length() || id == "all") {
    for (int i = 0; i < count() && nOrder < (int)(sizeof order / sizeof order[0]); i++) order[nOrder++] = i;
  } else {
    int i = find(id);
    if (i < 0) return false;
    order[nOrder++] = i;
  }
  pos = -1;
  step = -1;
  on = advance();
  return on;
}

void demoStop() {
  if (!on) return;
  on = false;
  setDemo(-1, nullptr);
}

bool demoActive() { return on; }

bool demoFrame(const ui::Dash& base, ui::Dash& d, int& side, String& caption) {
  if (!on) return false;
  int n;
  while (millis() - stepAt >= script(n)[step].ms) {
    if (!advance()) {
      demoStop();
      return false;
    }
  }
  const Step& s = script(n)[step];
  d = base;
  d.agentLinked = true;
  d.bt = 0;
  if (d.battery >= 0 && d.battery <= 15) d.battery = 80;
  d.minute = st.minute;
  clockfmt::Local t{st.minute / 60, st.minute % 60, 2, 20, 9};
  d.clock = clockfmt::time12(t).c_str();
  d.ampm = clockfmt::ampm(t).c_str();
  if (!d.date.length()) d.date = "Tue 20 Oct";
  d.wxValid = true;
  d.wxIcon = st.wx;
  d.wxDay = st.minute >= 6 * 60 && st.minute < 19 * 60;
  d.wxTemp = st.wx == "snow" ? -2 : st.wx == "rain" ? 14 : 22;
  d.wxHi = d.wxTemp + 4;
  d.wxLo = d.wxTemp - 7;
  d.wxCond = st.wx == "partly" ? "partly cloudy" : st.wx;
  d.cardUrgent = st.urgent;
  d.cardKind = st.urgent ? "approval" : st.cardKind;
  d.cardA = st.urgent ? "Claude wants Bash" : st.cardA;
  d.cardB = st.urgent ? "A yes  ·  B no" : st.cardB;
  d.cardIndex = 1;
  d.cardCount = 4;
  side = s.side;
  caption = String(at(order[pos]).title()) + " - " + s.label;
  return true;
}

}  // namespace faces
