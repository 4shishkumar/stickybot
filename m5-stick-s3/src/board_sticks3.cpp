#include "board.h"
#if defined(BOARD_STICKS3)
#include <M5Unified.h>

namespace board {

static void pmOutput(m5::M5PM1_Class::gpio_t pin, bool on) {
  auto& pm = M5.Power.M5pm1;
  pm.setGPIOFunction(pin, m5::M5PM1_Class::gpio);
  pm.setGPIOMode(pin, m5::M5PM1_Class::output);
  pm.setGPIOOutput(pin, on);
}

void begin() { M5.Power.M5pm1.setBatteryCharge(true); }  // make sure the charger is on

// The power chip's GPIO3 (M5Unified sets the output bit but doesn't make the pin an output;
// our standby leaves it low).
void speakerAmp(bool on) { pmOutput(m5::M5PM1_Class::gpio3, on); }

void statusLed(bool on) { M5.Power.M5pm1.setLedEnLevel(on); }  // green status LED (PY_STATUS_LED)

void chargerRestart() {
  M5.Power.M5pm1.setBatteryCharge(false);
  delay(300);
  M5.Power.M5pm1.setBatteryCharge(true);
}

bool chargerEnabled() {
  bool on = false;
  M5.Power.M5pm1.getBatteryCharge(&on);
  return on;
}

int powerSource() { return (int)M5.Power.M5pm1.getPowerSource(); }

// The L3B rail (LCD, backlight, mic, speaker amp) off via the power chip.
void prepareStandby() {
  pmOutput(m5::M5PM1_Class::gpio2, false);
  pmOutput(m5::M5PM1_Class::gpio3, false);
}

}  // namespace board
#endif
