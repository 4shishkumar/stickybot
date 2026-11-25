// The four global inputs use one table for execution, display, NVS and the wire.
#pragma once
#include "actions.h"
#include "launcher.h"

namespace buttons {

enum class Slot : uint8_t { DOUBLE, TRIPLE, CHORD, HOLD, COUNT };
constexpr int COUNT = static_cast<int>(Slot::COUNT);
constexpr size_t NAME_BYTES = 32;
constexpr size_t MAX_BINDING_BYTES = 6 + NAME_BYTES;  // "quick:" is the longest named prefix
constexpr size_t MAX_RECORD_BYTES = COUNT * MAX_BINDING_BYTES + COUNT - 1;
const char* key(Slot slot);
bool slotFor(actions::In input, Slot& out);

struct Binding {
  actions::Do action;
  std::string wire;
  std::string label;
  launcher::Tile tile = launcher::Tile::COUNT;
};

struct Table {
  Binding rows[COUNT];
  const Binding& operator[](Slot slot) const { return rows[static_cast<int>(slot)]; }
};

const Table& defaults();
const char* talkHint(const Table& map);
// All four required. The previous table survives a failed parse.
bool parse(const std::string (&values)[COUNT], Table& out);
std::string pack(const Table& t);
bool unpack(const std::string& record, Table& out);

enum class ChordStep { PASS, CONSUME, FIRE };
// The gesture owns both releases; a modal transition cancels it.
struct Chord {
  bool pending = false;
  ChordStep update(bool eligible, bool aDown, bool aHeld, bool bHeld, bool bHoldReached);
};

constexpr uint32_t TAP_TALK_LIMIT_MS = 30000;
bool finishTalk(bool latched, bool bDown, bool bHeld, bool linked, uint32_t elapsed);

}  // namespace buttons
