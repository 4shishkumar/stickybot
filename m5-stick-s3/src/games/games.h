// Games on the stick (firmware 2.5.0): the menu's "Games" entry opens a picker; the session
// runs the chosen game (src/logic/games*), draws it (draw.h), plays its sounds, keeps the best
// scores in NVS and tells the Mac agent how each game went.
//
// Buttons, the same in every game:
//   picker      A next game, B play, hold B back to Home
//   title card  A start (tilt games: lay it flat first, a 2 s countdown calibrates "flat"),
//               B / hold B back to the picker
//   playing     A is the game's button; B or hold B pauses (Resume / Restart / Quit: A next,
//               B pick, hold B resumes)
//   game over   A again, B the picker
// An approval or a question from Claude still takes the screen (main.cpp); the game pauses.
// The picker, a pause or a game over left alone for 90 s closes the games.
//
// While a game runs the CPU goes from 80 to 160 MHz (back when the games close), and the
// speaker stays up between effects.
//
// To the agent, at the end of each game (also one quit or restarted with points), when it's
// linked: {"src":"agent","evt":"game","id":"stack","score":12,"best":15,"new_best":false}
// (PROTOCOL.md). `@games` lists the games and the bests (protocol.cpp).
#pragma once
#include <Arduino.h>
#include "../logic/games.h"

namespace games {

struct Input {
  bool     aDown, aUp, aHeld;
  uint32_t aChangedAt;          // millis() of A's last change (Reaction counts from it)
  bool     bClick, bHold, bHeld;  // B released before the hold time / reached it / is down
  float    ax, ay;              // accelerometer, g (the stick's axes)
};

// From the menu. flipped: the portrait way up; side: +1 / -1, the landscape side for the
// landscape games (button A's end).
void open(bool flipped, int side);
bool active();
// One main-loop pass: false once the games have closed (back to Home; the caller restores
// the screen's orientation).
bool loop(const Input& in);
void close();

long   best(Id g);   // 0 = none yet
String json();       // @games: "games":[{"id","title","best","tilt","low"}]

}  // namespace games
