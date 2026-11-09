// The Launcher (firmware 2.5.1): triple-press B opens a grid of app tiles; the stick's
// "apps" (Games, Remote, Air mouse, Presenter, Media, Faces) and its settings (Quick
// settings, Settings) each have one.
//
//   Grid            A next tile (round all the pages), B open, hold B back to Home,
//                   flick the wrist left / right: the previous / next page
//   Quick settings  A next, B change (applied and saved at once), hold B back to the grid;
//                   "Settings" at the end opens the full settings menu
//   Faces           A next, B use it (saved) and show Home, hold B back to the grid
//
// The apps it opens (games, the IR remote, the remote modes) go back to Home when they end,
// as before; so do Settings' Save & exit / Cancel. Left alone for 60 s it closes (Home).
// The order and which tiles show come from the dashboard (`@launcher`, kept in NVS); Quick
// settings and Settings are always there. The pure logic: logic/launcher.*; the drawing:
// launcher_ui.*. main.cpp owns the mode and starts the apps (Action).
#pragma once
#include <Arduino.h>
#include "logic/launcher.h"
#include "protocol.h"

namespace launcher {

// What main.cpp does after an input
enum class Action : uint8_t { NONE, REDRAW, HOME, GAMES, REMOTE, AIR, PRESENTER, MEDIA, SETTINGS };

void   begin();                  // loads the layout and the last tile from NVS
void   open();                   // the grid, on the last tile opened
void   draw();                   // the screen it's on (main.cpp's render())
Action pressA();
Action clickB();                 // B released before the hold time
Action holdB();
Action flick(int dir);           // +1 right (next page), -1 left
bool   timedOut();               // 60 s without input
bool   onGrid();                 // (not Quick settings / Faces)

const Layout& layout();
// @launcher [tiles=<ids>] [reset=1]: sets (saved) and / or reports the tiles.
protocol::Result command(const protocol::Args& arg);
String stateJson();              // "launcher":"grapmfqs" for @state

}  // namespace launcher
