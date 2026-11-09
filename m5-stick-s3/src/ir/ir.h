// The IR remote (firmware 2.5.0): learn the buttons of your TV / AC / soundbar remotes, keep
// them on the stick (store.h), send them from the stick's Remote screen or from the Mac.
//
//   USB / Bluetooth "@ir <op> ..."  (PROTOCOL.md, "IR remote"):
//     list [dev= page=] | get dev= btn= | send dev= btn= [repeat=] | send code= [repeat=]
//     learn dev= [btn=] [once=1] | listen [secs=] | cancel | put dev= btn= code= [fav=]
//     rm dev= [btn=] | rename dev= [btn=] to= | fav dev= btn= on= | diag
//   Bluetooth {"src":"agent","ir":{"learn":{...}}} etc. (the same ops as objects); the stick
//   answers learning / listening with {"src":"agent","evt":"ir","state":...} events.
#pragma once
#include <Arduino.h>
#include <ArduinoJson.h>
#include "../protocol.h"

namespace ir {

void   begin();
void   loop();
protocol::Result run(const protocol::Args& arg);   // "@ir" (op= ...)
void   control(JsonObjectConst o);                 // {"ir":{...}} from the agent
String stateJson();                                // "ir":{...} for @state

bool   learning();        // learning or listening (the receiver is on)
bool   sending();

// --- The Remote screen (screen.cpp): main.cpp hands it the buttons while it is open ---
struct Input {
  bool aDown, aUp, aHeld, bSingle, bDouble, bTriple, bHold;
  float tiltX;            // accelerometer x (g): tilting scrolls
};
void screenOpen();                     // the menu's "Remote"
bool screenActive();                   // open, or a learn from the Mac is running (it shows)
// Draws it and handles the input; false when it isn't showing (nothing used)
bool screenLoop(const Input& in);

// --- inside the IR module ---
const char* sendButton(const char* dev, const char* btn, int repeat, bool held);  // nullptr = sent
void   releaseHeld();
const char* startLearn(const char* dev, const char* btn, bool once);
void   cancelLearn(const char* by);
// What the learn screen shows
struct LearnView {
  bool   active = false, listening = false;
  int    step = 0;          // 1 first press, 2 again
  String dev, btn, last;    // last: "Didn't match", "Saved", ...
  uint32_t lastAt = 0;
  bool   ok = false;
};
const LearnView& learnView();

}  // namespace ir
