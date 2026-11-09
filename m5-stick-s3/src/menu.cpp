#include "menu.h"
#include "faces/faces.h"

namespace menu {

constexpr uint32_t TIMEOUT_MS = 60000;
constexpr uint32_t CONFIRM_MS = 3000;
constexpr uint8_t  STANDBY_STEPS[] = {0, 5, 10, 30};
constexpr uint16_t ENTER_STEPS[]   = {0, 500, 1000, 1500, 2000, 3000};

enum Item {
  R_MOUSE, R_PRESENT, R_MEDIA,  // the remote modes (2.5.0, air.h): start one
  MOD_FN, MOD_CTRL, MOD_OPTION, MOD_SHIFT, MOD_CMD, KEY, TEST, ENTER,
  SLOT, BT, FORGET, STANDBY, SOUND, VOLUME, WMIC, M_LIFT, M_FACE, M_SHAKE, BRIGHT, HOME_FACE, CLOCK24, AUTOROT, LANDSCAPE, FLIP, WEB, DEV, POWER, SAVE, CANCEL, COUNT
};

static Draft    draft_;
static uint8_t  originalSlot;
static bool     btOff_;
static int      sel = 0;
static uint32_t lastInput = 0, forgetArmedAt = 0;
static bool     forgetArmed = false, emptyWarning = false;
static uint8_t  remote_ = 0;

uint8_t remoteMode() { return remote_; }

void open(const Draft& current, bool btOff) {
  draft_ = current;
  originalSlot = current.slot;
  btOff_ = btOff;
  sel = 0;
  forgetArmed = emptyWarning = false;
  lastInput = millis();
}

const Draft& draft() { return draft_; }

bool timedOut() { return millis() - lastInput > TIMEOUT_MS; }

static void touch() {
  lastInput = millis();
  if (forgetArmed && millis() - forgetArmedAt > CONFIRM_MS) forgetArmed = false;
}

Action next() {
  touch();
  sel = (sel + 1) % COUNT;
  forgetArmed = false;
  return Action::REDRAW;
}

static void nextKey() {
  int i = keyIndex(draft_.combo.key);
  draft_.combo.key = KEYS[(i + 1) % KEY_COUNT].code;
}

static void nextStandby() {
  size_t n = sizeof(STANDBY_STEPS);
  size_t i = 0;
  while (i < n && STANDBY_STEPS[i] != draft_.standbyMin) i++;
  draft_.standbyMin = STANDBY_STEPS[(i + 1) % n];
}

Action select() {
  touch();
  emptyWarning = false;
  switch (sel) {
    case R_MOUSE: case R_PRESENT: case R_MEDIA:
      remote_ = sel - R_MOUSE + 1;
      return Action::REMOTE;
    case MOD_FN: case MOD_CTRL: case MOD_OPTION: case MOD_SHIFT: case MOD_CMD:
      draft_.combo.mods ^= MODS[sel - MOD_FN].bit;
      return Action::REDRAW;
    case KEY:     nextKey(); return Action::REDRAW;
    case TEST:
      if (!draft_.combo.valid()) { emptyWarning = true; return Action::REDRAW; }
      return Action::TEST;
    case ENTER: {
      size_t n = sizeof(ENTER_STEPS) / sizeof(ENTER_STEPS[0]), i = 0;
      while (i < n && ENTER_STEPS[i] != draft_.enterDelayMs) i++;
      draft_.enterDelayMs = ENTER_STEPS[(i + 1) % n];
      return Action::REDRAW;
    }
    case SLOT:    draft_.slot = draft_.slot % 3 + 1; return Action::REDRAW;
    case BT:      btOff_ = !btOff_; return Action::BT_TOGGLE;
    case FORGET:
      if (forgetArmed) { forgetArmed = false; return Action::FORGET; }
      forgetArmed = true;
      forgetArmedAt = millis();
      return Action::REDRAW;
    case STANDBY: nextStandby(); return Action::REDRAW;
    case M_LIFT:  draft_.lift = !draft_.lift; return Action::REDRAW;
    case M_FACE:  draft_.faceDown = !draft_.faceDown; return Action::REDRAW;
    case M_SHAKE: draft_.shake = !draft_.shake; return Action::REDRAW;
    case BRIGHT:  // preview live, kept only on Save
      draft_.brightnessPct = draft_.brightnessPct >= 100 ? 25 : (draft_.brightnessPct / 25 + 1) * 25;
      return Action::BRIGHTNESS;
    case HOME_FACE: draft_.face = (draft_.face + 1) % faces::count(); return Action::REDRAW;
    case CLOCK24: draft_.clock24 = !draft_.clock24; return Action::REDRAW;
    case AUTOROT: draft_.autoRotate = !draft_.autoRotate; return Action::REDRAW;
    case LANDSCAPE: draft_.landscape = !draft_.landscape; return Action::REDRAW;
    case FLIP:    return Action::FLIP;
    case DEV:     draft_.devMode = !draft_.devMode; return Action::REDRAW;
    case SOUND:   draft_.sound = !draft_.sound; return Action::REDRAW;
    case WMIC:    draft_.wmic = !draft_.wmic; return Action::REDRAW;
    case VOLUME:  draft_.volume = draft_.volume >= 255 ? 64 : draft_.volume >= 192 ? 255 : draft_.volume >= 128 ? 192 : 128;
                  return Action::VOLUME;
    case POWER:   return Action::POWER_OFF;
    case WEB:     return Action::WEB;
    case SAVE:
      if (!draft_.combo.valid()) { emptyWarning = true; return Action::REDRAW; }
      return Action::SAVE;
    case CANCEL:  return Action::CANCEL;
  }
  return Action::NONE;
}

Action repeat() {
  if (sel != KEY) return Action::NONE;
  touch();
  nextKey();
  return Action::REDRAW;
}

ui::MenuView view() {
  static String title;
  title = emptyWarning ? "Pick a key or modifier" : comboToString(draft_.combo);

  ui::MenuView v{title.c_str(), {}, COUNT, sel, "A next  B change"};
  v.lines[R_MOUSE]   = "Air mouse  >";
  v.lines[R_PRESENT] = "Presenter  >";
  v.lines[R_MEDIA]   = "Media remote  >";
  for (int i = MOD_FN; i <= MOD_CMD; i++)
    v.lines[i] = String(MODS[i - MOD_FN].name) + ": " + ((draft_.combo.mods & MODS[i].bit) ? "ON" : "off");
  v.lines[KEY]     = String("Key: ") + KEYS[max(0, keyIndex(draft_.combo.key))].name;
  v.lines[TEST]    = "Test combo";
  v.lines[ENTER]   = draft_.enterDelayMs ? String("Enter after: ") + String(draft_.enterDelayMs / 1000.0f, 1) + "s"
                                         : String("Enter after: off");
  v.lines[SLOT]    = String("Mac slot: ") + draft_.slot + (draft_.slot != originalSlot ? " *" : "");
  v.lines[BT]      = btOff_ ? "Reconnect BT" : "Disconnect BT";
  v.lines[FORGET]  = forgetArmed ? "B again: forget" : String("Forget Mac ") + originalSlot;
  v.lines[STANDBY] = draft_.standbyMin ? String("Standby: ") + draft_.standbyMin + " min" : "Standby: off";
  v.lines[M_LIFT]  = String("Lift wakes: ") + (draft_.lift ? "ON" : "off");
  v.lines[M_FACE]  = String("Face-down sleep: ") + (draft_.faceDown ? "ON" : "off");
  v.lines[M_SHAKE] = String("Shake = Esc: ") + (draft_.shake ? "ON" : "off");
  v.lines[BRIGHT]  = String("Brightness: ") + draft_.brightnessPct + "%";
  v.lines[HOME_FACE] = String("Face: ") + faces::at(draft_.face).title();
  v.lines[CLOCK24] = String("Clock: ") + (draft_.clock24 ? "24-hour" : "12-hour");
  v.lines[AUTOROT] = String("Auto-rotate: ") + (draft_.autoRotate ? "ON" : "off");
  v.lines[LANDSCAPE] = String("Landscape screens: ") + (draft_.landscape ? "ON" : "off");
  v.lines[FLIP]    = "Flip screen";
  v.lines[DEV]     = String("Dev mode: ") + (draft_.devMode ? "ON" : "off");
  v.lines[SOUND]   = String("Sounds: ") + (draft_.sound ? "ON" : "off");
  v.lines[WMIC]    = String("Wireless mic: ") + (draft_.wmic ? "ON" : "off");
  v.lines[VOLUME]  = String("Volume: ") + (draft_.volume * 100 + 127) / 255 + "%";
  v.lines[POWER]   = "Power off";
  v.lines[WEB]     = "Web settings";
  bool restart = draft_.slot != originalSlot;
  v.lines[SAVE]    = restart ? "Save & restart" : "Save & exit";
  v.lines[CANCEL]  = "Cancel";

  if (sel == KEY) v.hint = "B next  hold=fast";
  else if (sel == ENTER) v.hint = "Enter after a hold";
  else if (sel == SLOT && draft_.slot != originalSlot) v.hint = "Save to switch Mac";
  else if (sel == DEV) v.hint = "USB: Wi-Fi on, OTA+logs";
  else if (sel == WMIC) v.hint = "A hold: Wispr mic by BT";
  else if (sel == HOME_FACE) v.hint = "Home screen look";
  else if (sel == LANDSCAPE) v.hint = draft_.autoRotate ? "Held sideways: wide" : "Needs auto-rotate";
  else if (sel >= R_MOUSE && sel <= R_MEDIA) v.hint = "B start  A+B hold: exit";
  else if (sel == TEST || sel == BT || sel == FLIP || sel == WEB || sel == POWER || sel >= SAVE) v.hint = "A next  B select";
  return v;
}

}  // namespace menu
