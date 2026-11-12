// Which sound an event screen plays (PROTOCOL.md "Events"). The Mac sends the event's "sound":
//   ""         the level's default (celebrate: jingle_win, nice: confirmation_002, headsup: question_001)
//   "off"      silent (quiet hours, or the user turned it off for that kind)
//   a name     a built-in clip or tune ("applause") or a moment ("answer")
// A name this firmware doesn't know (a newer Mac asking for a newer clip) falls back to the
// level's default, so the event isn't silent by mistake.
// Pure C++ (no Arduino): also built by the native unit tests.
#pragma once
#include <string.h>

namespace event_sound {

// playFn(name) starts a sound and returns false if there's no such name. Returns the name that
// played (nullptr: none).
template <class Play>
const char* play(const char* sound, const char* levelDefault, Play&& playFn) {
  if (sound && !strcmp(sound, "off")) return nullptr;
  if (sound && *sound && playFn(sound)) return sound;
  return levelDefault && *levelDefault && playFn(levelDefault) ? levelDefault : nullptr;
}

}  // namespace event_sound
