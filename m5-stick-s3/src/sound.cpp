#include "sound.h"
#include <M5Unified.h>
#include <Preferences.h>
#include "mic.h"
#include "board.h"
#include "settings.h"
#include "netlog.h"
#include "sounds_clips.h"
#include "logic/clip.h"
#include "buddy.h"

namespace sound {

// A small library of tunes; each event plays one of them (chosen in the Mac
// agent's dashboard, saved here so it works without the agent).
struct Note { uint16_t hz, ms; };  // hz 0 = rest
struct Tune { const char* name; const Note* notes; uint8_t n; };

constexpr uint32_t WAIT_FOR_MIC_MS = 2500;
constexpr uint8_t  DAC_GAIN = 0xD7;  // audio chip DAC: +12 dB (0xBF = 0 dB, 0.5 dB steps)
// A square wave sounds louder than the library's sine on a tiny speaker
static const uint8_t SQUARE[16] = {230, 230, 230, 230, 230, 230, 230, 230, 26, 26, 26, 26, 26, 26, 26, 26};

#define N(...) static const Note __VA_ARGS__
N(BLIP2[])      = {{1568, 60}, {0, 35}, {2093, 70}};
N(CHIME_UP[])   = {{988, 90}, {0, 25}, {1319, 140}};
N(CHIME_DOWN[]) = {{1319, 90}, {0, 25}, {988, 140}};
N(PING2[])      = {{1760, 70}, {0, 80}, {1760, 70}};
N(PING3[])      = {{1760, 50}, {0, 60}, {1760, 50}, {0, 60}, {2093, 90}};
N(KNOCK[])      = {{880, 40}, {0, 90}, {880, 40}};
N(BELL[])       = {{2093, 40}, {1568, 220}};
N(RISE[])       = {{784, 60}, {1175, 110}};
N(FALL[])       = {{659, 70}, {440, 120}};
N(BUZZ[])       = {{659, 120}, {0, 60}, {523, 180}};
N(TRIAD_UP[])   = {{659, 50}, {880, 50}, {1319, 80}};
N(TRIAD_DOWN[]) = {{880, 50}, {659, 50}, {440, 90}};
N(LOCK[])       = {{3200, 6}, {0, 30}, {2600, 8}, {0, 45}, {1568, 70}, {2349, 180}};
N(STEPS_UP[])   = {{660, 60}, {880, 60}, {1320, 120}};
N(CHIRP[])      = {{1047, 50}, {1568, 90}};
N(WARN3[])      = {{1047, 120}, {0, 60}, {880, 120}, {0, 60}, {698, 220}};
N(POP[])        = {{2637, 25}};
N(LISTEN_UP[])  = {{1319, 45}, {1976, 75}};  // short and bright: "go ahead"
N(ARPEGGIO[])   = {{523, 80}, {659, 80}, {784, 80}, {1047, 160}};
N(FANFARE[])    = {{784, 90}, {0, 20}, {784, 90}, {0, 20}, {784, 90}, {1047, 300}, {0, 40}, {932, 120}, {1047, 360}};
// game effects (not in the dashboard's list of tunes)
N(G_TAP[])     = {{1568, 18}};
N(G_FLAP[])    = {{880, 22}, {1175, 26}};
N(G_SCORE[])   = {{1760, 35}};
N(G_PERFECT[]) = {{1319, 35}, {1760, 35}, {2349, 60}};
N(G_HIT[])     = {{220, 70}};
N(G_OVER[])    = {{523, 90}, {392, 90}, {262, 200}};
N(G_BEST[])    = {{784, 70}, {988, 70}, {1175, 70}, {1568, 220}};
N(G_GO[])      = {{1976, 60}};
N(G_LEVEL[])   = {{1047, 60}, {1319, 60}, {1568, 60}, {2093, 120}};
N(G_BONUS[])   = {{2093, 30}, {2637, 50}};
#undef N

#define T(name, x) Tune{name, x, (uint8_t)(sizeof(x) / sizeof(x[0]))}
static const Tune TUNES[] = {
  T("blip", BLIP2), T("chime-up", CHIME_UP), T("chime-down", CHIME_DOWN), T("ping", PING2), T("ping-3", PING3),
  T("knock", KNOCK), T("bell", BELL), T("rise", RISE), T("fall", FALL), T("buzz", BUZZ), T("triad-up", TRIAD_UP),
  T("triad-down", TRIAD_DOWN), T("lock", LOCK), T("steps-up", STEPS_UP), T("chirp", CHIRP), T("warn", WARN3),
  T("pop", POP), T("arpeggio", ARPEGGIO), T("fanfare", FANFARE), T("listen-up", LISTEN_UP),
};
static const Tune GAME_TUNES[] = {
  T("g-tap", G_TAP), T("g-flap", G_FLAP), T("g-score", G_SCORE), T("g-perfect", G_PERFECT), T("g-hit", G_HIT),
  T("g-over", G_OVER), T("g-best", G_BEST), T("g-go", G_GO), T("g-level", G_LEVEL), T("g-bonus", G_BONUS),
};
#undef T
constexpr size_t TUNE_COUNT = sizeof(TUNES) / sizeof(TUNES[0]);

// Events, their names (as used by the dashboard / serial) and default tunes
struct Event { Cue cue; const char* name; const char* def; };
static const Event EVENTS[] = {
  // defaults: Kenney clips built into flash (tools/make_sounds.py), beeps where chosen
  {Cue::SENT, "sent", "drop_002"},             {Cue::ANSWER, "answer", "confirmation_001"},
  {Cue::APPROVAL, "approval", "ping-3"},       {Cue::APPROVED, "approved", "confirmation_002"},
  {Cue::DENIED, "denied", "back_002"},         {Cue::ERROR, "error", "error_004"},
  {Cue::CONNECTED, "connected", "open_002"},   {Cue::DISCONNECTED, "disconnected", "close_003"},
  {Cue::UNLOCK, "unlock", "unlock_click"},     {Cue::CHARGING, "charging", "pluck_001"},
  {Cue::LOW_BATTERY, "lowbattery", "error_008"}, {Cue::TEST, "test", "confirmation_001"},
  {Cue::LISTEN, "listen", "listen-up"},
};
constexpr size_t EVENT_COUNT = sizeof(EVENTS) / sizeof(EVENTS[0]);

static Preferences prefs;  // namespace "sounds": event name -> tune name ("off" = silent)
static String choice[EVENT_COUNT];

static const Tune* tuneByName(const String& n) {
  for (auto& t : TUNES)
    if (n == t.name) return &t;
  return nullptr;
}

static const BuiltinClip* clipByName(const String& n) {
  for (size_t i = 0; i < BUILTIN_CLIP_COUNT; i++)
    if (n == BUILTIN_CLIPS[i].name) return &BUILTIN_CLIPS[i];
  return nullptr;
}

static int eventIndex(const String& n) {
  for (size_t i = 0; i < EVENT_COUNT; i++)
    if (n == EVENTS[i].name) return i;
  return -1;
}

// --- Clips from the Mac ---------------------------------------------------------

constexpr uint32_t MAX_CLIP_SAMPLES = 16000 * 30;  // 30 s at 16 kHz (960 KB of PSRAM)

static portMUX_TYPE clipMux = portMUX_INITIALIZER_UNLOCKED;
static std::string clipInbox;            // [len u8][frame] ... filled by the Bluetooth task
static int16_t*    clipBuf = nullptr;    // decoded PCM (PSRAM)
static clip::Assembler clipAsm;          // places frames by sequence number (logic/clip.cpp)
static uint32_t    clipSamples = 0;
static uint16_t    clipRate = 16000;
static bool        clipPlaying = false, clipReady = false;
static uint32_t    clipEndsAt = 0;  // no earlier than this (isPlaying() can be false right after starting)

static void clipBegin(uint32_t samples, uint16_t rate) {
  if (clipPlaying) M5.Speaker.stop();
  clipPlaying = clipReady = false;
  free(clipBuf);
  clipSamples = min(samples, MAX_CLIP_SAMPLES);
  clipBuf = clipSamples ? (int16_t*)ps_malloc(clipSamples * 2) : nullptr;
  if (clipBuf) memset(clipBuf, 0, clipSamples * 2);
  clipAsm.reset(clipBuf, clipSamples);
  clipRate = rate ? rate : 16000;
}

void clipPacket(const uint8_t* data, size_t len) {
  if (!len || len > 250) return;
  portENTER_CRITICAL(&clipMux);
  if (clipInbox.size() < 64 * 1024) {
    clipInbox.push_back((char)len);
    clipInbox.append((const char*)data, len);
  }
  portEXIT_CRITICAL(&clipMux);
}

static void decodeFrame(const uint8_t* f, size_t len) { clipAsm.add(f, len); }

static void startBuiltin(const BuiltinClip* c) {
  clipBegin(c->samples, 16000);
  for (size_t i = 0; i < c->bytes;) {
    size_t len = c->data[i++];
    decodeFrame(c->data + i, len);
    i += len;
  }
  if (clipBuf) clipReady = true;
}

// Handles queued packets in order: start / ADPCM frame / end (logic/clip.cpp).
struct ClipQueueSink : clip::Sink {
  void begin(uint32_t samples, uint16_t rate) override {
    buddy::fastLink();
    clipBegin(samples, rate);
  }
  void frame(const uint8_t* f, size_t len) override { decodeFrame(f, len); }
  void end() override {
    if (!clipBuf) return;
    clipReady = true;
    Log.printf("clip: %lu of %lu frames, %.1f s\n", (unsigned long)clipAsm.frames,
               (unsigned long)clipAsm.expectedFrames(), clipSamples / (float)clipRate);
  }
};

static void decodeClipFrames() {
  std::string chunk;
  portENTER_CRITICAL(&clipMux);
  chunk.swap(clipInbox);
  portEXIT_CRITICAL(&clipMux);
  if (chunk.empty()) return;
  ClipQueueSink sink;
  clip::parseQueue((const uint8_t*)chunk.data(), chunk.size(), sink);
}

static const Tune* cur = nullptr;
static uint8_t  step = 0;
static uint32_t stepEnd = 0, queuedAt = 0;
static bool     pending = false, speakerOn = false, resumeMic = false, keep_ = false;

static void amp(bool on) { board::speakerAmp(on); }  // the speaker amp's power

void begin() {
  amp(false);
  prefs.begin("sounds", false);
  for (size_t i = 0; i < EVENT_COUNT; i++) choice[i] = prefs.getString(EVENTS[i].name, EVENTS[i].def);
}

bool busy() { return pending || cur || clipReady || clipPlaying; }

static void start(const Tune* t) {
  if (!t) return;
  cur = t;
  step = 0;
  stepEnd = 0;
  pending = true;
  queuedAt = millis();
}

static void startBuiltin(const BuiltinClip* c);

// A tune or a built-in clip, by name ("off" / unknown -> nothing). Returns true if found.
static bool startNamed(const String& name) {
  if (const Tune* t = tuneByName(name)) { start(t); return true; }
  if (const BuiltinClip* c = clipByName(name)) { startBuiltin(c); return true; }
  return false;
}

void playListen() {
  if (!settings::soundOn()) return;  // no cue: don't interrupt the mic at all
  mic::suspend(true);
  resumeMic = true;
  play(Cue::LISTEN);
  if (!cur && !clipReady) {  // "off" chosen for this event: nothing to wait for
    mic::suspend(false);
    resumeMic = false;
  }
}

void play(Cue c, bool force) {
  if (!settings::soundOn() && !force && c != Cue::TEST) return;
  for (size_t i = 0; i < EVENT_COUNT; i++)
    if (EVENTS[i].cue == c) {
      startNamed(choice[i]);
      return;
    }
}

bool play(const String& name) {
  int e = eventIndex(name);
  if (e >= 0) {
    startNamed(choice[e]);  // asked for by name: always plays
    return true;
  }
  return startNamed(name);
}

static void speakerOff();

void effect(const char* name) {
  if (!settings::soundOn() || mic::live()) return;
  for (auto& t : GAME_TUNES)
    if (!strcmp(name, t.name)) {
      if (cur) M5.Speaker.stop();
      start(&t);
      return;
    }
}

void keepOn(bool on) {
  keep_ = on;
  if (!on && !busy()) speakerOff();
}

bool setTune(const String& event, const String& tune) {
  int e = eventIndex(event);
  if (e < 0 || (tune != "off" && !tuneByName(tune) && !clipByName(tune))) return false;
  choice[e] = tune;
  prefs.putString(EVENTS[e].name, tune);
  return true;
}

void describe(JsonObject out) {
  JsonArray tunes = out["tunes"].to<JsonArray>();
  for (size_t i = 0; i < BUILTIN_CLIP_COUNT; i++) tunes.add(BUILTIN_CLIPS[i].name);  // real sounds first
  for (auto& t : TUNES) tunes.add(t.name);
  JsonObject map = out["map"].to<JsonObject>();
  for (size_t i = 0; i < EVENT_COUNT; i++)
    if (EVENTS[i].cue != Cue::TEST) map[EVENTS[i].name] = choice[i];
  out["on"] = settings::soundOn();
  out["volume"] = settings::soundVolume();
}

static void speakerOff() {
  if (resumeMic) {  // the "speak now" cue is done: give the chip back to the mic
    resumeMic = false;
    mic::suspend(false);
  }
  if (!speakerOn) return;
  M5.Speaker.stop();
  if (keep_) return;  // a game: stays up for the next effect
  M5.Speaker.end();
  amp(false);  // the amp draws power even when silent
  speakerOn = false;
}

static bool speakerUp() {
  if (mic::live()) return false;  // the mic has the audio chip
  if (!speakerOn) {
    amp(true);
    speakerOn = M5.Speaker.begin();
    M5.Speaker.setVolume(settings::soundVolume());
    M5.In_I2C.writeRegister8(0x18, 0x32, DAC_GAIN, 400000);
    if (!speakerOn) Log.println("sound: speaker failed to start");
  }
  return speakerOn;
}

void loop() {
  decodeClipFrames();
  if (clipPlaying) {
    if (millis() < clipEndsAt || M5.Speaker.isPlaying()) return;
    clipPlaying = false;
    free(clipBuf);
    clipBuf = nullptr;
    if (!cur) speakerOff();
  }
  if (clipReady && !cur) {
    if (!speakerUp()) return;
    clipReady = false;
    if (clipAsm.filled) {
      M5.Speaker.playRaw(clipBuf, clipAsm.filled, clipRate, false, 1, -1, true);
      clipPlaying = true;
      clipEndsAt = millis() + (uint64_t)clipAsm.filled * 1000 / clipRate + 80;
    }
    return;
  }
  if (!cur) return;
  if (pending) {
    if (!speakerUp()) {
      if (millis() - queuedAt > WAIT_FOR_MIC_MS) { cur = nullptr; pending = false; speakerOff(); }
      return;
    }
    pending = false;
  }
  if (millis() < stepEnd) return;
  if (step >= cur->n) {
    cur = nullptr;
    speakerOff();
    return;
  }
  const Note& n = cur->notes[step++];
  if (n.hz) M5.Speaker.tone(n.hz, n.ms, -1, true, SQUARE, sizeof SQUARE);
  stepEnd = millis() + n.ms + 5;
}

}  // namespace sound
