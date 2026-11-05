#include "world.h"
#include <string.h>
#include "pet.h"

namespace world {

Ci parseCi(const char* s) {
  if (!s) return Ci::NONE;
  if (!strcmp(s, "green")) return Ci::GREEN;
  if (!strcmp(s, "red")) return Ci::RED;
  return Ci::NONE;
}

void setProject(Session& s, const char* name) {
  strncpy(s.project, name ? name : "", PROJECT_LEN - 1);
  s.project[PROJECT_LEN - 1] = 0;
}

static bool hasSession(const Snapshot& w, uint16_t id) {
  for (int i = 0; i < w.nSessions; i++)
    if (w.sessions[i].id == id) return true;
  return false;
}

int diff(const Snapshot& b, const Snapshot& a, Kick out[], int max) {
  if (!b.valid || !a.valid) return 0;
  int n = 0;
  auto add = [&](Kick k) { if (n < max) out[n++] = k; };
  if (a.nTowers > b.nTowers) add(Kick::RELEASE);
  if (pet::levelFor(a.xp).level > pet::levelFor(b.xp).level) add(Kick::LEVEL_UP);
  if (a.merged > b.merged) add(Kick::MERGED);
  if (a.ci == Ci::RED && b.ci != Ci::RED) add(Kick::CI_RED);
  if (a.ci == Ci::GREEN && b.ci == Ci::RED) add(Kick::CI_GREEN);
  if (a.reviews > b.reviews) add(Kick::REVIEW);
  bool done = false, started = false;
  for (int i = 0; i < b.nSessions; i++) done |= !hasSession(a, b.sessions[i].id);
  for (int i = 0; i < a.nSessions; i++) started |= !hasSession(b, a.sessions[i].id);
  if (done) add(Kick::CLAUDE_DONE);
  if (started) add(Kick::CLAUDE_START);
  return n;
}

Kick kickForEvent(const char* kind) {
  if (!kind) return Kick::NONE;
  struct { const char* k; Kick v; } MAP[] = {
      {"pr_merged", Kick::MERGED},     {"ci_failed", Kick::CI_RED},       {"ci_fixed", Kick::CI_GREEN},
      {"claude_done", Kick::CLAUDE_DONE}, {"release", Kick::RELEASE},     {"review_requested", Kick::REVIEW},
  };
  for (auto& m : MAP)
    if (!strcmp(kind, m.k)) return m.v;
  return Kick::NONE;
}

}  // namespace world
