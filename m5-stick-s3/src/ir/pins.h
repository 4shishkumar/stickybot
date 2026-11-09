// The StickS3's infrared LED and receiver (M5Stack's StickS3 pin map: "IR: G46, G42", labelled
// IR_TX and IR_RX). The page doesn't say outright which is which; the most likely reading is
// TX = G46, RX = G42. `@ir diag` (or `ir diag` on USB serial) reads both pins' idle levels to
// confirm it on a real stick: a receiver module's output idles HIGH, the LED driver's input
// LOW. If they come out the other way round, build with -D IR_PINS_SWAPPED.
//
// No conflict with the rest of the firmware: I2C G47/G48, Grove G9/G10, buttons G11/G12, the
// screen G21/G39/G40/G41/G45, the mic / speaker (I2S) G14-G18.
#pragma once
#include <driver/gpio.h>

namespace ir {

#ifdef IR_PINS_SWAPPED
constexpr gpio_num_t TX_PIN = GPIO_NUM_42;
constexpr gpio_num_t RX_PIN = GPIO_NUM_46;
#else
constexpr gpio_num_t TX_PIN = GPIO_NUM_46;
constexpr gpio_num_t RX_PIN = GPIO_NUM_42;
#endif

constexpr bool     RX_ACTIVE_LOW = true;      // the receiver pulls its output low while it sees a carrier
constexpr bool     TX_ACTIVE_HIGH = true;     // the LED lights while the pin is high
constexpr float    TX_DUTY = 0.33f;           // the carrier's duty cycle

}  // namespace ir
