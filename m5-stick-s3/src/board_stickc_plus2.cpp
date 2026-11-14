#include "board.h"
#if defined(BOARD_STICKC_PLUS2)
#include <M5Unified.h>

// M5StickC Plus2: no power chip. M5Unified holds the power on (G4) in M5.begin() and reads the
// battery through a divider (G38). The buzzer needs no amp; the charger runs by itself.
namespace board {

void begin() {}
void speakerAmp(bool) {}
void statusLed(bool) {}     // the red LED shares G19 with the IR LED: left alone
void chargerRestart() {}
bool chargerEnabled() { return true; }
int  powerSource() { return -1; }
void prepareStandby() {}

}  // namespace board
#endif
