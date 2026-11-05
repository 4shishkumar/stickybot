#include "pet.h"

namespace pet {

int xpToNext(int level) { return 10 + 10 * level; }

Level levelFor(long xp) {
  if (xp < 0) xp = 0;
  int level = 1;
  while (level < 999 && xp >= xpToNext(level)) {
    xp -= xpToNext(level);
    level++;
  }
  return {level, (int)xp, xpToNext(level)};
}

uint8_t accessories(int bestStreak, int greenBuilds, int mergedTotal) {
  uint8_t a = 0;
  if (bestStreak >= 5) a |= HAT;
  if (greenBuilds >= 10) a |= SHADES;
  if (mergedTotal >= 100) a |= CROWN;
  return a;
}

void markActive(Streak& s, int32_t today) {
  if (s.lastDay == today) return;
  s.days = (s.lastDay >= 0 && today - s.lastDay == 1) ? s.days + 1 : 1;
  s.lastDay = today;
  if (s.days > s.best) s.best = s.days;
}

int shownStreak(const Streak& s, int32_t today) {
  if (s.lastDay < 0 || today - s.lastDay > 1) return 0;  // today or yesterday keeps it alive
  return s.days;
}

Mood steadyMood(const MoodInputs& in) {
  if (in.urgent) return Mood::ALERT;
  if (in.ciRed) return Mood::SAD;
  if (in.claudeRunning > 0) return Mood::WORKING;
  if (in.night) return Mood::SLEEPY;
  if (in.ciGreen) return Mood::HAPPY;
  return Mood::IDLE;
}

uint32_t reactionMs(Mood m) {
  switch (m) {
    case Mood::EATING:      return 3000;
    case Mood::CELEBRATING: return 4500;
    case Mood::HOP:         return 1400;
    case Mood::DIZZY:       return 2600;
    case Mood::FLIP:        return 1100;
    case Mood::PANIC:       return 2200;
    case Mood::LOOK:        return 3000;
    default:                return 0;
  }
}

}  // namespace pet
