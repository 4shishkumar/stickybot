#include "spring.h"

namespace spring {

void Spring::step(float target, float dt) {
  if (dt <= 0) return;
  if (dt > 1) dt = 1;  // the screen was off: don't fling it
  const float h = 1 / 120.0f;
  while (dt > 0) {
    float s = dt < h ? dt : h;
    v += (k * (target - x) - damping * v) * s;  // semi-implicit Euler: stable at this step
    x += v * s;
    dt -= s;
  }
}

}  // namespace spring
