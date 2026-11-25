#include "button_map.h"
#include "hosts.h"

namespace buttons {
using actions::Do;
using actions::In;

const char* key(Slot s) {
  static const char* const keys[] = {"b2", "b3", "ab", "bh"};
  return keys[static_cast<int>(s)];
}

bool slotFor(In in, Slot& out) {
  switch (in) {
    case In::B_DOUBLE: out = Slot::DOUBLE; return true;
    case In::B_TRIPLE: out = Slot::TRIPLE; return true;
    case In::CHORD: out = Slot::CHORD; return true;
    case In::B_HOLD: out = Slot::HOLD; return true;
    default: return false;
  }
}

const Table& defaults() {
  static const Table t{{{Do::UNLOCK, "unlock", "unlock"}, {Do::LAUNCHER, "launcher", "apps"},
                        {Do::LAST_REPLY, "reply", "reply"}, {Do::TALK, "talk", "talk"}}};
  return t;
}

const char* talkHint(const Table& map) {
  if (map[Slot::HOLD].action == Do::TALK) return "Hold B to ask";
  if (map[Slot::DOUBLE].action == Do::TALK) return "Double B to ask";
  if (map[Slot::TRIPLE].action == Do::TALK) return "Triple B to ask";
  if (map[Slot::CHORD].action == Do::TALK) return "B+A to ask";
  return "Agent ready";
}

// UTF-8 scalar values only; reserve path separators and reject control characters.
static bool validName(const std::string& name) {
  if (!hosts::name(name, NAME_BYTES) || name.find_first_of("/\\") != std::string::npos) return false;
  // Match dashboard trim for non-ASCII spaces as well (C0/C1 whitespace already failed above).
  static const char* const spaces[] = {" ", "\u00a0", "\u1680", "\u2000", "\u2001", "\u2002",
      "\u2003", "\u2004", "\u2005", "\u2006", "\u2007", "\u2008", "\u2009", "\u200a",
      "\u2028", "\u2029", "\u202f", "\u205f", "\u3000", "\ufeff"};
  for (const char* space : spaces) {
    size_t n = std::char_traits<char>::length(space);
    if (name.size() >= n && (!name.compare(0, n, space) || !name.compare(name.size() - n, n, space))) return false;
  }
  return true;
}

static bool parseBinding(Slot s, const std::string& v, Binding& out) {
  if (v == "talk") out = {Do::TALK, v, "talk"};
  else if (s == Slot::HOLD) {
    if (v != "dictation") return false;
    out = {Do::COMBO, v, "dictate"};
  } else if (v == "none") out = {Do::NONE, v, ""};
  else if (v == "unlock") out = defaults()[Slot::DOUBLE];
  else if (v == "launcher") out = defaults()[Slot::TRIPLE];
  else if (v == "reply") out = defaults()[Slot::CHORD];
  else if (v.compare(0, 5, "tile:") == 0) {
    launcher::Tile t;
    auto id = v.substr(5);
    if (!launcher::fromName(id.data(), id.size(), t) || id != launcher::info(t).id) return false;
    out = {Do::OPEN_TILE, v, launcher::info(t).label, t};
  } else {
    bool app = v.compare(0, 4, "app:") == 0;
    if (!app && v.compare(0, 6, "quick:") != 0) return false;
    auto name = v.substr(app ? 4 : 6);
    if (!validName(name)) return false;
    out = {app ? Do::MAC_APP : Do::QUICK, v, name};
  }
  return true;
}

bool parse(const std::string (&values)[COUNT], Table& out) {
  Table next;
  for (int i = 0; i < COUNT; ++i)
    if (!parseBinding(static_cast<Slot>(i), values[i], next.rows[i])) return false;
  out = std::move(next);
  return true;
}

std::string pack(const Table& t) {
  std::string out;
  for (int i = 0; i < COUNT; ++i) {
    if (i) out += '\n';
    out += t.rows[i].wire;
  }
  return out;
}

bool unpack(const std::string& record, Table& out) {
  if (record.size() > MAX_RECORD_BYTES) return false;
  std::string rows[COUNT];
  size_t start = 0;
  for (int i = 0; i < COUNT; ++i) {
    size_t end = record.find('\n', start);
    if ((end == std::string::npos) != (i == COUNT - 1)) return false;
    rows[i] = record.substr(start, end - start);
    if (end != std::string::npos) start = end + 1;
  }
  return parse(rows, out);
}

ChordStep Chord::update(bool eligible, bool aDown, bool aHeld, bool bHeld, bool bHoldReached) {
  if (!eligible) pending = false;
  else if (aDown && bHeld && !bHoldReached) pending = true;
  if (!pending) return ChordStep::PASS;
  if (aHeld || bHeld) return ChordStep::CONSUME;
  pending = false;
  return ChordStep::FIRE;
}

bool finishTalk(bool latched, bool bDown, bool bHeld, bool linked, uint32_t elapsed) {
  return latched ? bDown || !linked || elapsed >= TAP_TALK_LIMIT_MS : !bHeld;
}

}  // namespace buttons
