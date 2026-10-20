#include "mic.h"
#include <M5Unified.h>
#include <USB.h>
#include <USBAudioCard.h>
#include "netlog.h"
#include "buddy.h"
#include "logic/adpcm.h"

namespace mic {

constexpr uint32_t RATE  = 16000;        // speech: 16 kHz mono 16-bit
constexpr size_t   CHUNK = RATE / 100;   // 10 ms per recording chunk
constexpr uint8_t  GAIN  = 12;           // M5 Mic magnification (16 clipped loud speech)
constexpr int      SETTLE_CHUNKS = 10;   // first 100 ms after power-up: a click, sent as silence

// Agent audio frames: [seq u16][predictor i16][step index u8][4-bit samples...],
// little-endian, low nibble first. Each frame carries the encoder state at its
// start, so the Mac can decode it even if an earlier frame was lost.
constexpr size_t   HEADER       = 5;
constexpr size_t   MAX_SAMPLES  = 320;   // 20 ms
constexpr size_t   RING         = 48;    // ~1 s of frames: pre-roll + slack
constexpr int      SEND_PER_CHUNK = 2;   // drain up to 2 frames per 10 ms (catch up 2x; 3x lost frames)
constexpr uint32_t END_TIMEOUT_MS = 1500;

// A global so it registers its USB interface before the USB stack starts
// (the core starts USB before setup(), like the serial port).
static USBAudioCard uac(RATE, UAC_BPS_16, UAC_SPK_NONE, UAC_MIC_MONO);

struct Frame { uint8_t len; uint8_t data[HEADER + MAX_SAMPLES / 2]; };

// Shared between loop() and the task (all plain flags/indices, one writer each)
static volatile bool usbOn = false, isStreaming = false;
static volatile bool armed = false, sending = false, ending = false, drained = false;
static volatile bool runMic = false, taskInMic = false;
static volatile int  lastPeak = 0;
static bool          micOn = false;
static volatile bool suspended = false;
static uint32_t      endSince = 0;

// Task-only state
static Frame    ring[RING];
static size_t   head = 0, tail = 0;  // tail = next frame to send
static Frame    cur;
static size_t   curSamples = 0, frameSamples = MAX_SAMPLES;
static uint16_t seq = 0;
static adpcm::State enc;  // codec state at the end of the last encoded sample
static bool     wasArmed = false;
static volatile uint32_t statFrames = 0, statDropped = 0, statBusy = 0;

// IMA-ADPCM: logic/adpcm.cpp (shared with the clip decoder; unit-tested natively).

// --- Agent frames ----------------------------------------------------------------

static void startFrame() {
  adpcm::writeHeader(cur.data, seq, enc);
  seq++;
  curSamples = 0;
}

static void pushFrame() {
  if (!curSamples) return;
  cur.len = HEADER + (curSamples + 1) / 2;
  ring[head % RING] = cur;
  head++;
  if (head - tail > RING) { tail = head - RING; statDropped++; }  // full: drop the oldest
  startFrame();
}

static void resetEncoder() {
  head = tail = 0;
  seq = 0;
  statFrames = statDropped = statBusy = 0;
  enc = adpcm::State();
  // Biggest frame that fits one Bluetooth notification on this link
  size_t room = buddy::audioPayloadMax();
  frameSamples = room > HEADER ? min(MAX_SAMPLES, ((room - HEADER) * 2) & ~(size_t)1) : MAX_SAMPLES;
  startFrame();
}

static void addSamples(const int16_t* s, size_t n) {
  for (size_t i = 0; i < n; i++) {
    uint8_t code = adpcm::encodeSample(s[i], enc);
    uint8_t& b = cur.data[HEADER + curSamples / 2];
    if (curSamples & 1) b |= code << 4;
    else b = code;
    if (++curSamples >= frameSamples) pushFrame();
  }
}

static void drain() {
  for (int n = 0; n < SEND_PER_CHUNK && tail < head; n++) {
    const Frame& f = ring[tail % RING];
    if (!buddy::sendAudio(f.data, f.len)) { statBusy++; break; }  // link busy: try again next chunk
    tail++;
    statFrames++;
  }
}

// --- Recording task ----------------------------------------------------------------

static void process(int16_t* buf, int chunkNo) {
  if (chunkNo < SETTLE_CHUNKS) memset(buf, 0, CHUNK * sizeof(int16_t));
  int p = 0;
  for (size_t i = 0; i < CHUNK; i++) p = max(p, abs((int)buf[i]));
  lastPeak = p;
  if (usbOn && isStreaming) uac.write(buf, CHUNK * sizeof(int16_t));
  if (armed && !ending) addSamples(buf, CHUNK);
}

// Records 10 ms chunks while the mic runs (three buffers: the mic fills two
// while we use the third). Otherwise keeps the USB stream fed with silence.
static void task(void*) {
  static int16_t buf[3][CHUNK];
  static const int16_t zeros[CHUNK] = {};
  int i = 0, primed = 0, chunkNo = 0;
  for (;;) {
    if (armed && !wasArmed) resetEncoder();
    wasArmed = armed;
    if (armed) {
      if (sending) drain();
      if (ending) {
        pushFrame();  // the last, partial frame
        if (tail >= head) drained = true;
      }
    }

    taskInMic = true;
    if (runMic) {
      M5.Mic.record(buf[i], CHUNK, RATE);  // blocks until a slot is free
      if (primed >= 2) process(buf[(i + 1) % 3], chunkNo++);  // queued two calls ago, so done
      else primed++;
      i = (i + 1) % 3;
      continue;
    }
    taskInMic = false;
    i = primed = chunkNo = 0;
    if (isStreaming) uac.write(zeros, sizeof zeros);
    vTaskDelay(pdMS_TO_TICKS(10));
  }
}

static void onEvent(void*, esp_event_base_t, int32_t id, void* data) {
  if (id != ARDUINO_USB_AUDIO_CARD_INTERFACE_ENABLE_EVENT) return;
  auto* d = (arduino_usb_audio_card_event_data_t*)data;
  if (d->interface_enable.interface != UAC_INTERFACE_MIC) return;
  isStreaming = d->interface_enable.enable;
}

void begin() {
  uac.onEvent(onEvent);
  uac.begin();
  auto cfg = M5.Mic.config();
  cfg.magnification = GAIN;
  cfg.noise_filter_level = 0;
  M5.Mic.config(cfg);
  xTaskCreatePinnedToCore(task, "mic", 4096, nullptr, 3, nullptr, 1);
}

// The mic chip is set up over the same I2C bus as the power chip and IMU, so
// it is started and stopped here, in the main loop, never from the task.
void loop() {
  if (ending && (drained || millis() - endSince > END_TIMEOUT_MS)) {
    if (sending) {
      buddy::agentTalkEvent(false);
      Log.printf("Agent audio: %lu frames sent, %lu dropped, %lu busy retries, link %s\n", (unsigned long)statFrames,
                 (unsigned long)statDropped, (unsigned long)statBusy, buddy::linkInfo().c_str());
    }
    armed = sending = ending = drained = false;
  }

  // The mic runs for the whole press, even while the Mac briefly reopens the
  // USB input (Wispr does this as it starts), so the first words aren't cut off.
  bool want = (usbOn || armed) && !suspended;
  if (want && !micOn) {
    M5.Speaker.end();
    micOn = M5.Mic.begin();
    runMic = micOn;
    if (!micOn) Log.println("mic: failed to start");
  } else if (!want && micOn) {
    runMic = false;
    uint32_t t = millis();
    while (taskInMic && millis() - t < 100) delay(1);
    M5.Mic.end();
    micOn = false;
    Log.printf("mic: off (last peak %d)\n", lastPeak);
  }
}

void usbTalk(bool on) { usbOn = on; }
void suspend(bool on) { suspended = on; }

void preRoll() {
  if (armed) return;
  drained = ending = sending = false;
  armed = true;
}

void agentTalk() {
  if (!armed) preRoll();
  buddy::agentTalkEvent(true);
  sending = true;
}

void agentEnd() {
  if (!armed || ending) return;
  if (!sending) { armed = false; return; }  // just a click: drop the pre-roll
  endSince = millis();
  ending = true;
}

bool usbStreaming() { return isStreaming; }
bool live() { return micOn; }
int  peak() { return lastPeak; }

}  // namespace mic
