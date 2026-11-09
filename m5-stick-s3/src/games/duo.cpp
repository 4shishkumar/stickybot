// Two-stick multiplayer: the hook only (duo.h). Every call is a no-op until it's built.
#include "duo.h"

namespace games {
namespace duo {

bool available() { return false; }
bool invite(uint8_t, uint32_t) { return false; }
bool send(const Packet&) { return false; }
bool poll(Packet&) { return false; }
void end() {}

}  // namespace duo
}  // namespace games
