// The IR hardware through the ESP32-S3's RMT peripheral (IDF 5 driver/rmt_tx.h, rmt_rx.h).
//
// TX: one channel at 1 MHz (1 us ticks), the carrier (38 kHz unless the code says otherwise,
// 33 % duty) added by the RMT itself; marks / spaces become RMT symbols (a gap over 32 ms is
// split in two). Made on the first send and kept.
//
// RX: only while learning or listening (no RAM or channel otherwise). A task owns the channel:
// DMA, two 512-symbol buffers taking turns (re-armed as soon as one fills, so the next frame of
// an air conditioner's message isn't missed), a 1 us glitch filter and a 12 ms idle timeout per
// frame. Frames closer than JOIN_MS are joined into one capture with their measured gap; a
// capture is handed over JOIN_MS after its last frame.
#pragma once
#include <Arduino.h>

namespace ir {
namespace rmt {

constexpr uint32_t JOIN_MS = 150;

// --- TX ---
bool   txReady();                                   // the channel exists (or was made now)
// Starts sending one frame (marks / spaces in us, a mark first) with this carrier; false if busy
// or it doesn't fit. Returns at once: the RMT sends it in the background.
bool   send(const uint16_t* t, size_t n, uint32_t carrierHz);
bool   txBusy();
const char* txState();                              // "off" (not made yet) | "ok" | "error"

// --- RX ---
bool   rxStart();                                   // false: the channel couldn't be made
void   rxStop();
bool   rxActive();
// A finished capture (frames joined): copies up to max durations into out, returns how many
// (0 = none waiting). The last frame ended JOIN_MS ago.
size_t takeCapture(uint16_t* out, size_t max);
uint32_t rxFrames();                                // frames seen since rxStart (diag)
const char* rxState();                              // "off" | "on" | "error"

// --- Diag ---
int    pinLevel(int gpio);                          // reads a pin as an input (no pull), -1 if busy

}  // namespace rmt
}  // namespace ir
