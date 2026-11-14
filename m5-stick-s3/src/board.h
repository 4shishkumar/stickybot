// What differs between stick models. One board is picked at build time (-D BOARD_STICKS3 or
// -D BOARD_STICKC_PLUS2 in platformio.ini); each has its own board_<name>.cpp. Everything else
// in the firmware goes through M5Unified, which already knows both.
#pragma once
#include <driver/gpio.h>

#if !defined(BOARD_STICKS3) && !defined(BOARD_STICKC_PLUS2)
#define BOARD_STICKS3 1  // the first board; builds without a flag keep working
#endif

#if defined(BOARD_STICKS3)
#define STICK_BOARD "m5-sticks3"  // not BOARD_ID: M5Unified tests that one as a number
#define BOARD_HAS_USB_MIC 1       // native USB: the stick is also a USB microphone
#elif defined(BOARD_STICKC_PLUS2)
#define STICK_BOARD "m5-stickc-plus2"
#define BOARD_HAS_USB_MIC 0       // USB goes through a serial chip
#endif

namespace board {

#if defined(BOARD_STICKS3)
constexpr gpio_num_t WAKE_PIN = GPIO_NUM_11;  // button A
constexpr bool WAKE_PIN_PULLUP = true;        // no pull-up on the board
constexpr gpio_num_t IR_TX_PIN = GPIO_NUM_46;
constexpr gpio_num_t IR_RX_PIN = GPIO_NUM_42;
constexpr bool HAS_IR_RX = true;
constexpr bool HAS_POWER_CHIP = true;         // PM1: charger switch, status LED, USB voltage
#elif defined(BOARD_STICKC_PLUS2)
constexpr gpio_num_t WAKE_PIN = GPIO_NUM_37;  // button A (input only, pulled up on the board)
constexpr bool WAKE_PIN_PULLUP = false;
constexpr gpio_num_t IR_TX_PIN = GPIO_NUM_19; // shared with the red LED
constexpr gpio_num_t IR_RX_PIN = GPIO_NUM_NC; // no receiver
constexpr bool HAS_IR_RX = false;
constexpr bool HAS_POWER_CHIP = false;        // a hold pin and a battery voltage divider only
#endif

void begin();                 // right after M5.begin()
void speakerAmp(bool on);     // the speaker amp's power
void statusLed(bool on);      // the charging LED
void chargerRestart();        // switch charging off and on again (like re-plugging)
bool chargerEnabled();
int  powerSource();           // the power chip's raw source byte, -1 without one
void prepareStandby();        // just before deep sleep: turn off what M5.begin() turns back on

}  // namespace board
