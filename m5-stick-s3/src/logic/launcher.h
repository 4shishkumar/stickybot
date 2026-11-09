// The Launcher (firmware 2.5.1): triple-press B opens a grid of app tiles (Games, Remote,
// Air mouse, Presenter, Media, Faces, Quick settings, Settings). The pure parts, tested on
// the Mac (test_launcher): the tiles, the dashboard's order / visibility string, the grid
// and its pages, and the quick settings' steps. The screen is src/launcher_session.cpp (session) and
// src/launcher_ui.cpp (drawing).
//
// Order and visibility: an ordered list of the tiles shown. Quick settings and Settings are
// always there (a list without them gets them appended at the end), so the stick can always
// reach its settings. Two spellings:
//   ids      "games,remote,air,presenter,media,faces,quick,settings"  (@launcher tiles=)
//   letters  "grapmfqs"  (one letter per tile: NVS and @state, to keep @state short)
#pragma once
#include <stddef.h>
#include <stdint.h>

namespace launcher {

enum class Tile : uint8_t { GAMES, REMOTE, AIR, PRESENTER, MEDIA, FACES, QUICK, SETTINGS, COUNT };
constexpr int TILE_COUNT = (int)Tile::COUNT;

struct TileInfo {
  const char* id;      // "games" (@launcher tiles=)
  char        letter;  // 'g' (NVS, @state)
  const char* label;   // on the tile: "Games"
  const char* title;   // the full name: "Quick settings"
  const char* about;   // one line under the grid
  bool        fixed;   // always shown (Quick settings, Settings)
};
const TileInfo& info(Tile t);
// An id ("games") or a letter ("g"), n characters of s. false if unknown.
bool fromName(const char* s, size_t n, Tile& out);

struct Layout {
  Tile    tiles[TILE_COUNT];
  uint8_t n = 0;
  int  indexOf(Tile t) const;  // -1 if hidden
  bool operator==(const Layout& o) const;
};
Layout defaults();  // all eight, in the order above

enum class ParseError : uint8_t { NONE, EMPTY, UNKNOWN, DUPLICATE };
// "games,air,quick" (ids or letters, comma separated, spaces ignored). Missing fixed tiles are
// appended. On an error `out` is left alone; `bad` (if given) gets the offending name.
ParseError parse(const char* s, Layout& out, char* bad = nullptr, size_t badN = 0);
// "graq..." (letters only, no commas): what NVS and @state hold. Same rules.
ParseError parseLetters(const char* s, Layout& out);
void letters(const Layout& l, char* out, size_t n);  // n >= TILE_COUNT + 1
void ids(const Layout& l, char* out, size_t n);      // n >= 72 is always enough
const char* errorText(ParseError e);                 // "unknown tile" ...

// --- The grid --------------------------------------------------------------------------------
// Portrait 2 x 3, landscape 3 x 2: six tiles a page either way, so a tile stays on its page
// when the stick turns.
struct Grid {
  int cols, rows;
  int perPage() const { return cols * rows; }
};
Grid gridFor(bool wide);
struct Cell {
  int page, col, row;
};
Cell cellOf(int i, const Grid& g);
int  pages(int n, int perPage);       // at least 1
int  next(int sel, int n);            // A: the next tile, round to the first
int  prev(int sel, int n);
// Tilt / flick to the next (+1) or previous (-1) page, round; keeps the place on the page
// (or the last tile of a shorter page).
int  turnPage(int sel, int n, int perPage, int dir);
// The tile to start on: the last one opened, if it is still shown; else the first.
int  startAt(const Layout& l, Tile last);

// --- Quick settings ---------------------------------------------------------------------------
// Two sliders, five toggles, then "All settings" (opens Settings), in A's order.
enum class Quick : uint8_t { BRIGHTNESS, VOLUME, SOUND, WMIC, CLOCK24, ROTATE, GESTURES, ALL, COUNT };
constexpr int QUICK_COUNT = (int)Quick::COUNT;
uint8_t nextBrightness(uint8_t pct);  // 25 -> 50 -> 75 -> 100 -> 25 (10-100 from the dashboard: the next step up)
uint8_t nextVolume(uint8_t v);        // 64 -> 128 -> 192 -> 255 -> 64 (as the menu)
uint8_t volumePct(uint8_t v);         // 0-255 -> 0-100
// The gestures tile: on if any of lift / face-down / shake is on; a press turns all three off,
// or (all off) all three on.
bool gesturesOn(bool lift, bool faceDown, bool shake);
void toggleGestures(bool& lift, bool& faceDown, bool& shake);

}  // namespace launcher
