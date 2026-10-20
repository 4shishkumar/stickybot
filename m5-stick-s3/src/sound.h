#pragma once
#include <Arduino.h>
#include <ArduinoJson.h>

// Short sound cues on the stick's speaker. Each event plays a tune from a
// small built-in library; which one is chosen in the Mac agent's dashboard
// (Sounds tab) and saved on the stick. The speaker shares the audio chip with
// the microphone, so a cue waits until the mic is off (dropped after 2.5 s).
namespace sound {

enum class Cue {
  SENT,       // B released: request on its way
  ANSWER,     // the agent's answer arrived
  APPROVAL,   // Claude Code needs a yes/no
  APPROVED, DENIED,
  ERROR,      // didn't catch that / something failed
  CONNECTED, DISCONNECTED,  // the Mac
  UNLOCK,     // Mac password typed
  CHARGING, LOW_BATTERY,
  TEST,
  LISTEN,     // hold B registered: speak now (the mic pauses for it)
};

void begin();
void loop();
void play(Cue c, bool force = false);            // force: play even with sounds turned off
bool play(const String& name);                   // an event ("answer") or a tune ("ping-3"); always plays
bool setTune(const String& event, const String& tune);  // tune "off" = silent; saved
void describe(JsonObject out);                   // {tunes:[...], map:{event: tune}, on, volume}
bool busy();
void playListen();  // the "speak now" cue: pauses the mic, plays, resumes it

// Sound clips sent by the Mac (Kenney sound packs, spoken replies...), as
// packets on the "audio in" characteristic, all in order (see PROTOCOL.md):
//   [0][samples u32][rate u16]   start a clip (drops an unfinished one)
//   [1][IMA-ADPCM frame]         same frame format as the mic's
//   [2]                          end: play it
void clipPacket(const uint8_t* data, size_t len);  // from the Bluetooth task: queued

}  // namespace sound
