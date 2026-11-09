// StickyBot's pet progress and moods for the faces (StickyWorld, Aquarium). Pure C++
// (native-testable). The Mac agent keeps the real progress (mac-agent/stick_agent/pet.py,
// the same rules) and sends it to the stick; the stick only shows it.
//
//   XP:      merged PR +10, CI green +3, a Claude task finished +2, a release +25
//   Level:   1 at 0 XP; level n -> n+1 needs 10 + 10 * n XP (20, 30, 40...)
//   Streak:  days in a row with at least one merged PR or finished Claude task
//   Unlocks: hat at a 5-day streak (best ever), sunglasses at 10 green builds, crown at 100 PRs
//   Games:   (agent only, 2.5.0) +1 a game with points, +3 a new best, at most 10 XP a day
#pragma once
#include <stdint.h>

namespace pet {

constexpr int XP_MERGE = 10, XP_GREEN = 3, XP_CLAUDE = 2, XP_RELEASE = 25;

struct Level {
  int level;  // 1, 2, 3...
  int into;   // XP earned inside this level
  int need;   // XP this level needs in all (into < need)
};
int   xpToNext(int level);  // 10 + 10 * level
Level levelFor(long xp);

// Accessories, as bits (the agent sends them in "acc"; accessories() is the same rule)
enum : uint8_t { HAT = 1, SHADES = 2, CROWN = 4 };
uint8_t accessories(int bestStreak, int greenBuilds, int mergedTotal);

// Days are local day numbers (days since 1970 in local time).
struct Streak {
  int     days = 0;      // the current run
  int     best = 0;
  int32_t lastDay = -1;  // the last active day, -1 = never
};
void markActive(Streak& s, int32_t today);          // something happened today
int  shownStreak(const Streak& s, int32_t today);   // 0 once a whole day was missed

// What the bot is doing. The first group are the steady moods from the state of things;
// the rest are short reactions (an event or a toy) that play once on top.
enum class Mood : uint8_t {
  IDLE, HAPPY, WORKING, ALERT, SAD, SLEEPY,             // steady
  EATING, CELEBRATING, HOP, DIZZY, FLIP, PANIC, LOOK,   // reactions
};

struct MoodInputs {
  bool urgent;        // something waits for you (an approval, a question, a review request)
  bool ciRed;         // CI failed on one of your PRs
  bool ciGreen;
  int  claudeRunning; // Claude Code sessions working right now
  bool night;
};
// Steady mood, most important first: alert, sad, working, sleepy (night), happy, idle.
Mood steadyMood(const MoodInputs& in);

// How long each reaction plays (ms); 0 for the steady moods.
uint32_t reactionMs(Mood m);

}  // namespace pet
