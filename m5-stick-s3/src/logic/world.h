// What the faces know about your work: the agent's "world" message (PROTOCOL.md, Faces),
// and what changed between two of them. Pure C++ (native-testable).
#pragma once
#include <stdint.h>

namespace world {

constexpr int MAX_SESSIONS = 6;
constexpr int MAX_TOWERS = 8;
constexpr int PROJECT_LEN = 13;  // 12 characters + the terminator

enum class Ci : uint8_t { NONE, GREEN, RED };
Ci parseCi(const char* s);  // "green" | "red" | anything else = NONE

struct Session {
  uint16_t id = 0;               // stable per Claude Code session (the agent hashes its id)
  char     project[PROJECT_LEN] = {0};
  uint32_t secs = 0;             // running for this long
};

struct Snapshot {
  bool     valid = false;   // heard from the agent at least once (or loaded from the cache)
  int      mergedToday = 0;
  int      merged = 0;      // merged PRs in all = buildings in the city
  Ci       ci = Ci::NONE;
  int      asks = 0;        // approvals / questions waiting
  int      reviews = 0;     // review requests waiting
  Session  sessions[MAX_SESSIONS];
  int      nSessions = 0;
  // pet progress (kept by the agent)
  long     xp = 0;
  int      streak = 0;
  uint8_t  acc = 0;         // pet::HAT | SHADES | CROWN
  uint32_t seed = 1;        // the city's look
  int      greens = 0;      // green builds in all
  int      towers[MAX_TOWERS] = {0};  // a release tower stands after this many buildings
  int      nTowers = 0;
};

void setProject(Session& s, const char* name);  // copies, cut to 12 characters

// What happened, for the faces' reactions.
enum class Kick : uint8_t {
  NONE, MERGED, CI_RED, CI_GREEN, CLAUDE_START, CLAUDE_DONE, RELEASE, LEVEL_UP, REVIEW,
  // toys (from the motion sensor, not from the agent)
  SHAKE, FLICK, UPSIDE_DOWN, LIFT,
};

// The kicks from `before` to `after` (at most `max`, most important first). Nothing when
// `before` wasn't valid yet (the first message isn't news).
int diff(const Snapshot& before, const Snapshot& after, Kick out[], int max);

// An event's "kind" (events.py) -> its kick, NONE if it has none.
Kick kickForEvent(const char* kind);

}  // namespace world
