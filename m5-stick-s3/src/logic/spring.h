// A damped spring: a value that follows its target smoothly and settles without ringing
// (the Aquarium's water surface and the fish drifting with the tilt). Pure C++.
#pragma once

namespace spring {

struct Spring {
  float x = 0;  // where it is
  float v = 0;  // how fast it moves
  float k = 40;      // stiffness (1/s^2): higher follows faster
  float damping = 11; // (1/s): about critical for k = 30-40, so it barely overshoots

  // Advances by dt seconds towards target. Long frames are split into short steps, so a
  // slow redraw (battery) moves it the same way, just in fewer pictures.
  void step(float target, float dt);
};

}  // namespace spring
