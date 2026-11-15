#include "hosts.h"

namespace hosts {
bool token(const std::string& s, size_t max) {
  if (s.empty() || s.size() > max) return false;
  for (unsigned char c : s) if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
      (c >= '0' && c <= '9') || c == '-' || c == '_')) return false;
  return true;
}
bool name(const std::string& s, size_t maxBytes) {
  if (s.empty() || s.size() > maxBytes) return false;
  size_t count = 0;
  for (size_t i = 0; i < s.size();) {
    unsigned char c = s[i++]; uint32_t cp; size_t extra;
    if (c < 0x80) { cp = c; extra = 0; }
    else if (c >= 0xc2 && c <= 0xdf) { cp = c & 0x1f; extra = 1; }
    else if (c >= 0xe0 && c <= 0xef) { cp = c & 0x0f; extra = 2; }
    else if (c >= 0xf0 && c <= 0xf4) { cp = c & 7; extra = 3; }
    else return false;
    if (i + extra > s.size()) return false;
    for (size_t n = 0; n < extra; ++n) {
      unsigned char next = s[i++]; if ((next & 0xc0) != 0x80) return false;
      cp = (cp << 6) | (next & 0x3f);
    }
    if ((extra == 1 && cp < 0x80) || (extra == 2 && cp < 0x800) ||
        (extra == 3 && cp < 0x10000) || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff) ||
        cp < 32 || (cp >= 0x7f && cp <= 0x9f) || ++count > 80) return false;
  }
  return true;
}
bool label(const std::string& s) {
  if (!name(s, 12)) return false;
  for (unsigned char c : s) if (c > 126) return false;
  return true;
}
int Registry::byHandle(uint16_t h) const {
  if (h == NONE) return -1;
  for (size_t i = 0; i < CAPACITY; ++i) if (peers[i].handle == h) return (int)i;
  return -1;
}
int Registry::byId(const std::string& id) const {
  if (id.empty()) return -1;
  for (size_t i = 0; i < CAPACITY; ++i) if (peers[i].id == id) return (int)i;
  return -1;
}
int Registry::hello(uint16_t h, const std::string& address, bool encrypted,
                    const std::string& id, const std::string& full, const std::string& shortName,
                    const std::string& session, uint32_t now) {
  if (!encrypted || h == NONE || address.empty() || !token(id) || !token(session) ||
      !name(full, 320) || !label(shortName)) return -1;
  int current = byHandle(h);
  if (current >= 0 && peers[current].address != address) return -1;
  int at = -1;
  for (size_t i = 0; i < CAPACITY; ++i) {
    if (peers[i].address == address) {
      if (peers[i].id != id) return -1;
      at = (int)i;
    } else if (peers[i].id == id) return -1;
  }
  if (at < 0) for (size_t i = 0; i < CAPACITY; ++i) if (peers[i].id.empty()) { at = (int)i; break; }
  if (at < 0) return -1;
  Peer& p = peers[at];
  if (p.handle != h || p.session != session) p.generation = ++nextGeneration;
  p.address = address; p.id = id; p.name = full; p.label = shortName;
  p.handle = h; p.session = session; p.seen = now; p.present = true;
  return at;
}
void Registry::reconnect(uint16_t h, const std::string& address) {
  for (auto& p : peers) if (!p.id.empty() && p.address == address && p.handle != h) {
    p.handle = h; p.session.clear(); p.present = false; p.generation = ++nextGeneration;
  }
}
void Registry::disconnect(uint16_t h) {
  int i = byHandle(h);
  if (i < 0) return;
  auto& p = peers[i];
  p.handle = NONE; p.session.clear(); p.present = false; p.generation = ++nextGeneration;
}
void Registry::retire(uint16_t h) {
  int i = byHandle(h);
  if (i < 0) return;
  auto& p = peers[i];
  p.session.clear(); p.present = false; p.generation = ++nextGeneration;
}
bool Registry::valid(uint16_t h, const std::string& session) const {
  int i = byHandle(h);
  return i >= 0 && !session.empty() && peers[i].session == session;
}
Lease Registry::lease(uint16_t h) const {
  int i = byHandle(h);
  return i < 0 ? Lease{} : Lease{h, peers[i].generation};
}
bool Registry::owns(Lease l, uint16_t h) const {
  int i = byHandle(h);
  return i >= 0 && l.handle == h && l.generation == peers[i].generation && !peers[i].session.empty();
}
bool Registry::online(int i, uint32_t now) const {
  return i >= 0 && i < (int)CAPACITY && peers[i].handle != NONE && peers[i].present &&
      uint32_t(now - peers[i].seen) < PRESENCE_MS;
}
uint16_t Registry::selected() const {
  int i = byId(active);
  return i < 0 ? NONE : peers[i].handle;
}
bool Registry::select(const std::string& id, bool busy) {
  int i = byId(id);
  if (busy || i < 0 || peers[i].handle == NONE) return false;
  active = id;
  return true;
}
std::string Registry::display(int i) const {
  if (i < 0 || i >= (int)CAPACITY || peers[i].id.empty()) return "No Mac";
  const Peer& p = peers[i];
  for (size_t n = 0; n < CAPACITY; ++n) if ((int)n != i && peers[n].label == p.label && !peers[n].id.empty())
    return p.label.substr(0, 5) + "-" + p.id.substr(p.id.size() > 4 ? p.id.size() - 4 : 0) + "-" + std::to_string(i + 1);
  return p.label;
}
bool Transfer::claim(const Registry& peers, uint16_t h, bool begin, bool receiving) {
  int i = peers.byHandle(h);
  if (i < 0 || peers.peers[i].session.empty()) return false;
  if (receiving && (host != peers.peers[i].id || (!peers.owns(owner,h) && !begin))) return false;
  host = peers.peers[i].id;
  owner = peers.lease(h);
  return true;
}
bool Lines::push(char c, std::string& complete) {
  if (c == '\n') {
    bool ready = !dropping && !partial.empty();
    if (ready) complete.swap(partial);
    partial.clear(); dropping = false;
    return ready;
  }
  if (!dropping) {
    if (partial.size() < 4096) partial += c;
    else { partial.clear(); dropping = true; }
  }
  return false;
}
}  // namespace hosts
