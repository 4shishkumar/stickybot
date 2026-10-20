// Motion gestures from the BMI270 (accelerometer + gyro).
//   lift        : picked up / moved after resting -> wake the screen
//   face-down   : lying screen-down for a few seconds -> standby
//   shake       : a few sharp jolts -> Escape (or deny a Claude prompt)
//   flick L/R   : quick wrist twist -> previous / next screen page
//   orientation : which way up the stick is held -> auto-rotate the screen
#pragma once
#include <Arduino.h>
#include "logic/gestures.h"

namespace motion {

using Event = gestures::Event;  // NONE, LIFT, FACE_DOWN, SHAKE, FLICK_LEFT, FLICK_RIGHT

void  begin();
Event poll();             // call every loop; returns at most one event
int   upright();          // +1 upright, -1 upside down, 0 unknown/flat
int   sideways();         // lying on its long edge (screen vertical): +1 / -1 by side, 0 otherwise
void  accel(float& x, float& y, float& z);  // last reading (g)
void  gyro(float& x, float& y, float& z);   // last reading (deg/s)

}  // namespace motion
