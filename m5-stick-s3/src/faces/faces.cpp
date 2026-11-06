#include "faces.h"
#include <Preferences.h>
#include "common.h"
#include "../netlog.h"

namespace faces {

// --- Registry: add a face here (and its file) --------------------------------------
#define FACES(X)                                                                            \
  X(classic) X(stickyWorld) X(aquarium) X(bigTimeFace) X(wordsFace) X(analog) X(dashboard) \
  X(space) X(lava)

#define FACE_DECLARE(f) Face& f();
FACES(FACE_DECLARE)
#define FACE_REF(f) &f(),

static Face* const* list(int& n) {
  static Face* const L[] = {FACES(FACE_REF)};
  n = sizeof L / sizeof L[0];
  return L;
}

int count() {
  int n;
  list(n);
  return n;
}

Face& at(int i) {
  int n;
  Face* const* l = list(n);
  return *l[constrain(i, 0, n - 1)];
}

int find(const String& id) {
  for (int i = 0; i < count(); i++)
    if (id == at(i).id()) return i;
  return -1;
}

static int active_ = 0, demoFace_ = -1;

Face& active() { return at(active_); }

void select(const String& id) {
  int i = find(id);
  int next = i < 0 ? 0 : i;
  if (next != active_) at(next).reset();
  active_ = next;
}

String json() {
  String j = "[";
  for (int i = 0; i < count(); i++) {
    Face& f = at(i);
    j += String(i ? "," : "") + "{\"id\":\"" + f.id() + "\",\"title\":\"" + f.title() + "\",\"about\":\"" + f.about() +
         "\"}";
  }
  return j + "]";
}

// --- Home --------------------------------------------------------------------------

static Face& shown() { return demoFace_ >= 0 ? at(demoFace_) : active(); }

void drawHome(const ui::Dash& d, bool wide) { shown().draw(d, wide, millis()); }

uint32_t frameMs(bool onBattery, uint32_t idleMs) { return shown().frameMs(onBattery, idleMs); }

// The same reaction twice in a row (the event and the world update both say "merged"):
// once. Toys can repeat sooner.
static uint32_t lastKick[16];

static bool isToy(world::Kick k) {
  return k == world::Kick::SHAKE || k == world::Kick::FLICK || k == world::Kick::UPSIDE_DOWN || k == world::Kick::LIFT;
}

static void dispatch(world::Kick k) {
  uint8_t i = (uint8_t)k;
  if (k == world::Kick::NONE || i >= 16) return;
  uint32_t now = millis(), gap = isToy(k) ? 1500 : 45000;
  if (lastKick[i] && now - lastKick[i] < gap) return;
  lastKick[i] = now ? now : 1;
  shown().kick(k, now);
}

void toy(world::Kick k) { dispatch(k); }

void demoKick(world::Kick k) {
  lastKick[(uint8_t)k] = 0;  // the demo shows each one, however soon after the last
  dispatch(k);
}

// --- The world -----------------------------------------------------------------------

static world::Snapshot live_, saved_;
static const world::Snapshot* demoWorld_ = nullptr;
static Preferences store;
constexpr uint8_t CACHE_VERSION = 1;

const world::Snapshot& world() { return demoWorld_ ? *demoWorld_ : live_; }

// The part worth keeping across a restart (not the sessions or what waits right now)
static world::Snapshot keep(const world::Snapshot& w) {
  world::Snapshot k;
  k.valid = w.valid;
  k.mergedToday = w.mergedToday;
  k.merged = w.merged;
  k.ci = w.ci;
  k.xp = w.xp;
  k.streak = w.streak;
  k.acc = w.acc;
  k.seed = w.seed;
  k.greens = w.greens;
  k.nTowers = w.nTowers;
  for (int i = 0; i < w.nTowers; i++) k.towers[i] = w.towers[i];
  return k;
}

static bool sameKept(const world::Snapshot& a, const world::Snapshot& b) {
  if (a.merged != b.merged || a.mergedToday != b.mergedToday || a.ci != b.ci || a.xp != b.xp ||
      a.streak != b.streak || a.acc != b.acc || a.seed != b.seed || a.greens != b.greens || a.nTowers != b.nTowers)
    return false;
  for (int i = 0; i < a.nTowers; i++)
    if (a.towers[i] != b.towers[i]) return false;
  return true;
}

void begin() {
  store.begin("faces", false);
  if (store.getUChar("v", 0) == CACHE_VERSION && store.getBytesLength("world") == sizeof(world::Snapshot)) {
    store.getBytes("world", &live_, sizeof live_);
    live_.nSessions = 0;
    live_.asks = live_.reviews = 0;
    saved_ = live_;
    Log.printf("Faces: cached world, %d PRs, %ld XP\n", live_.merged, live_.xp);
  }
}

void onWorld(JsonObjectConst j) {
  world::Snapshot w;
  w.valid = true;
  w.mergedToday = j["mergedToday"] | 0;
  w.merged = j["merged"] | 0;
  w.ci = world::parseCi(j["ci"] | "");
  w.asks = j["asks"] | 0;
  w.reviews = j["reviews"] | 0;
  for (JsonObjectConst s : j["claude"].as<JsonArrayConst>()) {
    if (w.nSessions >= world::MAX_SESSIONS) break;
    world::Session& ss = w.sessions[w.nSessions];
    ss.id = s["id"] | (w.nSessions + 1);
    world::setProject(ss, s["p"] | "");
    ss.secs = s["s"] | 0;
    w.nSessions++;
  }
  JsonObjectConst p = j["pet"];
  w.xp = p["xp"] | 0L;
  w.streak = p["streak"] | 0;
  w.acc = p["acc"] | 0;
  w.seed = p["seed"] | 1;
  w.greens = p["greens"] | 0;
  for (int t : p["towers"].as<JsonArrayConst>()) {
    if (w.nTowers >= world::MAX_TOWERS) break;
    w.towers[w.nTowers++] = t;
  }
  world::Kick k[8];
  int n = world::diff(live_, w, k, 8);
  live_ = w;
  if (!demoWorld_)
    for (int i = 0; i < n; i++) dispatch(k[i]);
  if (!sameKept(live_, saved_) || !saved_.valid) {  // rare: a PR merged, XP...
    saved_ = keep(live_);
    store.putBytes("world", &saved_, sizeof saved_);
    store.putUChar("v", CACHE_VERSION);
  }
}

void onEventKind(const char* kind) {
  if (!demoWorld_) dispatch(world::kickForEvent(kind));
}

void setDemo(int face, const world::Snapshot* w) {
  if (face != demoFace_) {  // a fresh start for each face shown, and for the real one after
    if (demoFace_ >= 0) at(demoFace_).reset();
    if (face >= 0) at(face).reset();
    else active().reset();
  }
  demoFace_ = face;
  demoWorld_ = w;
}

}  // namespace faces
