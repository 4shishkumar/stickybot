#include "games.h"
#include <M5Unified.h>
#include <Preferences.h>
#include "draw.h"
#include "duo.h"
#include "../ui.h"
#include "../ui_draw.h"
#include "../buddy.h"
#include "../sound.h"
#include "../netlog.h"
#include "../faces/faces.h"
#include "../logic/games_one.h"
#include "../logic/games_tilt.h"

namespace games {

enum class Scr { PICKER, READY, CALIB, PLAY, PAUSE, OVER };

constexpr uint32_t IDLE_CLOSE_MS = 90000;
constexpr uint32_t CALIB_MS = 2000, CALIB_SAMPLE_FROM = 1100;  // the countdown; "flat" = its last 0.9 s
constexpr uint32_t OVER_LOCK_MS = 700;
constexpr uint32_t GAP_PAUSE_MS = 300;   // the loop was away this long (an approval): pause
constexpr uint32_t GAME_CPU_MHZ = 160;

static bool     open_ = false, flipped_ = false, waitA_ = false, waitB_ = false, dirty_ = true;
static int      side_ = 1, sel_ = 0, pauseSel_ = 0;
static Scr      scr_ = Scr::PICKER;
static Id       cur_ = Id::STACK;
static uint32_t last_ = 0, scrAt_ = 0, lastFrame_ = 0, lastInput_ = 0, cpuWas_ = 80;
static long     bests_[COUNT];
static bool     loaded_ = false, newBest_ = false, failed_ = false, reported_ = false;
static long     final_ = 0;
static float    rawX_ = 0, rawY_ = 0;
static uint32_t perfAt_ = 0, perfFrames_ = 0, perfUs_ = 0;
static Preferences prefs;

static Stack    stack_;
static Flappy   flappy_;
static Dino     dino_;
static Reaction reaction_;
static Maze     maze_;
static Balance  balance_;
static Catch    catch_;
static Tilt     tilt_;

// --- Best scores (NVS "games": id -> best) ------------------------------------------------

static void load() {
  if (loaded_) return;
  prefs.begin("games", false);
  for (int i = 0; i < COUNT; i++) bests_[i] = prefs.getLong(info((Id)i).id, 0);
  loaded_ = true;
}

long best(Id g) {
  load();
  return bests_[(int)g];
}

String json() {
  load();
  String j = "\"games\":[";
  for (int i = 0; i < COUNT; i++) {
    const Info& n = info((Id)i);
    if (i) j += ",";
    j += String("{\"id\":\"") + n.id + "\",\"title\":\"" + n.title + "\",\"best\":" + bests_[i] +
         ",\"tilt\":" + (n.tilt ? "true" : "false") + ",\"low\":" + (n.lowerBetter ? "true" : "false") + "}";
  }
  return j + "]";
}

// --- The game being played ------------------------------------------------------------------

static Base& game() {
  switch (cur_) {
    case Id::STACK: return stack_;
    case Id::FLAPPY: return flappy_;
    case Id::DINO: return dino_;
    case Id::REACTION: return reaction_;
    case Id::MAZE: return maze_;
    case Id::BALANCE: return balance_;
    default: return catch_;
  }
}

static int sideOf(Id g) { return info(g).landscape ? side_ : 0; }
static int rotation() { return info(cur_).landscape ? (side_ > 0 ? 1 : 3) : (flipped_ ? 2 : 0); }

static void drawGame(const draw::Ctx& c) {
  switch (cur_) {
    case Id::STACK: draw::stack(stack_, c); break;
    case Id::FLAPPY: draw::flappy(flappy_, c); break;
    case Id::DINO: draw::dino(dino_, c); break;
    case Id::REACTION: draw::reaction(reaction_, c); break;
    case Id::MAZE: draw::maze(maze_, c); break;
    case Id::BALANCE: draw::balance(balance_, c); break;
    default: draw::catcher(catch_, c); break;
  }
}

static void go(Scr s) {
  scr_ = s;
  scrAt_ = millis();
  dirty_ = true;
}

static void reset() {
  const uint32_t seed = esp_random();
  switch (cur_) {
    case Id::STACK: stack_.reset(seed); break;
    case Id::FLAPPY: flappy_.reset(seed); break;
    case Id::DINO: dino_.reset(seed); break;
    case Id::REACTION: reaction_.reset(seed); break;
    case Id::MAZE: maze_.reset(seed); break;
    case Id::BALANCE: balance_.reset(seed); break;
    default: catch_.reset(seed); break;
  }
  newBest_ = failed_ = reported_ = false;
  final_ = 0;
}

static void start() {
  reset();
  if (info(cur_).tilt) {
    tilt_.begin(rotation());
    go(Scr::CALIB);
  } else {
    if (cur_ == Id::FLAPPY) flappy_.flap();  // the press that starts it is the first flap
    go(Scr::PLAY);
  }
  last_ = millis();
}

// The game is over (or quit / restarted with points): the best, the agent.
static void report() {
  if (reported_) return;
  reported_ = true;
  Base& g = game();
  final_ = g.score;
  failed_ = cur_ == Id::REACTION && reaction_.failed();
  if (final_ <= 0 && !g.over) return;  // quit before anything happened
  const int i = (int)cur_;
  newBest_ = better(cur_, final_, bests_[i]);
  if (newBest_) {
    bests_[i] = final_;
    prefs.putLong(info(cur_).id, final_);
  }
  Log.printf("games: %s %ld%s (best %ld)\n", info(cur_).id, final_, newBest_ ? " NEW BEST" : "", bests_[i]);
  if (buddy::agentLinked())
    buddy::sendLine(String("{\"src\":\"agent\",\"evt\":\"game\",\"id\":\"") + info(cur_).id + "\",\"score\":" + final_ +
                    ",\"best\":" + bests_[i] + ",\"new_best\":" + (newBest_ ? "true" : "false") + "}");
}

static void finish() {
  report();
  go(Scr::OVER);
  if (newBest_) {
    ui::useSide(sideOf(cur_));
    draw::overConfetti();
    sound::effect("g-best");
  } else {
    sound::effect(failed_ ? "g-hit" : "g-over");
  }
}

static void sounds(uint16_t fx) {
  if (fx & FX_OVER) return;  // finish() plays the end
  if (fx & FX_LEVEL) sound::effect("g-level");
  else if (fx & FX_PERFECT) sound::effect("g-perfect");
  else if (fx & FX_HIT) sound::effect("g-hit");
  else if (fx & FX_BONUS) sound::effect("g-bonus");
  else if (fx & FX_SCORE) sound::effect("g-score");
  else if (fx & FX_GO) sound::effect("g-go");
  else if (fx & FX_ACTION) sound::effect(cur_ == Id::FLAPPY ? "g-flap" : "g-tap");
  else if (fx & FX_WALL) sound::effect("g-tap");
}

static void press(const Input& in) {
  switch (cur_) {
    case Id::STACK: stack_.drop(); break;
    case Id::FLAPPY: flappy_.flap(); break;
    case Id::DINO: dino_.press(); break;
    case Id::REACTION: {
      const uint32_t late = millis() - in.aChangedAt;
      reaction_.press(late < 100 ? late : 0);
      dirty_ = true;
      break;
    }
    default: break;
  }
}

static void step(uint32_t ms) {
  tilt_.feed(rawX_, rawY_, ms / 1000.0f);
  switch (cur_) {
    case Id::STACK: stack_.step(ms); break;
    case Id::FLAPPY: flappy_.step(ms); break;
    case Id::DINO: dino_.step(ms); break;
    case Id::REACTION: {
      const auto was = reaction_.phase();
      reaction_.step(ms);
      if (reaction_.phase() != was) dirty_ = true;  // green: at once
      break;
    }
    case Id::MAZE: maze_.step(ms, tilt_.x(), tilt_.y()); break;
    case Id::BALANCE: balance_.step(ms, tilt_.x()); break;
    default: catch_.step(ms, tilt_.x()); break;
  }
}

// --- Open / close ------------------------------------------------------------------------------

void open(bool flipped, int side) {
  load();
  open_ = true;
  flipped_ = flipped;
  side_ = side >= 0 ? 1 : -1;
  waitA_ = waitB_ = true;  // the press that opened them
  lastInput_ = last_ = millis();
  cpuWas_ = getCpuFrequencyMhz();
  setCpuFrequencyMhz(GAME_CPU_MHZ);
  sound::keepOn(true);
  go(Scr::PICKER);
  Log.println("games: open");
}

void close() {
  if (!open_) return;
  if (scr_ == Scr::PLAY || scr_ == Scr::PAUSE) report();
  open_ = false;
  draw::release();
  duo::end();
  sound::keepOn(false);
  setCpuFrequencyMhz(cpuWas_);
  Log.println("games: closed");
}

bool active() { return open_; }

// --- The loop ------------------------------------------------------------------------------------

bool loop(const Input& raw) {
  if (!open_) return false;
  Input in = raw;
  const uint32_t now = millis();
  uint32_t dt = now - last_;
  last_ = now;
  rawX_ = in.ax;
  rawY_ = in.ay;
  if (waitA_) {
    if (!in.aHeld) waitA_ = false;
    in.aDown = in.aUp = false;
  }
  if (waitB_) {
    if (!in.bHeld) waitB_ = false;
    in.bClick = in.bHold = false;
  }
  if (in.aDown || in.bClick || in.bHold) {
    lastInput_ = now;
    dirty_ = true;
  }
  ui::keepAwake();
  if (dt > GAP_PAUSE_MS && scr_ == Scr::PLAY) {  // something else had the screen
    pauseSel_ = 0;
    go(Scr::PAUSE);
    dt = 0;
  }
  if (scr_ != Scr::PLAY && scr_ != Scr::CALIB && now - lastInput_ > IDLE_CLOSE_MS) {
    close();
    return false;
  }

  switch (scr_) {
    case Scr::PICKER:
      if (in.aDown) { sel_ = (sel_ + 1) % COUNT; sound::effect("g-tap"); }
      if (in.bClick) { cur_ = (Id)sel_; go(Scr::READY); }
      if (in.bHold) { close(); return false; }
      break;
    case Scr::READY:
      if (in.aDown) start();
      else if (in.bClick || in.bHold) { waitB_ = in.bHold; go(Scr::PICKER); }
      break;
    case Scr::CALIB: {
      const uint32_t into = now - scrAt_;
      if (into >= CALIB_SAMPLE_FROM) tilt_.sample(in.ax, in.ay);
      if (in.bHold || in.bClick) { waitB_ = in.bHold; go(Scr::PICKER); break; }
      if (into >= CALIB_MS) {
        tilt_.calibrate();
        sound::effect("g-go");
        go(Scr::PLAY);
      }
      break;
    }
    case Scr::PLAY:
      if (in.bClick || in.bHold) {
        waitB_ = in.bHold;
        pauseSel_ = 0;
        go(Scr::PAUSE);
        break;
      }
      if (in.aDown) press(in);
      if (in.aUp && cur_ == Id::DINO) dino_.release();
      step(dt > 50 ? 50 : dt);
      sounds(game().takeFx());
      if (game().over) finish();
      break;
    case Scr::PAUSE:
      if (in.aDown) pauseSel_ = (pauseSel_ + 1) % draw::PAUSE_ITEMS;
      if (in.bHold) { waitB_ = true; go(Scr::PLAY); break; }
      if (in.bClick) {
        if (pauseSel_ == 0) go(Scr::PLAY);
        else {
          report();  // a game left with points counts
          if (pauseSel_ == 1) start();
          else go(Scr::PICKER);
        }
      }
      break;
    case Scr::OVER:
      if (now - scrAt_ < OVER_LOCK_MS) break;
      if (in.aDown) start();
      else if (in.bClick || in.bHold) { waitB_ = in.bHold; go(Scr::PICKER); }
      break;
  }

  // Draw: the game at ~40 fps, the rest when something changes (and their small animations)
  uint32_t every = 0;
  switch (scr_) {
    case Scr::PLAY: every = cur_ == Id::REACTION ? 100 : 25; break;
    case Scr::CALIB: every = 50; break;
    case Scr::OVER: every = newBest_ ? 40 : 250; break;
    case Scr::READY: every = 250; break;
    default: every = 0; break;
  }
  if (!dirty_ && !(every && now - lastFrame_ >= every)) return true;
  dirty_ = false;
  lastFrame_ = now;
  const uint32_t t0 = micros();
  draw::Ctx c;
  c.best = bests_[(int)cur_];
  c.acc = faces::world().acc;
  c.now = now;
  switch (scr_) {
    case Scr::PICKER:
      ui::useSide(0);
      draw::picker(sel_, bests_, now);
      break;
    case Scr::READY:
      ui::useSide(sideOf(cur_));
      draw::ready(cur_, c);
      break;
    case Scr::CALIB: {
      ui::useSide(sideOf(cur_));
      float tx, ty;
      screenTilt(rotation(), in.ax, in.ay, tx, ty);
      draw::calibrate(cur_, CALIB_MS - (now - scrAt_ < CALIB_MS ? now - scrAt_ : CALIB_MS), tx, ty, c);
      break;
    }
    case Scr::PLAY:
      ui::useSide(sideOf(cur_));
      drawGame(c);
      break;
    case Scr::PAUSE:
      ui::useSide(sideOf(cur_));
      drawGame(c);
      draw::pause(pauseSel_);
      break;
    case Scr::OVER:
      ui::useSide(sideOf(cur_));
      drawGame(c);
      draw::over(cur_, final_, bests_[(int)cur_], newBest_, failed_, now - scrAt_);
      break;
  }
  draw::present();
  if (scr_ == Scr::PLAY) {  // frame cost, in the log every 10 s of play (tuning on the stick)
    perfUs_ += micros() - t0;
    perfFrames_++;
    if (now - perfAt_ > 10000) {
      if (perfAt_ && perfFrames_)
        Log.printf("games: %s %lu fps, %lu us a frame\n", info(cur_).id,
                   (unsigned long)(perfFrames_ * 1000 / (now - perfAt_)), (unsigned long)(perfUs_ / perfFrames_));
      perfAt_ = now;
      perfFrames_ = perfUs_ = 0;
    }
  }
  return true;
}

}  // namespace games
