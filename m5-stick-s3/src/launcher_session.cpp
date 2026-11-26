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

static Catalog  catalog;
static Layout   layout_ = catalog.grid();
static Screen   screen_ = Screen::GRID;
static int      sel = 0, qsel = 0, fsel = 0;
static uint32_t lastInput = 0;

static void touch() { lastInput = millis(); }

void begin() {
  unpack(settings::launcherRecord().c_str(), catalog);
  layout_ = catalog.grid();
}

static Tile lastTile() {
  Tile t;
  const char c = settings::launcherLast();
  return fromName(&c, 1, t) ? t : Tile::GAMES;
}

void open() {
  screen_ = Screen::GRID;
  sel = catalog.openingSelection(lastTile());
  touch();
}

bool timedOut() { return millis() - lastInput > TIMEOUT_MS; }
bool visible(Tile tile) { return catalog.visible.indexOf(tile) >= 0; }
bool onGrid() { return screen_ == Screen::GRID; }

static launcherui::QuickView quickView() {
  launcherui::QuickView v;
  v.sel = qsel;
  v.brightness = settings::brightnessPct();
  v.volume = sound::volume();
  v.sound = sound::on();
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
      sound::setVolume(nextVolume(sound::volume()));
      if (!sound::on()) sound::setOn(true);
      sound::play(sound::Cue::TEST);
      break;
    case Quick::SOUND:
      sound::setOn(!sound::on());
      sound::play(sound::Cue::TEST);
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
  Log.event(diagnostics::Event::Launcher);
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
      Log.event(diagnostics::Event::Launcher);
      return Action::HOME;
    }
    case Screen::GRID: break;
  }
  if (!layout_.n) return Action::NONE;
  return openTile(layout_.tiles[sel < layout_.n ? sel : 0]);
}

Action openTile(Tile t, bool remember) {
  touch();
  sel = startAt(layout_, t);
  if (remember) settings::saveLauncherLast(info(t).letter);
  Log.event(diagnostics::Event::Launcher);
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
  char l[TILE_COUNT + 1], f[TILE_COUNT + 1];
  letters(catalog.visible, l, sizeof l);
  letters(catalog.favorites, f, sizeof f);
  return String("\"launcher\":\"") + l + "\",\"launcherFav\":\"" + f + "\",\"stickOpen\":1";
}

protocol::Result command(const protocol::Args& arg) {
  protocol::Result r;
  const String tiles = arg("tiles"), favorites = arg("favorites"), reset = arg("reset");
  Catalog next = catalog;
  if (reset == "1" || reset == "true") {
    next = Catalog{};
  } else {
    if (tiles.length()) {
      char bad[16] = "";
      const ParseError e = parse(tiles.c_str(), next.visible, bad, sizeof bad);
      if (e != ParseError::NONE) {
        for (char* p = bad; *p; ++p)
          if (*p == '"' || *p == '\\' || (uint8_t)*p < 0x20) *p = '?';
        return {400, String("{\"ok\":false,\"error\":\"") + errorText(e) + (bad[0] ? String(": ") + bad : String()) + "\"}"};
      }
    }
    if (favorites.length()) {
      if (!setFavorites(next, favorites.c_str())) return {400, "{\"ok\":false,\"error\":\"Invalid favorite tiles\"}"};
    } else {
      retainVisibleFavorites(next);
    }
  }
  if (!(next == catalog)) {
    if (!settings::saveLauncherRecord(next.pack())) return {500, "{\"ok\":false,\"error\":\"Could not save launcher\"}"};
    catalog = next;
    layout_ = catalog.grid();
    sel = 0;
    Log.event(diagnostics::Event::Launcher);
  }
  char ids_[80];
  ids(catalog.visible, ids_, sizeof ids_);
  r.json = "{\"ok\":true," + stateJson() + ",\"tiles\":\"" + ids_ + "\"}";
  return r;
}

}  // namespace launcher
