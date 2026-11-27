// One physical press belongs to the screen where it began.
#pragma once
#include <cstdint>
#include <cstddef>
#include <cstring>
#include "actions.h"
#include "hosts.h"

namespace events {

enum class InputResult { NONE, DETAIL, OPEN, READ, CLOSE };

struct Detail {
  bool shown = false, read = false, opened = false, armed = false;
  InputResult input(bool down, bool up, bool held, bool close, bool canOpen, bool canRead) {
    if (close) return InputResult::CLOSE;
    if (!shown) {
      if (!down) return InputResult::NONE;
      shown = true;
      return InputResult::DETAIL;
    }
    if (down) armed = true;
    if (!armed) return InputResult::NONE;
    const auto& screen = actions::screen(actions::Id::EVENT_DETAIL);
    const uint8_t flags = (canOpen && !opened ? actions::EVENT_URL : 0) | (canRead && !read ? actions::SPEECH : 0);
    if (held) {
      armed = false;  // a long press can never open a link on release
      if (actions::lookup(screen, actions::In::A_HOLD, flags) == actions::Do::LISTEN) {
        read = true;
        return InputResult::READ;
      }
    } else if (up) {
      armed = false;
      if (actions::lookup(screen, actions::In::A_CLICK, flags) == actions::Do::OPEN_EVENT) {
        opened = true;
        return InputResult::OPEN;
      }
    }
    return InputResult::NONE;
  }
};

inline bool sameSender(const hosts::Registry& registry, bool shared, hosts::Lease owner,
                       uint32_t epoch, uint16_t handle, uint32_t liveEpoch) {
  return owner.handle != hosts::NONE && owner.handle == handle && epoch == liveEpoch &&
         (!shared || registry.owns(owner, handle));
}

// Conservative display affordance; the Mac performs full validation against its delivery record.
inline bool webLink(const char* url) {
  const char* host = !strncmp(url, "https://", 8) ? url + 8 : !strncmp(url, "http://", 7) ? url + 7 : nullptr;
  if (!host || !*host || strchr("/?#:", *host)) return false;
  for (const char* p = host; *p; ++p) if ((uint8_t)*p <= 0x20 || *p == 0x7f || *p == '\\') return false;
  return true;
}

// Count UTF-8 scalars without splitting the final one. A malformed field is unusable.
inline size_t prefixBytes(const char* s, size_t size, size_t maxChars) {
  size_t bytes = 0;
  for (size_t i = 0; bytes < size && i < maxChars; ++i) {
    const uint8_t c = s[bytes];
    size_t len = c < 0x80 ? 1 : c >= 0xc2 && c < 0xe0 ? 2 : c >= 0xe0 && c < 0xf0 ? 3 :
                 c >= 0xf0 && c <= 0xf4 ? 4 : 0;
    if (!c || !len || bytes + len > size) return 0;
    for (size_t j = 1; j < len; ++j) if ((s[bytes+j] & 0xc0) != 0x80) return 0;
    if (len >= 3) {
      const uint8_t second = s[bytes + 1];
      if ((c == 0xe0 && second < 0xa0) || (c == 0xed && second >= 0xa0) ||
          (c == 0xf0 && second < 0x90) || (c == 0xf4 && second >= 0x90)) return 0;
    }
    bytes += len;
  }
  return bytes;
}

}  // namespace events
