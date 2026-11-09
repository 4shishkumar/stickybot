// Motion gestures from the BMI270 (accelerometer + gyro).
//   lift        : picked up / moved after resting -> wake the screen
//   face-down   : lying screen-down for a few seconds -> standby
//   shake       : a few sharp jolts -> Escape (or deny a Claude prompt)
//   flick L/R   : quick wrist twist -> previous / next screen page
//   orientation : which way up the stick is held -> auto-rotate the screen
//                 (portrait / upside down / landscape either way, logic/orientation.cpp)
#pragma once
#include <Arduino.h>
#include "logic/gestures.h"
#include "logic/orientation.h"

namespace motion {

using Event = gestures::Event;  // NONE, LIFT, FACE_DOWN, SHAKE, FLICK_LEFT, FLICK_RIGHT

void  begin();
Event poll();             // call every loop; returns at most one event
int   upright();          // +1 upright, -1 upside down, 0 unknown/flat
int   sideways();         // lying on its long edge (screen vertical): +1 / -1 by side, 0 otherwise
const orientation::Tracker& facing();     // which way up, debounced (auto-rotate)
void  resetFacing(orientation::Orient o);  // at boot: start from the saved flip
void  accel(float& x, float& y, float& z);  // last reading (g)
void  gyro(float& x, float& y, float& z);   // last reading (deg/s)
// The air mouse (air.cpp) reads at 100 Hz instead of 50, and takes each new sample once.
void     setFast(bool on);
uint32_t sampleCount();      // goes up by one per new reading
uint32_t sampleMicros();     // when the last one was read

}  // namespace motion
