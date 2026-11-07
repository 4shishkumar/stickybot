// Firmware update over the Bluetooth link (see PROTOCOL.md, "Firmware update"):
// the receiver's state machine, without flash or Bluetooth. Pure C++
// (native-testable); ota.cpp owns the ESP-IDF side (esp_ota_*) behind Sink.
//
// The agent sends `begin` (size, sha256, version) as JSON, then the app image in
// packets [offset u32 LE][bytes] by write without response, at most `WINDOW`
// bytes past the last acknowledged offset. Packets are only taken in order:
// one that starts past the next expected byte means something was dropped, so
// the receiver answers GAP (= "resend from here"); old / duplicate bytes are
// ignored. After a disconnect the session stays in memory and a `begin` with
// the same size + sha256 resumes at the next expected byte. `end` checks the
// size and the SHA-256 of what was written (read back from flash) before the
// image is committed (validated + made the boot partition).
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace otarx {

constexpr uint32_t WINDOW         = 16384;   // bytes in flight past the last ack (the stick's queue holds more)
constexpr uint32_t ACK_EVERY      = 4096;    // ack after this many new bytes...
constexpr uint32_t ACK_IDLE_MS    = 150;     // ... or this long after the last ack when there's news
constexpr uint32_t GAP_RETRY_MS   = 300;     // repeat a GAP if the resend doesn't start
constexpr uint32_t IDLE_TIMEOUT_MS = 180000; // no data / begin for this long: give up (covers a reconnect)
constexpr int      MIN_BATTERY    = 30;      // % — or USB power
constexpr size_t   HEADER         = 4;       // [offset u32] before the data in each packet
constexpr uint16_t CHIP_ESP32S3   = 0x0009;  // esp_image_header_t.chip_id

// Why a `begin` was refused, or a session ended. Names are the wire strings (errorName).
enum class Error : uint8_t {
  NONE,
  BUSY,         // dictation, talk, an approval, pairing or the menu is on
  BATTERY,      // < MIN_BATTERY % and not on USB power
  PENDING,      // the running firmware was just updated and isn't confirmed yet
  UNSUPPORTED,  // compressed stream (not implemented: send raw)
  SIZE,         // 0, bigger than the partition, or data past the end
  SHA,          // the sha256 isn't 64 hex digits, or the written image doesn't match it
  DOWNGRADE,    // older than the running version (needs "force")
  SAME,         // the running version (needs "force")
  IMAGE,        // not an ESP32-S3 app image, or esp_ota_end refused it
  OPEN,         // esp_ota_begin failed
  WRITE,        // esp_ota_write failed
  READ,         // reading back for the hash failed
  INCOMPLETE,   // `end` before every byte arrived (the offset says where to resume)
  TIMEOUT,      // nothing for IDLE_TIMEOUT_MS
  NOT_STARTED,  // `end` / data without a session
};
const char* errorName(Error e);

// -1 / 0 / 1 comparing "1.8.0" with "1.8.1-test" (numbers only; a suffix is ignored).
int compareVersions(const char* a, const char* b);
bool parseSha256(const char* hex, uint8_t out[32]);

struct Begin {
  uint32_t size = 0;
  const char* sha256 = "";   // hex
  const char* version = "";
  bool compressed = false;
  bool force = false;        // allow the same or an older version
};

struct Conditions {
  const char* busy = nullptr;   // why the stick can't update now (nullptr = free)
  int  batteryPct = -1;         // -1 unknown
  bool usbPower = false;
  bool pendingVerify = false;   // running image not confirmed yet (esp_ota_begin would fail)
  uint32_t partitionSize = 0;   // the inactive OTA slot
  const char* running = "";     // FW_VERSION
};

// Refusal for a new session (NONE = may start). Resuming skips the version checks.
Error refuse(const Begin& b, const Conditions& c, bool resuming = false);
// The same for a plain file (a face package, faces/pkg/install.cpp): busy, size (against
// partitionSize, the largest file) and the sha256 only; no battery or version checks.
Error refuseFile(const Begin& b, const Conditions& c);

// The first bytes of an app image: magic 0xE9 and the chip id (needs >= 14 bytes).
bool imageHeaderOk(const uint8_t* p, size_t n, uint16_t chipId = CHIP_ESP32S3);

// The flash side (ota.cpp: esp_ota_begin / write / sha read-back / end + set boot / abort).
struct Sink {
  virtual ~Sink() {}
  virtual bool open(uint32_t size) = 0;
  virtual bool write(const uint8_t* p, size_t n) = 0;
  virtual bool sha256(uint32_t size, uint8_t out[32]) = 0;  // of the first `size` bytes written
  virtual bool commit() = 0;                                // validate + set the boot partition
  virtual void abort() = 0;
  // After open(): bytes already written by an earlier session that this one continues (a
  // file sink keeps a part file across a restart); 0 = start at the beginning.
  virtual uint32_t resumeFrom() { return 0; }
};

struct Reply {
  enum class Kind : uint8_t { NONE, READY, REFUSED, ACK, GAP, DONE, ERROR, CANCELLED };
  Kind     kind = Kind::NONE;
  uint32_t offset = 0;
  Error    error = Error::NONE;
  bool     resumed = false;  // READY: continuing an earlier session
};

class Receiver {
 public:
  enum class State : uint8_t { IDLE, RECEIVING, DONE };

  // What arrives: an app image (firmware: its header is checked, refuse()) or any file
  // (refuseFile(), no header check).
  enum class Kind : uint8_t { APP_IMAGE, FILE };

  explicit Receiver(Sink& sink, Kind kind = Kind::APP_IMAGE) : sink_(sink), kind_(kind) {}

  Reply begin(const Begin& b, const Conditions& c, uint32_t now);
  Reply data(uint32_t offset, const uint8_t* p, size_t n, uint32_t now);
  // A raw packet from the characteristic: [offset u32 LE][bytes].
  Reply packet(const uint8_t* p, size_t n, uint32_t now);
  Reply poll(uint32_t now);  // timed acks, GAP repeats, the idle timeout
  Reply end(uint32_t now);
  Reply cancel();

  State    state() const { return state_; }
  bool     receiving() const { return state_ == State::RECEIVING; }
  uint32_t next() const { return next_; }
  uint32_t size() const { return size_; }
  int      percent() const { return size_ ? (int)((uint64_t)next_ * 100 / size_) : 0; }
  const char* version() const { return version_; }

 private:
  Reply ack(uint32_t now);
  Reply fail(Error e);

  Sink& sink_;
  Kind  kind_;
  State state_ = State::IDLE;
  uint32_t size_ = 0, next_ = 0, lastAcked_ = 0;
  uint32_t lastAckAt_ = 0, lastDataAt_ = 0, gapAt_ = 0;
  bool gap_ = false, dup_ = false, headerChecked_ = false;
  uint8_t sha_[32] = {};
  uint8_t head_[16] = {};
  char version_[24] = {};
};

}  // namespace otarx
