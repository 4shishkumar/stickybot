// Bounded host identity and focus rules, independent of Bluetooth and Arduino.
#pragma once
#include <array>
#include <cstdint>
#include <string>

namespace hosts {
constexpr size_t CAPACITY = 3;
constexpr uint16_t NONE = 0xffff;
constexpr uint32_t PRESENCE_MS = 25000;
struct Peer {
  std::string address, id, name, label, session;
  uint16_t handle = NONE;
  uint32_t generation = 0, seen = 0;
  bool present = false;
};
struct Lease {
  uint16_t handle = NONE;
  uint32_t generation = 0;
};
bool token(const std::string& s, size_t max = 64);
bool name(const std::string& s, size_t maxBytes);
bool label(const std::string& s);
class Registry {
 public:
  std::array<Peer, CAPACITY> peers{};
  std::string active;
  // An existing bond owns its host ID; neither another bond nor another ID can replace it.
  int hello(uint16_t handle, const std::string& address, bool encrypted,
            const std::string& id, const std::string& full, const std::string& shortName,
            const std::string& session, uint32_t now);
  int byHandle(uint16_t h) const;
  int byId(const std::string& id) const;
  void reconnect(uint16_t handle, const std::string& address);
  void disconnect(uint16_t handle);
  void retire(uint16_t handle);
  int forget(const std::string& id);
  bool valid(uint16_t h, const std::string& session) const;
  Lease lease(uint16_t h) const;
  bool owns(Lease lease, uint16_t h) const;
  bool online(int index, uint32_t now) const;
  uint16_t selected() const;
  bool select(const std::string& id, bool busy);
  std::string display(int index) const;
 private:
  uint32_t nextGeneration = 0;
};
// A resumable transfer retains the bonded host identity across transport sessions.
// Only that host's new begin may claim a fresh lease; old binary packets stay rejected.
struct Transfer {
  std::string host;
  Lease owner;
  bool claim(const Registry& peers, uint16_t handle, bool begin, bool receiving);
  void revoke() { owner = {}; }
};
// Each connection has its own parser; an overlong line is dropped in full, never truncated
// into a different valid command. The caller bounds the transport queue separately.
struct Lines {
  std::string partial;
  bool dropping = false;
  bool push(char c, std::string& complete);
};
}  // namespace hosts
