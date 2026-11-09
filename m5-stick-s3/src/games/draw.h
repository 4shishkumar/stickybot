// Drawing the games (firmware 2.5.0) on ui::canvas: the picker, each game's frame, the
// title card, the calibration countdown, pause and game over. Only drawing: the session
// (buttons, sounds, NVS, the agent) is games.cpp, the rules are src/logic/games*. The host
// renderer (tools/face_shots, `make games`) draws the same frames on the Mac.
//
// The caller turns the screen first (ui::useSide(), the game's orientation), then draws a
// frame (a game, plus an overlay for pause / game over) and calls present().
//
// Static backgrounds (skies, the maze's walls) are drawn once into a second sprite (PSRAM)
// and copied under each frame; release() frees it when the games close.
#pragma once
#include <Arduino.h>
#include "../logic/games.h"
#include "../logic/games_one.h"
#include "../logic/games_tilt.h"

namespace games {
namespace draw {

// What every frame needs besides the game: its best, StickyBot's accessories (pet::HAT...).
struct Ctx {
  long    best = 0;
  uint8_t acc = 0;
  uint32_t now = 0;  // ms (animations that run while the game is frozen: pause, game over)
};

void picker(int selected, const long bests[COUNT], uint32_t now);
// The game's title card: how to play, the best, A to start (behind it the game's backdrop).
void ready(Id g, const Ctx& c);
// Tilt games: "lay it flat", the countdown and a bubble level (tx, ty: the raw tilt, g).
void calibrate(Id g, uint32_t msLeft, float tx, float ty, const Ctx& c);

void stack(const Stack& s, const Ctx& c);
void flappy(const Flappy& f, const Ctx& c);
void dino(const Dino& d, const Ctx& c);
void reaction(const Reaction& r, const Ctx& c);
void maze(const Maze& m, const Ctx& c);
void balance(const Balance& b, const Ctx& c);
void catcher(const Catch& k, const Ctx& c);

// Overlays, over the frame just drawn (it's dimmed under them)
constexpr int PAUSE_ITEMS = 3;  // Resume, Restart, Quit
void pause(int selected);
// Game over: the score, the best (or "New best!" with confetti; restart it with
// overConfetti() when it's shown), what A and B do. sinceMs: since it ended.
void over(Id g, long score, long best, bool newBest, bool failed, uint32_t sinceMs);
void overConfetti();

void present();   // push the canvas to the screen
void release();   // free the cached background

// The game's colour (the picker's accent, its title card)
uint16_t accent(Id g);

}  // namespace draw
}  // namespace games
