#include "games_one.h"
#include <math.h>

namespace games {

static constexpr uint32_t STEP_MS = 8;  // physics steps: a long frame is split into these

template <typename F>
static void substeps(uint32_t ms, F f) {
  while (ms) {
    const uint32_t s = ms > STEP_MS ? STEP_MS : ms;
    f(s / 1000.0f);
    ms -= s;
  }
}

static float minf(float a, float b) { return a < b ? a : b; }
static float maxf(float a, float b) { return a > b ? a : b; }

// --- Stack ---------------------------------------------------------------------------------

void Stack::reset(uint32_t seed) {
  *this = Stack();
  rng_.seed_(seed);
  dir_ = rng_.range(0, 1) ? 1 : -1;  // which side the first block comes from
  ring_[0] = {(W - START_W) / 2, START_W};
  w_ = START_W;
  next();
}

void Stack::next() {
  speed_ = minf(90 + 5.0f * placed_, 240);
  dir_ = -dir_;  // from the other side each time
  x_ = dir_ > 0 ? lo() : hi();
}

void Stack::step(uint32_t ms) {
  substeps(ms, [&](float dt) {
    if (!over) {
      x_ += dir_ * speed_ * dt;
      if (x_ > hi()) { x_ = hi() - (x_ - hi()); dir_ = -1; }
      if (x_ < lo()) { x_ = lo() + (lo() - x_); dir_ = 1; }
    }
    if (debris()) {
      debV_ += 900 * dt;
      debFall_ += debV_ * dt;
    }
  });
  debT_ = debT_ + ms > 99999 ? 99999 : debT_ + ms;
  t += ms;
}

Stack::Drop Stack::drop() {
  if (over) return Drop::MISS;
  const Block top = block(placed_);
  dropAt_ = t;
  if (fabsf(x_ - top.x) <= PERFECT_PX) {  // snaps on
    combo_++;
    score += 2;
    Block nb{top.x, w_};
    if (combo_ % 3 == 0 && w_ < START_W) {  // three in a row: a little wider again
      const float nw = minf(START_W, w_ + GROW_PX);
      nb.x = minf(maxf(0, nb.x - (nw - w_) / 2), W - nw);
      nb.w = w_ = nw;
    }
    ring_[++placed_ % KEEP] = nb;
    fx |= FX_ACTION | FX_PERFECT;
    last_ = Drop::PERFECT;
    next();
    return last_;
  }
  const float l = maxf(x_, top.x), r = minf(x_ + w_, top.x + top.w);
  debLevel_ = placed_ + 1;
  debFall_ = debV_ = 0;
  debT_ = 0;
  if (r - l < 1) {  // missed the tower
    debX_ = x_;
    debW_ = w_;
    over = true;
    fx |= FX_HIT | FX_OVER;
    last_ = Drop::MISS;
    return last_;
  }
  if (x_ < top.x) { debX_ = x_; debW_ = top.x - x_; }
  else { debX_ = top.x + top.w; debW_ = x_ + w_ - debX_; }
  ring_[++placed_ % KEEP] = {l, r - l};
  w_ = r - l;
  combo_ = 0;
  score += 1;
  fx |= FX_ACTION | FX_SCORE;
  last_ = Drop::CUT;
  next();
  return last_;
}

// --- Flappy --------------------------------------------------------------------------------

void Flappy::reset(uint32_t seed) {
  *this = Flappy();
  rng_.seed_(seed);
  y_ = H * 0.42f;
  speed_ = 62;
  for (int i = 0; i < PIPES; i++) place(pipes_[i], W + 40 + i * SPACING);
}

void Flappy::place(Pipe& p, float x) {
  p.x = x;
  p.gapH = maxf(54, 72 - score * 0.6f);
  const float margin = 34;
  p.gapY = rng_.between(margin + p.gapH / 2, GROUND - margin - p.gapH / 2);
  p.scored = false;
}

void Flappy::flap() {
  if (over) return;
  started_ = true;
  vy_ = FLAP_V;
  fx |= FX_ACTION;
}

void Flappy::step(uint32_t ms) {
  substeps(ms, [&](float dt) {
    if (!started_) {  // waiting for the first flap: bob in place
      scroll_ += 40 * dt;
      return;
    }
    vy_ = minf(vy_ + GRAVITY * dt, MAX_FALL);
    y_ += vy_ * dt;
    if (y_ < R) { y_ = R; vy_ = 0; }
    if (y_ + R >= GROUND) {
      y_ = GROUND - R;
      vy_ = 0;
      if (!over) { over = true; fx |= FX_HIT | FX_OVER; }
    }
    if (over) return;  // falls to the ground, the pipes stop
    scroll_ += speed_ * dt;
    float far = -1000;
    for (auto& p : pipes_) {
      p.x -= speed_ * dt;
      far = maxf(far, p.x);
    }
    for (auto& p : pipes_) {
      if (p.x + PIPE_W < -2) {  // gone off the left: the next one after the last
        place(p, far + SPACING);
        far = p.x;
      }
      if (!p.scored && p.x + PIPE_W < BIRD_X - R) {
        p.scored = true;
        score++;
        fx |= FX_SCORE;
        speed_ = minf(62 + score * 1.2f, 100);
      }
      const float r = R - 1.5f;  // a little forgiving
      const float top = p.gapY - p.gapH / 2, bottom = p.gapY + p.gapH / 2;
      if (circleHitsBox(BIRD_X, y_, r, p.x, -200, PIPE_W, top + 200) ||
          circleHitsBox(BIRD_X, y_, r, p.x, bottom, PIPE_W, GROUND - bottom + 10)) {
        over = true;
        fx |= FX_HIT | FX_OVER;
        if (vy_ < 0) vy_ = 0;
        return;
      }
    }
  });
  if (!started_) y_ = H * 0.42f + sinf(t / 260.0f) * 4;
  t += ms;
}

// --- Dino ----------------------------------------------------------------------------------

void Dino::reset(uint32_t seed) {
  *this = Dino();
  rng_.seed_(seed);
  for (auto& o : obs_) o.active = false;
  nextGap_ = 0;
  spawn();
  obs_[0].x = W + 60;  // a moment before the first one
}

void Dino::press() {
  holding_ = true;
  if (over || !onGround()) return;
  vy_ = JUMP_V;
  holdMs_ = 0;
  fx |= FX_ACTION;
}

void Dino::release() { holding_ = false; }

void Dino::spawn() {
  Obstacle* o = nullptr;
  for (auto& x : obs_)
    if (!x.active) { o = &x; break; }
  if (!o) return;
  o->active = true;
  o->x = W + 4;
  o->fly = dist_ > 1500 && rng_.unit() < 0.25f;
  if (o->fly) {  // a drone at head height: stay down (or hop low)
    o->w = 18;
    o->h = 10;
    o->lift = rng_.between(27, 33);
  } else {      // a bug on the ground: wider and taller as it goes
    o->w = rng_.between(10, 16);
    if (dist_ > 800 && rng_.unit() < 0.2f) o->w *= 2;  // a pair
    o->h = rng_.between(14, 24 + minf(dist_ / 400, 10));
    o->lift = 0;
  }
  nextGap_ = speed_ * rng_.between(0.7f, 1.4f) + 50;
}

void Dino::step(uint32_t ms) {
  substeps(ms, [&](float dt) {
    if (over) return;
    dist_ += speed_ * dt;
    speed_ = minf(START_SPEED + dist_ * 0.008f, MAX_SPEED);
    if (y_ > 0 || vy_ > 0) {
      const bool floaty = holding_ && vy_ > 0 && holdMs_ < HOLD_MS;
      holdMs_ += dt * 1000;
      vy_ -= (floaty ? G_HOLD : G) * dt;
      y_ += vy_ * dt;
      if (y_ <= 0) { y_ = 0; vy_ = 0; }
    }
    float right = -1000;
    for (auto& o : obs_) {
      if (!o.active) continue;
      o.x -= speed_ * dt;
      if (o.x + o.w < -6) { o.active = false; continue; }
      right = maxf(right, o.x + o.w);
      // the bot's box, a couple of pixels smaller than drawn (fair at speed)
      const float bx = BOT_X - BOT_W / 2 + 3, bw = BOT_W - 6, bTop = GROUND - y_ - BOT_H + 3, bBot = GROUND - y_;
      const float oTop = GROUND - o.lift - o.h, oBot = GROUND - o.lift;
      if (bx < o.x + o.w && bx + bw > o.x && bTop < oBot - 1 && bBot > oTop + 1) {
        over = true;
        fx |= FX_HIT | FX_OVER;
        return;
      }
    }
    if (right < W - nextGap_) spawn();
    const long s = (long)(dist_ / 10);
    if (s / 100 != score / 100) fx |= FX_SCORE;  // every 100: a ding
    score = s;
  });
  t += ms;
}

// --- Reaction ------------------------------------------------------------------------------

void Reaction::reset(uint32_t seed) {
  *this = Reaction();
  rng_.seed_(seed);
  wait();
}

void Reaction::wait() {
  phase_ = Phase::WAIT;
  phaseAt_ = t;
  waitMs_ = (uint32_t)rng_.range(MIN_WAIT, MAX_WAIT);
}

void Reaction::step(uint32_t ms) {
  t += ms;
  switch (phase_) {
    case Phase::WAIT:
      if (inPhase() >= waitMs_) {
        phase_ = Phase::GO;
        phaseAt_ = t;  // from now: the screen turns green right after this step
        fx |= FX_GO;
      }
      break;
    case Phase::GO:
      if (inPhase() >= SLOW_MS + 500) {  // asleep? counts as slow
        times_[round_] = SLOW_MS;
        phase_ = Phase::SHOWN;
        phaseAt_ = t;
        fx |= FX_HIT;
      }
      break;
    case Phase::SHOWN:
      if (inPhase() >= SHOW_MS) {
        if (round_ + 1 < ROUNDS) {
          round_++;
          wait();
        } else {
          long sum = 0;
          for (int v : times_) sum += v;
          score = (sum + ROUNDS / 2) / ROUNDS;
          phase_ = Phase::DONE;
          over = true;
          fx |= FX_OVER;
        }
      }
      break;
    default:
      break;
  }
}

void Reaction::press(uint32_t lateMs) {
  if (phase_ == Phase::WAIT) {  // too early
    phase_ = Phase::FALSE_START;
    phaseAt_ = t;
    score = 0;
    over = true;
    fx |= FX_HIT | FX_OVER;
  } else if (phase_ == Phase::GO) {
    uint32_t ms = inPhase();
    ms = ms > lateMs ? ms - lateMs : 1;
    times_[round_] = ms > SLOW_MS ? SLOW_MS : (int)ms;
    phase_ = Phase::SHOWN;
    phaseAt_ = t;
    fx |= FX_SCORE;
  }
}

}  // namespace games
