// Screen drawing and screen sleep.
#pragma once
#include "combo.h"

namespace ui {

struct Status {
  Combo   combo;
  uint8_t slot;
  int     bt;        // 0 connected, 1 waiting, 2 Bluetooth off
  int     battery;   // 0-100, -1 unknown
  bool    charging;
  String  eta;
  bool operator!=(const Status& o) const {
    return combo.mods != o.combo.mods || combo.key != o.combo.key || slot != o.slot ||
           bt != o.bt || battery != o.battery || charging != o.charging || eta != o.eta;
  }
};

// Status screen pages, switched with a flick of the wrist. Waking the screen
// always starts on Home.
enum class Page { HOME, AGENT, STATUS, COUNT };

struct Dash {
  // home
  Combo    combo;
  uint8_t  slot;
  int      bt;           // 0 connected, 1 waiting, 2 Bluetooth off
  String   name;
  int      battery;      // 0-100, -1 unknown
  bool     charging;
  String   batLine;      // "4.02 V · charging"
  String   clock;        // "3:05" (or "15:05" with the 24-hour setting) once the Mac agent has sent the time, else ""
  String   ampm;         // "AM" / "PM" ("" with the 24-hour setting)
  bool     clock24 = false;  // the 12 / 24-hour setting
  // Mac agent (+ the Claude desktop app, if the stick is named "Claude...")
  bool     agentLinked;
  String   agentHeard, agentText;
  String   date;         // "Tue 20 Oct", "" until the agent has sent the time
  String   longDate;     // "Tuesday 20 October" (dock clock)
  int      minute = -1;  // minutes since local midnight (the faces' sky), -1 until the agent has sent the time
  int      second = -1;  // 0-59 along with minute (the Analog face's second hand)
  int      wday = -1, mday = 0, mon = 0;  // the date's parts (wday 0 Sunday, mon 0 January), wday -1 until then
  // bottom card on the home screen (from the agent)
  String   cardKind, cardA, cardB;
  bool     cardUrgent;
  int      cardIndex, cardCount;  // position in the agent's rotation (source dots)
  // weather (from the agent)
  bool     wxValid;
  int      wxTemp, wxHi, wxLo;
  String   wxIcon, wxCond;
  bool     wxDay;
  bool     linked;
  int      sessions, running, waiting;
  long     tokens;
  String   claudeMsg;
  // system
  String   wifi, fw;
  uint32_t uptimeS, heap;
  int      chargerRestarts;
};

struct MenuView {
  const char* title;
  String      lines[40];  // the menu has 34 items (2.5.0); menu.cpp static_asserts they fit
  int         count;
  int         selected;
  const char* hint;
};

void begin();
void setFlipped(bool flipped);
// Auto-rotate: flipped = the portrait way up; side 0 = portrait, +1 / -1 = landscape
// (rotation 1, button A on the right / rotation 3, A on the left) for the screens that
// have a landscape layout. Takes effect at the next draw.
void setOrientation(bool flipped, int side);
// Games (2.5.0): turn to this side now (0 portrait, +1 / -1 landscape) and draw there; the
// screens with a landscape layout use it too until the next setOrientation().
void useSide(int side);
void setBrightness(uint8_t pct);
// For a screen drawn outside ui.cpp with a landscape layout (the remote modes, air_ui.cpp):
// turns to the held side (setOrientation) and undims; true = draw the landscape layout.
bool turnToHeldSide();

// Screen power. The screen is off by default to save battery: on for a while
// after boot, then only when woken (button B, menu, pairing, web settings).
bool wake(uint32_t forMs = 20000);  // returns true if it was asleep
void keepAwake();                   // reset the idle timer if awake
void tick(bool allowSleep);         // turns the screen off when its time is up
void off();
bool awake();

void setLevel(int peak);
void setTilt(float ax, float ay);   // accelerometer (g), for the eyes following the tilt            // mic level for the live screens (call ~25x/s)

void drawPage(Page p, const Dash& d);
void drawFaceDemo(const Dash& d, const String& caption);  // a face's demo frame (faces/demo.cpp), with a caption
void drawHolding(const Combo& c, bool usbMic, bool wirelessMic = false);  // A held: dictation
void drawMenu(const MenuView& m);
void drawInfo(const char* title, const String lines[], int count, const char* hint);
void drawMessage(const char* title, const char* sub, uint16_t bg);
void drawUpdate(int pct, const String& version, const char* line);  // firmware update over Bluetooth
void drawPairing(uint32_t code);
void drawPrompt(const String& tool, const String& hint, const String& from);
// A question from a Claude session (1.7.2): the question, then A / B with their labels
void drawAsk(const String& question, const String& from, const String& a, const String& b);
void drawTalk(bool linked);  // B held: talking to the Mac agent
void drawAgent(const String& state, const String& heard, const String& text);
void drawMood(int mood, const String& label);  // demo: 0 idle, 1 listen, 2 think, 3 work, 4 done, 5 error, 6 ask, 7 offline, 8 party
constexpr int MOOD_COUNT = 9;

// Home: the bottom card cross-fades between items (now and then the eyes peek over it).
bool homeAnimating();   // a fade / peek is running: redraw ~25x/s
void forceHomePeek();   // the next change peeks (demo)

// Dock: a desk clock while charging. side: 0 portrait (stacked clock), +1/-1 lying on a
// side (landscape full-width clock). night: dim warm colours at ~5% brightness.
void drawDock(const Dash& d, int side, bool night);
void dockEnd();         // back to portrait + normal brightness (safe to call any time)

// Full-screen event (see events.h): level 0 celebrate, 1 nice, 2 headsup;
// ms = time since it started (drives the animation; redraw ~25x/s).
void drawEvent(uint8_t level, const String& title, const String& sub, const String& icon, uint32_t ms);

}  // namespace ui
