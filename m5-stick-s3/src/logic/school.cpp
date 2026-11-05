#include "school.h"
#include <math.h>
#include <string.h>

namespace school {

constexpr uint32_t RISE_MS = 500;  // a finished fish swims up this long before it leaps

float sizeFor(uint32_t secs) {
  float k = secs >= 3600 ? 1.0f : secs / 3600.0f;
  return 0.8f + 0.8f * k;
}

uint8_t hueFor(const char* project) {
  uint32_t h = 2166136261u;  // FNV-1a
  for (const char* p = project ? project : ""; *p; p++) h = (h ^ (uint8_t)*p) * 16777619u;
  return (uint8_t)(h % HUES);
}

uint32_t School::rand() {
  rng ^= rng << 13;
  rng ^= rng >> 17;
  rng ^= rng << 5;
  return rng;
}

float School::frand(float lo, float hi) { return lo + (hi - lo) * (rand() % 10000) / 10000.0f; }

void School::pickTarget(Fish& f, bool sleepy) {
  f.tx = frand(8, (float)w - 8);
  f.ty = sleepy ? frand(h * 0.55f, h - 8.0f) : frand(10, (float)h - 10);
}

int School::freeSlot() const {
  for (int i = 0; i < MAX_FISH; i++)
    if (!fish[i].on) return i;
  return -1;
}

void School::begin(uint32_t seed, int w_, int h_, uint32_t now) {
  rng = seed ? seed : 1;
  w = w_ > 20 ? w_ : 20;
  h = h_ > 20 ? h_ : 20;
  clear(now);
}

void School::clear(uint32_t now) {
  for (auto& f : fish) f = Fish{};
  for (int i = 0; i < AMBIENT; i++) {
    Fish& f = fish[i];
    f.on = true;
    f.ambient = true;
    f.x = frand(10, (float)w - 10);
    f.y = frand(h * 0.3f, h * 0.8f);
    f.size = frand(0.55f, 0.7f);
    f.hue = (uint8_t)i;
    f.born = now;
    f.vx = (i ? -1 : 1) * 8.0f;
    pickTarget(f, false);
  }
}

void School::resize(int w_, int h_) {
  float sx = (float)w_ / w, sy = (float)h_ / h;
  w = w_;
  h = h_;
  for (auto& f : fish) {
    f.x *= sx; f.tx *= sx; f.jumpX *= sx;
    f.y *= sy; f.ty *= sy;
  }
}

int School::count() const {
  int n = 0;
  for (auto& f : fish) n += f.on;
  return n;
}

int School::sessions() const {
  int n = 0;
  for (auto& f : fish) n += f.on && !f.ambient;
  return n;
}

void School::sync(const Session* s, int n, uint32_t now) {
  // finished: their fish jump
  for (auto& f : fish) {
    if (!f.on || f.ambient || f.st != State::SWIM) continue;
    bool still = false;
    for (int i = 0; i < n; i++) still |= s[i].id == f.id;
    if (!still) {
      f.st = State::JUMP;
      f.jumpAt = now;
    }
  }
  // running: grow, or swim in
  for (int i = 0; i < n; i++) {
    Fish* f = nullptr;
    for (auto& g : fish)
      if (g.on && !g.ambient && g.id == s[i].id && g.st == State::SWIM) f = &g;
    if (!f) {
      int slot = freeSlot();
      if (slot < 0) continue;  // a full tank: this one waits
      f = &fish[slot];
      *f = Fish{};
      f->on = true;
      f->id = s[i].id;
      f->hue = hueFor(s[i].project);
      f->born = now;
      bool left = rand() & 1;
      f->x = left ? -12.0f : w + 12.0f;
      f->y = frand(h * 0.2f, h * 0.75f);
      f->vx = left ? 20.0f : -20.0f;
      pickTarget(*f, false);
      f->tx = left ? frand(w * 0.25f, w * 0.6f) : frand(w * 0.4f, w * 0.75f);  // well inside
    }
    f->size = sizeFor(s[i].secs);
    strncpy(f->label, s[i].project ? s[i].project : "", sizeof f->label - 1);
    f->label[sizeof f->label - 1] = 0;
  }
}

float School::jumpPhase(const Fish& f, uint32_t now) {
  uint32_t e = now - f.jumpAt;
  if (e < RISE_MS) return (float)e / RISE_MS - 1.0f;
  return (float)(e - RISE_MS) / (JUMP_MS - RISE_MS);
}

void School::step(float dt, float drift, bool sleepy, uint32_t now) {
  if (dt <= 0) return;
  if (dt > 0.25f) dt = 0.25f;
  if (drift > 1) drift = 1;
  if (drift < -1) drift = -1;
  for (auto& f : fish) {
    if (!f.on) continue;
    if (f.st == State::JUMP) {
      float p = jumpPhase(f, now);
      if (p >= 1) { f.on = false; continue; }
      if (p < 0) {  // swim up to just under the surface
        f.y += (2 - f.y) * (dt * 6 > 1 ? 1 : dt * 6);
        f.x += f.vx * dt * 0.5f;
        f.jumpX = f.x;
      } else {      // the leap: an arc above the surface, then back in
        float dir = f.vx >= 0 ? 1.0f : -1.0f;
        f.x = f.jumpX + dir * 26 * p;
        f.y = -sinf(3.14159f * p) * h * 0.32f;
      }
      continue;
    }
    const float speed = sleepy ? 3.5f : (f.ambient ? 9 : 12) + 8 * f.size;
    float tx = f.tx + drift * w * 0.45f;  // tilted: the low side pulls
    if (tx < 6) tx = 6;
    if (tx > w - 6) tx = w - 6;
    float dx = tx - f.x, dy = f.ty - f.y, d = sqrtf(dx * dx + dy * dy);
    if (d < 6 || (rand() % 1000) < (uint32_t)(dt * 120)) {
      pickTarget(f, sleepy);
    } else {
      float k = dt * 2.2f > 1 ? 1 : dt * 2.2f;
      f.vx += (dx / d * speed - f.vx) * k;
      f.vy += (dy / d * speed * 0.6f - f.vy) * k;
    }
    f.x += (f.vx + drift * 10) * dt;
    f.y += f.vy * dt;
    if (f.y < 6) { f.y = 6; f.vy = fabsf(f.vy); }
    if (f.y > h - 5) { f.y = h - 5; f.vy = -fabsf(f.vy); }
    if (now - f.born > 4000) {  // swum in: stay inside
      if (f.x < 4) f.x = 4;
      if (f.x > w - 4) f.x = w - 4;
    }
  }
}

void School::scatter() {
  for (auto& f : fish) {
    if (!f.on || f.st != State::SWIM) continue;
    bool right = f.x < w / 2.0f;
    f.vx = right ? 70.0f : -70.0f;
    f.vy = frand(-30, 30);
    f.tx = right ? w - 10.0f : 10.0f;
    f.ty = frand(10, (float)h - 10);
  }
}

}  // namespace school
