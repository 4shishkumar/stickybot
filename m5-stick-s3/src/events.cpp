#include "events.h"

namespace events {

constexpr size_t   MAX_QUEUED = 5;
constexpr uint32_t CELEBRATE_MS = 6000, NICE_MS = 4000, HEADSUP_MS = 60000;

static Event    queue[MAX_QUEUED];
static size_t   count = 0;
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

void push(const Event& e) {
  if (count == MAX_QUEUED) {  // full: drop the oldest
    for (size_t i = 1; i < count; i++) queue[i - 1] = queue[i];
    count--;
  }
  queue[count++] = e;
}

bool pending() { return count > 0; }
bool active() { return active_; }
const Event& current() { return cur; }

void start() {
  if (!count) return;
  cur = queue[0];
  for (size_t i = 1; i < count; i++) queue[i - 1] = queue[i];
  count--;
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
