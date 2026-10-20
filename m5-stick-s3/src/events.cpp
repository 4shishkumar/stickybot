#include "events.h"
#include "logic/fixed_queue.h"

namespace events {

constexpr size_t   MAX_QUEUED = 5;
constexpr uint32_t CELEBRATE_MS = 6000, NICE_MS = 4000, HEADSUP_MS = 60000;

static FixedQueue<Event, MAX_QUEUED> queue;
static Event    cur;
static bool     active_ = false;
static uint32_t since = 0;

bool parseLevel(const String& s, Level& out) {
  if (s == "celebrate") out = Level::CELEBRATE;
  else if (s == "nice") out = Level::NICE;
  else if (s == "headsup") out = Level::HEADSUP;
  else return false;
  return true;
}

void push(const Event& e) { queue.push(e); }  // full: drops the oldest

bool pending() { return !queue.empty(); }
bool active() { return active_; }
const Event& current() { return cur; }

void start() {
  if (!queue.pop(cur)) return;
  active_ = true;
  since = millis();
}

uint32_t elapsed() { return millis() - since; }

bool expired() {
  uint32_t ms = cur.level == Level::CELEBRATE ? CELEBRATE_MS : cur.level == Level::NICE ? NICE_MS : HEADSUP_MS;
  return elapsed() >= ms;
}

void finish() { active_ = false; }

const char* defaultSound(Level l) {
  return l == Level::CELEBRATE ? "jingle_win" : l == Level::NICE ? "confirmation_002" : "question_001";
}

}  // namespace events
