#include "launcher.h"
#include <string.h>

namespace launcher {

static const TileInfo TILES[TILE_COUNT] = {
    {"games", 'g', "Games", "Games", "Seven little games", false},
    {"remote", 'r', "Remote", "IR remote", "TV / AC: send, learn", false},
    {"air", 'a', "Air mouse", "Air mouse", "Point to move the pointer", false},
    {"presenter", 'p', "Presenter", "Presenter", "Slides + a talk timer", false},
    {"media", 'm', "Media", "Media remote", "Play, volume, tracks", false},
    {"faces", 'f', "Faces", "Faces", "Pick the Home face", false},
    {"quick", 'q', "Quick", "Quick settings", "Brightness, sound, toggles", true},
    {"settings", 's', "Settings", "Settings", "Combo, Bluetooth, all the rest", true},
};

const TileInfo& info(Tile t) { return TILES[(int)t < TILE_COUNT ? (int)t : 0]; }

bool fromName(const char* s, size_t n, Tile& out) {
  for (int i = 0; i < TILE_COUNT; i++) {
    const TileInfo& t = TILES[i];
    if ((n == 1 && s[0] == t.letter) || (strlen(t.id) == n && strncmp(s, t.id, n) == 0)) {
      out = (Tile)i;
      return true;
    }
  }
  return false;
}

int Layout::indexOf(Tile t) const {
  for (int i = 0; i < n; i++)
    if (tiles[i] == t) return i;
  return -1;
}

bool Layout::operator==(const Layout& o) const {
  if (n != o.n) return false;
  for (int i = 0; i < n; i++)
    if (tiles[i] != o.tiles[i]) return false;
  return true;
}

Layout defaults() {
  Layout l;
  for (int i = 0; i < TILE_COUNT; i++) l.tiles[i] = (Tile)i;
  l.n = TILE_COUNT;
  return l;
}

static void addFixed(Layout& l) {
  for (int i = 0; i < TILE_COUNT; i++)
    if (TILES[i].fixed && l.indexOf((Tile)i) < 0) l.tiles[l.n++] = (Tile)i;
}

static void setBad(char* bad, size_t badN, const char* s, size_t n) {
  if (!bad || !badN) return;
  if (n >= badN) n = badN - 1;
  memcpy(bad, s, n);
  bad[n] = 0;
}

ParseError parse(const char* s, Layout& out, char* bad, size_t badN) {
  if (!s) return ParseError::EMPTY;
  Layout l;
  bool seen[TILE_COUNT] = {};
  const char* p = s;
  while (true) {
    while (*p == ' ') p++;
    const char* e = p;
    while (*e && *e != ',') e++;
    const char* end = e;
    while (end > p && end[-1] == ' ') end--;
    if (end > p) {
      Tile t;
      if (!fromName(p, end - p, t)) {
        setBad(bad, badN, p, end - p);
        return ParseError::UNKNOWN;
      }
      if (seen[(int)t]) {
        setBad(bad, badN, p, end - p);
        return ParseError::DUPLICATE;
      }
      seen[(int)t] = true;
      l.tiles[l.n++] = t;
    }
    if (!*e) break;
    p = e + 1;
  }
  if (!l.n) return ParseError::EMPTY;
  addFixed(l);
  out = l;
  return ParseError::NONE;
}

ParseError parseLetters(const char* s, Layout& out) {
  if (!s || !*s) return ParseError::EMPTY;
  Layout l;
  bool seen[TILE_COUNT] = {};
  for (const char* p = s; *p; p++) {
    Tile t;
    if (!fromName(p, 1, t)) return ParseError::UNKNOWN;
    if (seen[(int)t]) return ParseError::DUPLICATE;
    seen[(int)t] = true;
    l.tiles[l.n++] = t;
  }
  addFixed(l);
  out = l;
  return ParseError::NONE;
}

void letters(const Layout& l, char* out, size_t n) {
  if (!n) return;
  size_t k = 0;
  for (int i = 0; i < l.n && k + 1 < n; i++) out[k++] = info(l.tiles[i]).letter;
  out[k] = 0;
}

void ids(const Layout& l, char* out, size_t n) {
  if (!n) return;
  size_t k = 0;
  out[0] = 0;
  for (int i = 0; i < l.n; i++) {
    const char* id = info(l.tiles[i]).id;
    size_t len = strlen(id) + (i ? 1 : 0);
    if (k + len + 1 > n) break;
    if (i) out[k++] = ',';
    strcpy(out + k, id);
    k += strlen(id);
  }
}

const char* errorText(ParseError e) {
  switch (e) {
    case ParseError::EMPTY: return "no tiles";
    case ParseError::UNKNOWN: return "unknown tile";
    case ParseError::DUPLICATE: return "tile listed twice";
    default: return "";
  }
}

// --- Grid ---------------------------------------------------------------------------------

Grid gridFor(bool wide) { return wide ? Grid{3, 2} : Grid{2, 3}; }

Cell cellOf(int i, const Grid& g) {
  const int per = g.perPage(), k = i % per;
  return {i / per, k % g.cols, k / g.cols};
}

int pages(int n, int perPage) { return n <= 0 || perPage <= 0 ? 1 : (n + perPage - 1) / perPage; }

int next(int sel, int n) { return n <= 0 ? 0 : (sel + 1) % n; }
int prev(int sel, int n) { return n <= 0 ? 0 : (sel + n - 1) % n; }

int turnPage(int sel, int n, int perPage, int dir) {
  const int np = pages(n, perPage);
  if (np <= 1 || n <= 0) return sel;
  const int page = ((sel / perPage + (dir >= 0 ? 1 : -1)) % np + np) % np;
  const int i = page * perPage + sel % perPage;
  return i < n ? i : n - 1;
}

int startAt(const Layout& l, Tile last) {
  const int i = l.indexOf(last);
  return i < 0 ? 0 : i;
}

// --- Quick settings ---------------------------------------------------------------------------

uint8_t nextBrightness(uint8_t pct) { return pct >= 100 ? 25 : (uint8_t)((pct / 25 + 1) * 25); }

uint8_t nextVolume(uint8_t v) { return v >= 255 ? 64 : v >= 192 ? 255 : v >= 128 ? 192 : v >= 64 ? 128 : 64; }

uint8_t volumePct(uint8_t v) { return (uint8_t)((v * 100 + 127) / 255); }

bool gesturesOn(bool lift, bool faceDown, bool shake) { return lift || faceDown || shake; }

void toggleGestures(bool& lift, bool& faceDown, bool& shake) {
  const bool on = !gesturesOn(lift, faceDown, shake);
  lift = faceDown = shake = on;
}

}  // namespace launcher
