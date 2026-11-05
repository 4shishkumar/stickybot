#include "ota_rx.h"
#include <string.h>

namespace otarx {

const char* errorName(Error e) {
  switch (e) {
    case Error::NONE:        return "none";
    case Error::BUSY:        return "busy";
    case Error::BATTERY:     return "battery";
    case Error::PENDING:     return "pending";
    case Error::UNSUPPORTED: return "unsupported";
    case Error::SIZE:        return "size";
    case Error::SHA:         return "sha";
    case Error::DOWNGRADE:   return "downgrade";
    case Error::SAME:        return "same";
    case Error::IMAGE:       return "image";
    case Error::OPEN:        return "open";
    case Error::WRITE:       return "write";
    case Error::READ:        return "read";
    case Error::INCOMPLETE:  return "incomplete";
    case Error::TIMEOUT:     return "timeout";
    case Error::NOT_STARTED: return "not_started";
  }
  return "?";
}

static void versionParts(const char* v, long out[3]) {
  out[0] = out[1] = out[2] = 0;
  int i = 0;
  const char* p = v ? v : "";
  while (*p && i < 3) {
    if (*p >= '0' && *p <= '9') {
      long n = 0;
      while (*p >= '0' && *p <= '9') n = n * 10 + (*p++ - '0');
      out[i++] = n;
      if (*p != '.') break;  // "1.8.1-test": stop at the suffix
    } else {
      p++;
    }
  }
}

int compareVersions(const char* a, const char* b) {
  long x[3], y[3];
  versionParts(a, x);
  versionParts(b, y);
  for (int i = 0; i < 3; i++)
    if (x[i] != y[i]) return x[i] < y[i] ? -1 : 1;
  return 0;
}

static int hexval(char c) {
  if (c >= '0' && c <= '9') return c - '0';
  if (c >= 'a' && c <= 'f') return c - 'a' + 10;
  if (c >= 'A' && c <= 'F') return c - 'A' + 10;
  return -1;
}

bool parseSha256(const char* hex, uint8_t out[32]) {
  if (!hex || strlen(hex) != 64) return false;
  for (int i = 0; i < 32; i++) {
    int hi = hexval(hex[2 * i]), lo = hexval(hex[2 * i + 1]);
    if (hi < 0 || lo < 0) return false;
    out[i] = (uint8_t)(hi << 4 | lo);
  }
  return true;
}

Error refuse(const Begin& b, const Conditions& c, bool resuming) {
  if (c.busy) return Error::BUSY;
  if (c.pendingVerify) return Error::PENDING;
  if (!c.usbPower && c.batteryPct < MIN_BATTERY) return Error::BATTERY;  // unknown (-1) counts as low
  if (b.compressed) return Error::UNSUPPORTED;
  if (b.size == 0 || b.size > c.partitionSize) return Error::SIZE;
  uint8_t sha[32];
  if (!parseSha256(b.sha256, sha)) return Error::SHA;
  if (!resuming && !b.force) {
    int cmp = compareVersions(b.version, c.running);
    if (cmp < 0) return Error::DOWNGRADE;
    if (cmp == 0) return Error::SAME;
  }
  return Error::NONE;
}

bool imageHeaderOk(const uint8_t* p, size_t n, uint16_t chipId) {
  // esp_image_header_t: magic (0xE9) at 0, chip_id (u16 LE) at 12
  return n >= 14 && p[0] == 0xE9 && (uint16_t)(p[12] | p[13] << 8) == chipId;
}

Reply Receiver::fail(Error e) {
  if (state_ == State::RECEIVING) sink_.abort();
  state_ = State::IDLE;
  Reply r;
  r.kind = Reply::Kind::ERROR;
  r.error = e;
  r.offset = next_;
  return r;
}

Reply Receiver::ack(uint32_t now) {
  lastAcked_ = next_;
  lastAckAt_ = now;
  dup_ = false;
  Reply r;
  r.kind = Reply::Kind::ACK;
  r.offset = next_;
  return r;
}

Reply Receiver::begin(const Begin& b, const Conditions& c, uint32_t now) {
  uint8_t sha[32];
  bool same = state_ == State::RECEIVING && b.size == size_ && parseSha256(b.sha256, sha) &&
              memcmp(sha, sha_, 32) == 0;
  Reply r;
  Error e = refuse(b, c, same);
  if (e != Error::NONE) {  // an earlier session (if any) stays, so a later begin can still resume it
    r.kind = Reply::Kind::REFUSED;
    r.error = e;
    return r;
  }
  if (same) {  // resume after a reconnect: everything before next_ is written
    gap_ = dup_ = false;
    lastAcked_ = next_;
    lastAckAt_ = lastDataAt_ = now;
    r.kind = Reply::Kind::READY;
    r.offset = next_;
    r.resumed = true;
    return r;
  }
  if (state_ == State::RECEIVING) sink_.abort();  // a different image: start over
  state_ = State::IDLE;
  if (!sink_.open(b.size)) {
    r.kind = Reply::Kind::ERROR;
    r.error = Error::OPEN;
    return r;
  }
  state_ = State::RECEIVING;
  size_ = b.size;
  next_ = lastAcked_ = 0;
  lastAckAt_ = lastDataAt_ = now;
  gap_ = dup_ = headerChecked_ = false;
  parseSha256(b.sha256, sha_);
  strncpy(version_, b.version ? b.version : "", sizeof(version_) - 1);
  version_[sizeof(version_) - 1] = 0;
  r.kind = Reply::Kind::READY;
  r.offset = 0;
  return r;
}

Reply Receiver::data(uint32_t offset, const uint8_t* p, size_t n, uint32_t now) {
  Reply none;
  if (state_ != State::RECEIVING || !n) return none;
  lastDataAt_ = now;
  if (offset > next_) {  // something before this was dropped: ask for a resend (not on every packet)
    if (!gap_ || now - gapAt_ >= GAP_RETRY_MS) {
      gap_ = true;
      gapAt_ = now;
      Reply r;
      r.kind = Reply::Kind::GAP;
      r.offset = next_;
      return r;
    }
    return none;
  }
  uint64_t end = (uint64_t)offset + n;
  if (end <= next_) {  // already have it (a resend after a lost ack): re-ack soon
    dup_ = true;
    return none;
  }
  if (end > size_) return fail(Error::SIZE);
  size_t skip = next_ - offset;
  p += skip;
  n -= skip;
  if (next_ < sizeof(head_)) {  // keep the image header to check it
    size_t k = sizeof(head_) - next_ < n ? sizeof(head_) - next_ : n;
    memcpy(head_ + next_, p, k);
  }
  if (!headerChecked_ && next_ + n >= sizeof(head_)) {
    headerChecked_ = true;
    if (!imageHeaderOk(head_, sizeof(head_))) return fail(Error::IMAGE);
  }
  if (!sink_.write(p, n)) return fail(Error::WRITE);
  next_ += n;
  gap_ = false;
  if (next_ - lastAcked_ >= ACK_EVERY || next_ == size_) return ack(now);
  return none;
}

Reply Receiver::packet(const uint8_t* p, size_t n, uint32_t now) {
  if (n <= HEADER) return Reply{};
  uint32_t offset = (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
  return data(offset, p + HEADER, n - HEADER, now);
}

Reply Receiver::poll(uint32_t now) {
  Reply none;
  if (state_ != State::RECEIVING) return none;
  if (now - lastDataAt_ >= IDLE_TIMEOUT_MS) return fail(Error::TIMEOUT);
  if (gap_ && now - gapAt_ >= GAP_RETRY_MS) {
    gapAt_ = now;
    Reply r;
    r.kind = Reply::Kind::GAP;
    r.offset = next_;
    return r;
  }
  if ((next_ != lastAcked_ || dup_) && now - lastAckAt_ >= ACK_IDLE_MS) return ack(now);
  return none;
}

Reply Receiver::end(uint32_t now) {
  if (state_ != State::RECEIVING) {
    Reply r;
    r.offset = next_;
    if (state_ == State::DONE) r.kind = Reply::Kind::DONE;  // a repeated end: same answer
    else { r.kind = Reply::Kind::ERROR; r.error = Error::NOT_STARTED; }
    return r;
  }
  lastDataAt_ = now;
  if (next_ != size_) {  // keep the session: the agent resumes from offset
    Reply r;
    r.kind = Reply::Kind::ERROR;
    r.error = Error::INCOMPLETE;
    r.offset = next_;
    return r;
  }
  uint8_t got[32];
  if (!sink_.sha256(size_, got)) return fail(Error::READ);
  if (memcmp(got, sha_, 32) != 0) return fail(Error::SHA);
  if (!sink_.commit()) {
    state_ = State::IDLE;  // commit failing has already ended the handle (esp_ota_end)
    Reply r;
    r.kind = Reply::Kind::ERROR;
    r.error = Error::IMAGE;
    r.offset = next_;
    return r;
  }
  state_ = State::DONE;
  Reply r;
  r.kind = Reply::Kind::DONE;
  r.offset = next_;
  return r;
}

Reply Receiver::cancel() {
  Reply r;
  if (state_ != State::RECEIVING) return r;
  sink_.abort();
  state_ = State::IDLE;
  r.kind = Reply::Kind::CANCELLED;
  r.offset = next_;
  return r;
}

}  // namespace otarx
