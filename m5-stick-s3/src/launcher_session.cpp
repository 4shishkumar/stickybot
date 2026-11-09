#include "launcher_session.h"
#include "launcher_ui.h"
#include "netlog.h"
#include "settings.h"
#include "sound.h"
#include "ui.h"
#include "buddy.h"
#include "faces/faces.h"

namespace launcher {

constexpr uint32_t TIMEOUT_MS = 60000;

enum class Screen : uint8_t { GRID, QUICK, FACES };

static Layout   layout_ = defaults();
static Screen   screen_ = Screen::GRID;
static int      sel = 0, qsel = 0, fsel = 0;
static uint32_t lastInput = 0;

static void touch() { lastInput = millis(); }

void begin() {
  Layout l;
  String saved = settings::launcherTiles();
  if (saved.length() && parseLetters(saved.c_str(), l) == ParseError::NONE) layout_ = l;
}

const Layout& layout() { return layout_; }

static Tile lastTile() {
  Tile t;
  const char c = settings::launcherLast();
  return fromName(&c, 1, t) ? t : Tile::GAMES;
}

void open() {
  screen_ = Screen::GRID;
  sel = startAt(layout_, lastTile());
  touch();
}

bool timedOut() { return millis() - lastInput > TIMEOUT_MS; }
bool onGrid() { return screen_ == Screen::GRID; }

static launcherui::QuickView quickView() {
  launcherui::QuickView v;
  v.sel = qsel;
  v.brightness = settings::brightnessPct();
  v.volume = settings::soundVolume();
  v.sound = settings::soundOn();
  v.wmic = settings::wirelessMic();
  v.clock24 = settings::clock24();
  v.rotate = settings::autoRotate();
  v.gestures = gesturesOn(settings::liftWakes(), settings::faceDownSleeps(), settings::shakeEscapes());
  return v;
}

void draw() {
  switch (screen_) {
    case Screen::GRID: {
      launcherui::GridView v;
      v.layout = layout_;
      v.sel = sel < layout_.n ? sel : 0;
      v.clock = buddy::clock();
      launcherui::drawGrid(v);
      break;
    }
    case Screen::QUICK:
      launcherui::drawQuick(quickView());
      break;
    case Screen::FACES: {
      launcherui::FacesView v;
      v.sel = fsel;
      v.current = max(0, faces::find(settings::face()));
      launcherui::drawFaces(v);
      break;
    }
  }
}

Action pressA() {
  touch();
  switch (screen_) {
    case Screen::GRID: sel = next(sel, layout_.n); break;
    case Screen::QUICK: qsel = next(qsel, QUICK_COUNT); break;
    case Screen::FACES: fsel = next(fsel, faces::count()); break;
  }
  return Action::REDRAW;
}

Action flick(int dir) {
  if (screen_ != Screen::GRID) return Action::NONE;
  const int to = turnPage(sel, layout_.n, gridFor(false).perPage(), dir);
  if (to == sel) return Action::NONE;
  touch();
  sel = to;
  return Action::REDRAW;
}

// Quick settings: B changes the selected one, saved at once (the same setters as the menu,
// the dashboard and the protocol).
static Action quickChange() {
  switch ((Quick)qsel) {
    case Quick::BRIGHTNESS: {
      const uint8_t p = nextBrightness(settings::brightnessPct());
      settings::saveBrightnessPct(p);
      ui::setBrightness(p);
      break;
    }
    case Quick::VOLUME:  // turning it up unmutes; a preview at the new level
      settings::saveSoundVolume(nextVolume(settings::soundVolume()));
      if (!settings::soundOn()) settings::saveSoundOn(true);
      sound::play(sound::Cue::TEST);
      break;
    case Quick::SOUND:
      settings::saveSoundOn(!settings::soundOn());
      if (settings::soundOn()) sound::play(sound::Cue::TEST);
      break;
    case Quick::WMIC: settings::saveWirelessMic(!settings::wirelessMic()); break;
    case Quick::CLOCK24: settings::saveClock24(!settings::clock24()); break;
    case Quick::ROTATE:
      settings::saveAutoRotate(!settings::autoRotate());
      if (!settings::autoRotate()) ui::setFlipped(settings::flipped());  // back to the manual flip (as the menu)
      break;
    case Quick::GESTURES: {
      bool lift = settings::liftWakes(), face = settings::faceDownSleeps(), shake = settings::shakeEscapes();
      toggleGestures(lift, face, shake);
      settings::saveMotion(lift, face, shake);
      break;
    }
    default:
      return Action::SETTINGS;  // "Settings": the full menu
  }
  Log.printf("Quick settings: %d changed\n", qsel);
  return Action::REDRAW;
}

Action clickB() {
  touch();
  switch (screen_) {
    case Screen::QUICK: return quickChange();
    case Screen::FACES: {
      const String id = faces::at(fsel).id();
      settings::saveFace(id);
      faces::select(id);
      Log.printf("Face: %s (launcher)\n", id.c_str());
      return Action::HOME;
    }
    case Screen::GRID: break;
  }
  if (!layout_.n) return Action::NONE;
  const Tile t = layout_.tiles[sel < layout_.n ? sel : 0];
  settings::saveLauncherLast(info(t).letter);
  Log.printf("Launcher: %s\n", info(t).id);
  switch (t) {
    case Tile::GAMES: return Action::GAMES;
    case Tile::REMOTE: return Action::REMOTE;
    case Tile::AIR: return Action::AIR;
    case Tile::PRESENTER: return Action::PRESENTER;
    case Tile::MEDIA: return Action::MEDIA;
    case Tile::SETTINGS: return Action::SETTINGS;
    case Tile::FACES:
      screen_ = Screen::FACES;
      fsel = max(0, faces::find(settings::face()));
      return Action::REDRAW;
    default:
      screen_ = Screen::QUICK;
      qsel = 0;
      return Action::REDRAW;
  }
}

Action holdB() {
  touch();
  if (screen_ == Screen::GRID) return Action::HOME;
  screen_ = Screen::GRID;  // Quick settings / Faces: back to the grid
  return Action::REDRAW;
}

// --- Protocol --------------------------------------------------------------------------------

String stateJson() {
  char l[TILE_COUNT + 1];
  letters(layout_, l, sizeof l);
  return String("\"launcher\":\"") + l + "\"";
}

protocol::Result command(const protocol::Args& arg) {
  protocol::Result r;
  const String tiles = arg("tiles"), reset = arg("reset");
  Layout l = layout_;
  if (reset == "1" || reset == "true") {
    l = defaults();
  } else if (tiles.length()) {
    char bad[16] = "";
    const ParseError e = parse(tiles.c_str(), l, bad, sizeof bad);
    if (e != ParseError::NONE) {
      for (char* p = bad; *p; p++)
        if (*p == '"' || *p == '\\' || (uint8_t)*p < 0x20) *p = '?';  // it goes back in JSON
      r.status = 400;
      r.json = String("{\"ok\":false,\"error\":\"") + errorText(e) + (bad[0] ? String(": ") + bad : String()) + "\"}";
      return r;
    }
  }
  if (!(l == layout_)) {
    layout_ = l;
    char lt[TILE_COUNT + 1];
    letters(l, lt, sizeof lt);
    settings::saveLauncherTiles(l == defaults() ? String() : String(lt));
    if (sel >= layout_.n) sel = 0;
    Log.printf("Launcher tiles: %s\n", lt);
  }
  char ids_[80];
  ids(layout_, ids_, sizeof ids_);
  r.json = "{\"ok\":true," + stateJson() + ",\"tiles\":\"" + ids_ + "\"}";
  return r;
}

}  // namespace launcher
