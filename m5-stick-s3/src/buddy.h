// Mac link — a Nordic UART service next to the keyboard, on the same
// Bluetooth connection. Used by:
//   - the Mac agent (tools/mac-agent): hold B to talk, status back on screen;
//     audio goes over an extra notify characteristic (see mic.cpp)
//   - the Claude Code hook (tools/claude-hook): permission prompts, A/B
//   - Claude's "Hardware Buddy" protocol, which the hook reuses:
//     https://github.com/anthropics/claude-desktop-buddy/blob/main/REFERENCE.md
//     (the Claude desktop app itself only lists devices named "Claude...", so
//     it only finds the stick if you give it such a name)
// Writes need an encrypted (paired) link, so only your own Mac can use it.
#pragma once
#include <Arduino.h>

namespace buddy {

// Prompts come from two sources: the Claude desktop app (its heartbeat
// snapshots) and the Claude Code terminal hook (messages tagged "src":"cli").
// Each source can only set or clear its own prompt.
struct Prompt {
  String id, tool, hint;
  String from;  // "" = desktop app, otherwise e.g. the terminal project name
};

void attach();          // call after the keyboard has started, before advertising
void loop();
bool linked();          // heard from the desktop app in the last 30 s
int  running();
int  waiting();
int  total();          // open sessions
long tokensToday();
String lastMessage();  // latest status line from the desktop app
bool hasPrompt();
const Prompt& prompt();
void decide(bool approve);  // answer the current prompt

// --- Mac agent ---------------------------------------------------------------
struct Agent {
  String state;  // listening / thinking / working / done / error ("" = none yet)
  String heard;  // what the agent understood
  String text;   // its reply or progress line
};
bool   agentLinked();              // agent heartbeat seen recently
const  Agent& agent();
const  Agent& lastAnswer();        // the last successful exchange ("done"), for the Agent page
uint32_t agentUpdated();           // millis() of the last status, 0 = none
void   agentTalkEvent(bool start); // tell the agent a talk press started / ended
void   agentCancel();              // dismissed on the stick: the agent drops the request
size_t audioPayloadMax();          // largest audio frame one notification can carry
bool   sendAudio(const uint8_t* data, size_t len);  // safe from any task
void   eventShown(const String& id);  // ack: {"src":"agent","evt":"event_shown","id":...}
void   fastLink();                 // ask the Mac for a quick connection interval (audio coming)
String linkInfo();                 // "MTU 256, 15 ms" (for logs)
String clock();                    // local 12-hour time "3:25" from the agent's clock, "" until it has sent it
String ampm();                     // "AM" / "PM"
int    hour();                     // local hour 0-23, -1 until the agent has sent the time
String longDate();                 // "Tuesday 20 October", "" until then
String date();                     // local "Tue 20 Oct", "" until then
// The card at the bottom of the home screen, chosen by the agent
struct HomeCard {
  String kind;    // approval | reply | claude | fact | news | history | hello | meeting
  String a, b;    // first line, second line
  bool   urgent = false;
  int    index = 0, count = 0;  // position in the agent's rotation (for the source dots)
};
HomeCard homeCard();               // kind "" when the agent is offline

struct Weather {
  bool   valid = false;
  int    temp = 0, hi = 0, lo = 0;  // °C
  String icon;                      // clear | partly | cloudy | fog | rain | snow | storm
  String cond;                      // "light rain"
  bool   day = true;
  String place;
  uint32_t at = 0;                  // millis() when received
};
const Weather& weather();           // from the agent (Open-Meteo), stale after 2 h


}  // namespace buddy
