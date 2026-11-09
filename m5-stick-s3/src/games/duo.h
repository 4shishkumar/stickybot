// Two-stick multiplayer over ESP-NOW: a hook, NOT implemented yet (available() is false and
// nothing is sent). The plan, so the rest of the games already fit it:
//
//   - Pairing: both sticks open the same game's title card; one broadcasts an INVITE with the
//     game id and a seed (ESP-NOW on a fixed channel, Wi-Fi otherwise off), the other answers
//     JOIN. The link must not disturb Bluetooth (ESP-NOW shares the radio; test
//     key presses and the wireless mic while a duel runs).
//   - Fair games: the rules are deterministic (src/logic/games*: the same seed and inputs give
//     the same game), so both sticks play the SAME maze / pipes / bugs from the shared seed and
//     only send their progress: SCORE packets (~5 a second) and an OVER with the final score.
//     Each stick draws the other's score small at the top; the higher one wins (Reaction: the
//     lower average; both see the same green at the same time from a START with a delay).
//   - Ghost: later, the other stick's position (Flappy height, maze ball) as a faint ghost.
//   - Nothing about it reaches the Mac; the "game" event to the agent stays per stick.
#pragma once
#include <stdint.h>

namespace games {
namespace duo {

enum class Kind : uint8_t { INVITE, JOIN, START, SCORE, OVER, LEAVE };

struct Packet {          // what would go over ESP-NOW (<= 250 bytes; this is 16)
  uint8_t  version = 1;
  Kind     kind = Kind::INVITE;
  uint8_t  game = 0;     // games::Id
  uint8_t  flags = 0;
  uint32_t seed = 0;     // the shared game
  int32_t  score = 0;
  uint32_t ms = 0;       // the sender's game time
};

bool available();                // false: not implemented
bool invite(uint8_t game, uint32_t seed);
bool send(const Packet& p);
bool poll(Packet& out);          // a packet from the other stick
void end();

}  // namespace duo
}  // namespace games
