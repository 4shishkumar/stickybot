#include "rmt.h"
#include <driver/rmt_rx.h>
#include <driver/rmt_tx.h>
#include <driver/gpio.h>
#include <esp_heap_caps.h>
#include <esp_timer.h>
#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>
#include <freertos/task.h>
#include "pins.h"
#include "../logic/ir_codec.h"
#include "../netlog.h"

namespace ir {
namespace rmt {

constexpr uint32_t RES_HZ = 1000000;  // 1 us ticks
constexpr uint16_t MAX_TICKS = 32767;

// --- TX ----------------------------------------------------------------------------------------

static rmt_channel_handle_t txCh = nullptr;
static rmt_encoder_handle_t copyEnc = nullptr;
static bool txFailed = false;
static uint32_t txCarrier = 0;
static rmt_symbol_word_t txSym[ircodec::MAX_RAW / 2 + 16];

bool txReady() {
  if (txCh) return true;
  if (txFailed) return false;
  rmt_tx_channel_config_t cfg = {};
  cfg.gpio_num = TX_PIN;
  cfg.clk_src = RMT_CLK_SRC_DEFAULT;
  cfg.resolution_hz = RES_HZ;
  cfg.mem_block_symbols = 48;   // refilled from txSym while it sends
  cfg.trans_queue_depth = 2;
  cfg.flags.invert_out = !TX_ACTIVE_HIGH;
  rmt_copy_encoder_config_t ec = {};
  if (rmt_new_tx_channel(&cfg, &txCh) != ESP_OK || rmt_new_copy_encoder(&ec, &copyEnc) != ESP_OK ||
      rmt_enable(txCh) != ESP_OK) {
    Log.println("IR: the RMT TX channel couldn't be made");
    txFailed = true;
    txCh = nullptr;
    return false;
  }
  Log.printf("IR: TX on G%d\n", (int)TX_PIN);
  return true;
}

bool txBusy() { return txCh && rmt_tx_wait_all_done(txCh, 0) != ESP_OK; }

const char* txState() { return txFailed ? "error" : txCh ? "ok" : "off"; }

bool send(const uint16_t* t, size_t n, uint32_t carrierHz) {
  if (!n || !txReady() || txBusy()) return false;
  if (carrierHz != txCarrier) {
    rmt_carrier_config_t cc = {};
    cc.frequency_hz = carrierHz;
    cc.duty_cycle = TX_DUTY;
    if (rmt_apply_carrier(txCh, &cc) != ESP_OK) return false;
    txCarrier = carrierHz;
  }
  // (level, duration) pieces, two per symbol; long ones split
  size_t k = 0;
  bool half = false;
  const size_t cap = sizeof txSym / sizeof txSym[0];
  auto piece = [&](uint32_t level, uint32_t d) {
    while (d) {
      const uint16_t part = d > MAX_TICKS ? MAX_TICKS : d;
      d -= part;
      if (k >= cap) return false;
      if (!half) {
        txSym[k].level0 = level;
        txSym[k].duration0 = part;
      } else {
        txSym[k].level1 = level;
        txSym[k].duration1 = part;
        k++;
      }
      half = !half;
    }
    return true;
  };
  for (size_t i = 0; i < n; i++)
    if (!piece(i % 2 == 0 ? 1 : 0, t[i] ? t[i] : 1)) return false;
  if (half && !piece(0, 100)) return false;  // a symbol needs both halves: a short space ends it
  rmt_transmit_config_t tc = {};
  tc.loop_count = 0;
  tc.flags.eot_level = 0;
  return rmt_transmit(txCh, copyEnc, txSym, k * sizeof(rmt_symbol_word_t), &tc) == ESP_OK;
}

// --- RX ----------------------------------------------------------------------------------------

constexpr size_t BUF_SYMBOLS = 512;
struct RxEvent {
  uint8_t  buf;
  uint16_t n;
  int64_t  at;
};

static TaskHandle_t rxTask = nullptr;
static QueueHandle_t rxQ = nullptr;
static volatile bool rxWant = false, rxRunning = false, rxError = false;
static volatile uint32_t frames = 0;
static portMUX_TYPE mux = portMUX_INITIALIZER_UNLOCKED;
// the capture being joined (task), the one handed over (loop)
static uint16_t* cap = nullptr;    // PSRAM (task context only)
static size_t capN = 0;
static int64_t capLast = 0;
static uint16_t* ready = nullptr;
static uint16_t* frameBuf = nullptr;
static volatile size_t readyN = 0;

static bool IRAM_ATTR onDone(rmt_channel_handle_t, const rmt_rx_done_event_data_t* e, void* ctx) {
  BaseType_t woke = pdFALSE;
  RxEvent ev{(uint8_t)(uintptr_t)ctx, (uint16_t)e->num_symbols, esp_timer_get_time()};
  xQueueSendFromISR(rxQ, &ev, &woke);
  return woke == pdTRUE;
}

static void publish() {
  portENTER_CRITICAL(&mux);
  memcpy(ready, cap, capN * sizeof(uint16_t));
  readyN = capN;
  portEXIT_CRITICAL(&mux);
  capN = 0;
}

// One frame's symbols -> durations appended to cap (with the gap since the last frame)
static void addFrame(const rmt_symbol_word_t* s, size_t ns, int64_t at) {
  uint16_t* f = frameBuf;
  size_t n = 0;
  uint32_t total = 0;
  int last = -1;  // 1 mark, 0 space
  auto put = [&](int level, uint32_t d) {
    if (!d) return;
    const int mark = RX_ACTIVE_LOW ? !level : level;
    if (n == 0 && !mark) return;  // starts with a mark
    total += d;
    if (mark == last && n) {
      f[n - 1] = (uint16_t)min<uint32_t>(65535, f[n - 1] + d);
    } else if (n < ircodec::MAX_RAW) {
      f[n++] = (uint16_t)min<uint32_t>(65535, d);
      last = mark;
    }
  };
  for (size_t i = 0; i < ns; i++) {
    put(s[i].level0, s[i].duration0);
    put(s[i].level1, s[i].duration1);
  }
  if (n && last == 0) n--;  // the idle at the end
  if (!n) return;
  frames++;
  if (capN && at - capLast > (int64_t)JOIN_MS * 1000) publish();
  if (capN) {
    int64_t gap = (at - (int64_t)total) - capLast;  // this frame's first edge - the last frame's end
    if (gap < (int64_t)ircodec::FRAME_GAP_US + 1) gap = ircodec::FRAME_GAP_US + 1;
    if (capN + 1 + n > ircodec::MAX_RAW) return;  // full: keep what we have
    cap[capN++] = (uint16_t)min<int64_t>(65535, gap);
  }
  const size_t room = ircodec::MAX_RAW - capN;
  memcpy(cap + capN, f, min(n, room) * sizeof(uint16_t));
  capN += min(n, room);
  capLast = at;  // ~ the end of the frame (+ the idle timeout, the same for every frame)
}

static void rxLoop(void*) {
  rmt_channel_handle_t ch = nullptr;
  rmt_symbol_word_t* buf[2] = {nullptr, nullptr};
  rmt_rx_channel_config_t cfg = {};
  cfg.gpio_num = RX_PIN;
  cfg.clk_src = RMT_CLK_SRC_DEFAULT;
  cfg.resolution_hz = RES_HZ;
  cfg.mem_block_symbols = BUF_SYMBOLS;
  cfg.flags.with_dma = 1;
  size_t bufSymbols = BUF_SYMBOLS;
  if (rmt_new_rx_channel(&cfg, &ch) != ESP_OK) {  // no DMA: the channel's own memory (4 blocks)
    cfg.flags.with_dma = 0;
    cfg.mem_block_symbols = 192;
    bufSymbols = 192;
    if (rmt_new_rx_channel(&cfg, &ch) != ESP_OK) ch = nullptr;
  }
  for (int i = 0; i < 2 && ch; i++) {
    buf[i] = (rmt_symbol_word_t*)heap_caps_malloc(BUF_SYMBOLS * sizeof(rmt_symbol_word_t),
                                                  MALLOC_CAP_DMA | MALLOC_CAP_INTERNAL);
    if (!buf[i]) ch = nullptr;
  }
  rmt_receive_config_t rc = {};
  rc.signal_range_min_ns = 1000;       // shorter pulses are noise
  rc.signal_range_max_ns = 12000000;   // 12 ms without an edge ends a frame
  int cur = 0;
  if (ch) {
    rmt_rx_event_callbacks_t cbs = {};
    cbs.on_recv_done = onDone;
    // the buffer index rides in the callback's context: re-registered per receive is not
    // allowed, so the task tracks which buffer is armed and the ISR reports `cur` at arm time
    rmt_rx_register_event_callbacks(ch, &cbs, nullptr);
    if (rmt_enable(ch) != ESP_OK ||
        rmt_receive(ch, buf[0], bufSymbols * sizeof(rmt_symbol_word_t), &rc) != ESP_OK)
      ch = nullptr;
  }
  if (!ch) {
    Log.println("IR: the RMT RX channel couldn't be made");
    rxError = true;
  } else {
    Log.printf("IR: listening on G%d (%s)\n", (int)RX_PIN, cfg.flags.with_dma ? "DMA" : "no DMA");
  }
  rxRunning = ch != nullptr;
  while (rxWant && ch) {
    RxEvent ev;
    if (xQueueReceive(rxQ, &ev, pdMS_TO_TICKS(30)) == pdTRUE) {
      const int done = cur;
      cur ^= 1;
      rmt_receive(ch, buf[cur], bufSymbols * sizeof(rmt_symbol_word_t), &rc);  // re-arm at once
      addFrame(buf[done], ev.n, ev.at);
    } else if (capN && esp_timer_get_time() - capLast > (int64_t)JOIN_MS * 1000) {
      publish();
    }
  }
  if (ch) {
    rmt_disable(ch);
    rmt_del_channel(ch);
  }
  for (auto b : buf)
    if (b) heap_caps_free(b);
  capN = 0;
  rxRunning = false;
  rxTask = nullptr;
  vTaskDelete(nullptr);
}

bool rxStart() {
  if (rxTask) return !rxError;
  for (uint16_t** b : {&cap, &ready, &frameBuf}) {
    if (*b) continue;
    *b = (uint16_t*)heap_caps_malloc(ircodec::MAX_RAW * sizeof(uint16_t), MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    if (!*b) *b = (uint16_t*)malloc(ircodec::MAX_RAW * sizeof(uint16_t));
    if (!*b) return false;
  }
  if (!rxQ) rxQ = xQueueCreate(8, sizeof(RxEvent));
  xQueueReset(rxQ);
  rxError = false;
  rxWant = true;
  frames = 0;
  capN = 0;
  readyN = 0;
  if (xTaskCreatePinnedToCore(rxLoop, "ir-rx", 4096, nullptr, 5, &rxTask, 1) != pdPASS) {
    rxTask = nullptr;
    rxWant = false;
    return false;
  }
  for (int i = 0; i < 50 && !rxRunning && !rxError; i++) delay(2);
  return !rxError;
}

void rxStop() {
  rxWant = false;
  for (int i = 0; i < 60 && rxTask; i++) delay(5);
}

bool rxActive() { return rxRunning; }
uint32_t rxFrames() { return frames; }
const char* rxState() { return rxError ? "error" : rxRunning ? "on" : "off"; }

size_t takeCapture(uint16_t* out, size_t max) {
  if (!readyN) return 0;
  portENTER_CRITICAL(&mux);
  const size_t n = min((size_t)readyN, max);
  memcpy(out, ready, n * sizeof(uint16_t));
  readyN = 0;
  portEXIT_CRITICAL(&mux);
  return n;
}

int pinLevel(int gpio) {
  if ((gpio == TX_PIN && txCh) || (gpio == RX_PIN && rxRunning)) return -1;
  gpio_reset_pin((gpio_num_t)gpio);
  gpio_set_direction((gpio_num_t)gpio, GPIO_MODE_INPUT);
  gpio_set_pull_mode((gpio_num_t)gpio, GPIO_FLOATING);
  delayMicroseconds(200);
  return gpio_get_level((gpio_num_t)gpio);
}

}  // namespace rmt
}  // namespace ir
