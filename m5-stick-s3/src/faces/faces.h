// Faces: the Home screen's looks, one picked in the settings ("face", default "classic").
//
//   classic      the clock, weather and the agent's card (Home as before 2.0)
//   stickyworld  StickyBot in a pixel world: sky by the time and weather, a city that grows
//                with your merged PRs, moods from your CI / Claude sessions / what waits for you
//   aquarium     a fish per running Claude Code session, PRs as bubbles, tilt the water
//   bigtime      just the time, as big as it gets (minimal)
//   words        the time in words, "twenty-five to eleven" (minimal)
//   analog       a clean dial with a ticking second hand, the date and the weather
//   dashboard    time, weather, battery, what waits, Claude, CI and the card in one grid
//   space        StickyBot over a ringed planet: sessions orbit it, merged PRs launch rockets
//   lava         a lava lamp: a blob per Claude session, its colour says how things are
//
// Adding a face: one file in src/faces/ that subclasses Face and defines `Face& myFace()`
// (see classic.cpp, the smallest), plus one X(myFace) in FACES in faces.cpp (and, for a
// clock with no world to show, its id in demo.cpp's QUIET list: the short demo).
//
// Every face keeps the time and anything urgent on screen (common.h: urgent(), timeBar()).
// The other screens (prompts, events, the agent, the dock clock) are not faces: they
// interrupt Home as before.
#pragma once
#include <Arduino.h>
#include <ArduinoJson.h>
#include "../ui.h"
#include "../logic/world.h"

namespace faces {

class Face {
 public:
  virtual ~Face() = default;
  virtual const char* id() const = 0;     // saved in the settings, used by the protocol
  virtual const char* title() const = 0;  // "StickyWorld"
  virtual const char* about() const = 0;  // one line for the pickers
  // Draws the whole Home screen on ui::canvas (ui.cpp pushes it). wide: landscape 240 x 135.
  virtual void draw(const ui::Dash& d, bool wide, uint32_t now) = 0;
  // Something happened (world::Kick): a PR merged, CI failed, a toy (shake, flick...).
  virtual void kick(world::Kick k, uint32_t now) {}
  // How often Home wants a new picture (ms), 0 = only when something changes.
  virtual uint32_t frameMs(bool onBattery, uint32_t idleMs) const { return 0; }
  // Forget the animation state (after a demo, so its fake fish don't all leap out).
  virtual void reset() {}
};

// --- Registry ---------------------------------------------------------------------
int         count();
Face&       at(int i);
int         find(const String& id);   // -1 if unknown
Face&       active();
void        select(const String& id); // unknown -> classic (the caller saves the setting)

// --- Home --------------------------------------------------------------------------
void     drawHome(const ui::Dash& d, bool wide);           // the active face (or the demo's)
uint32_t frameMs(bool onBattery, uint32_t idleMs);         // the active face's wish
void     toy(world::Kick k);                               // SHAKE / FLICK / UPSIDE_DOWN / LIFT

// --- The world (from the agent: PROTOCOL.md "Faces") ------------------------------------
void begin();                                  // loads the cached world from NVS
const world::Snapshot& world();                // the demo's while one runs
void onWorld(JsonObjectConst w);               // {"src":"agent","world":{...}} or in the hello
void onEventKind(const char* kind);            // an event's "kind" (events.py), for reactions
String json();                                 // [{"id","title","about"},...] for @state

// --- Demo / preview (serial "screens faces", protocol "@face demo=<id>") -------------
bool demoStart(const String& id);   // "all" or "" = every face; false if unknown
void demoStop();
bool demoActive();
// The next demo frame: fills d (from base, with the demo's fake data), the side to show
// (0 portrait, 1 landscape) and a caption. false once the demo is over.
bool demoFrame(const ui::Dash& base, ui::Dash& d, int& side, String& caption);

}  // namespace faces
