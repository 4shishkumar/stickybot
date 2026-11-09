// The Remote screen: pick a remote, then its buttons. Portrait.
//   Remotes:  A open   B next   B-B previous   hold B: back to Home   (tilt sideways scrolls)
//   Buttons:  A send (hold: repeat)   B next   B-B previous   B-B-B favourite   hold B: remotes
//   "+ Learn": point the remote at the stick's top and press the button (twice)
// A learn started from the Mac shows here too (B cancels it).
#include "ir.h"
#include "store.h"
#include "../ui_draw.h"
#include "../netlog.h"

namespace ir {

using namespace ui;

constexpr uint32_t IDLE_MS = 60000;
constexpr uint32_t FLASH_MS = 350;
constexpr uint32_t RESULT_MS = 2200;
constexpr uint32_t TILT_EVERY_MS = 450;

struct Item {
  String dev, btn, proto;
  bool fav = false;
  uint8_t kind = 0;  // 0 a remote / button, 1 favourites, 2 learn
};

static bool open_ = false, inButtons = false, favView = false;
static Item items[irnames::MAX_BUTTONS + 2 > irnames::MAX_DEVICES + 2 ? irnames::MAX_BUTTONS + 2 : irnames::MAX_DEVICES + 2];
static int count = 0, sel = 0, devSel = 0;
static String curDev;
static uint32_t lastInput = 0, flashAt = 0, lastDraw = 0, lastTilt = 0;
static bool flashOk = true;
static String flashText;
static bool learnShown = false;

static void loadDevices() {
  char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
  const int n = store::list(ids, irnames::MAX_DEVICES);
  count = 0;
  bool anyFav = false;
  for (int i = 0; i < n; i++) {
    irdoc::Device d;
    if (store::load(ids[i], d))
      for (auto& b : d.buttons) anyFav |= b.fav;
  }
  if (anyFav) {
    items[count] = Item{};
    items[count].kind = 1;
    count++;
  }
  for (int i = 0; i < n; i++) {
    items[count] = Item{};
    items[count++].dev = ids[i];
  }
  if (n < irnames::MAX_DEVICES) {
    items[count] = Item{};
    items[count++].kind = 2;
  }
  sel = constrain(devSel, 0, count - 1);
}

static void loadButtons() {
  count = 0;
  auto add = [&](const String& dev, const irdoc::Button& b) {
    if (count >= irnames::MAX_BUTTONS) return;
    Item& it = items[count++];
    it = Item{};
    it.dev = dev;
    it.btn = b.name.c_str();
    it.fav = b.fav;
    it.proto = b.code.substr(0, b.code.find(':')).c_str();
  };
  if (favView) {
    char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
    const int n = store::list(ids, irnames::MAX_DEVICES);
    for (int i = 0; i < n; i++) {
      irdoc::Device d;
      if (store::load(ids[i], d))
        for (auto& b : d.buttons)
          if (b.fav) add(ids[i], b);
    }
  } else {
    irdoc::Device d;
    store::load(curDev.c_str(), d);
    for (int pass = 0; pass < 2; pass++)  // favourites first
      for (auto& b : d.buttons)
        if (b.fav == (pass == 0)) add(curDev, b);
    if ((int)d.buttons.size() < irnames::MAX_BUTTONS) {
      items[count] = Item{};
      items[count].kind = 2;
      count++;
    }
  }
  sel = constrain(sel, 0, max(0, count - 1));
}

void screenOpen() {
  open_ = true;
  inButtons = favView = false;
  devSel = 0;
  store::begin();
  loadDevices();
  lastInput = millis();
  ui::wake(IDLE_MS);
  Log.println("Remote screen");
}

bool screenActive() { return open_ || learning() || learnShown; }

static String title(const String& s) {
  if (!s.length()) return s;
  String t = s;
  t.replace("-", " ");
  t.setCharAt(0, toupper(t[0]));
  return t;
}

static String itemLabel(const Item& it) {
  if (it.kind == 1) return "Favourites";
  if (it.kind == 2) return inButtons ? "+ Learn a button" : "+ Learn a remote";
  if (!inButtons) return title(it.dev);
  return favView ? title(it.dev) + ": " + it.btn : it.btn;
}

static void star(int cx, int cy, int r, uint16_t c) {
  for (int i = 0; i < 5; i++) {
    const float a0 = -PI / 2 + i * 2 * PI / 5, a1 = a0 + PI / 5, a2 = a0 + 2 * PI / 5;
    canvas.fillTriangle(cx, cy, cx + r * cosf(a0), cy + r * sinf(a0), cx + r * 0.45f * cosf(a1), cy + r * 0.45f * sinf(a1), c);
    canvas.fillTriangle(cx, cy, cx + r * 0.45f * cosf(a1), cy + r * 0.45f * sinf(a1), cx + r * cosf(a2), cy + r * sinf(a2), c);
  }
}

static void drawList() {
  dockEnd();
  const uint32_t since = millis() - flashAt;
  const bool flashing = flashAt && since < FLASH_MS;
  uint16_t bg = GROUND;
  if (flashing) bg = blend(flashOk ? AGENT : ERR, GROUND, smooth(since / (float)FLASH_MS));
  canvas.fillScreen(bg);
  // top bar
  const String head = !inButtons ? "Remote" : favView ? "Favourites" : title(curDev);
  txt(ellipsize(head, F12, W - 2 * PAD - 30), PAD, BAR_H / 2, F12, AGENT);
  if (count) txt(String(sel + 1) + "/" + count, W - PAD, BAR_H / 2, F9, FAINT, middle_right);
  canvas.drawFastHLine(0, BAR_H, W, LINE);

  if (!count) {
    txt("Nothing here yet", W / 2, H / 2, F12, DIM, middle_center);
  } else {
    // the selected one, big, in a card
    const Item& it = items[sel];
    const int cy = BAR_H + 12 + 46;
    canvas.fillRoundRect(4, BAR_H + 10, W - 8, 92, 12, flashing ? blend(PANEL, bg, 0.5f) : PANEL);
    const String label = itemLabel(it);
    String lines[3];
    const lgfx::IFont* f = fit(label, W - 24, {F24, F18, F14});
    int nl = wrap(label, f, lines, 2, W - 24);
    if (nl == 2 && f != F14) {
      f = F14;
      nl = wrap(label, f, lines, 2, W - 24);
    }
    const int lh = lineH(f);
    for (int i = 0; i < nl; i++)
      txt(lines[i], W / 2, cy - (nl - 1) * lh / 2 + i * lh, f, it.kind == 2 ? AMBER : INK, middle_center);
    if (it.fav) star(16, BAR_H + 22, 6, SUN);
    if (inButtons && it.kind == 0) txt(it.proto, W / 2, BAR_H + 90, F9, FAINT, middle_center);
    if (flashing) txt(flashText, W / 2, BAR_H + 112, F12, flashOk ? AGENT : ERR, middle_center);
    // what comes next
    int y = BAR_H + 128;
    for (int k = 1; k <= 3 && y < H - 30; k++, y += 20) {
      if (count < 2 || k >= count) break;
      const Item& nx = items[(sel + k) % count];
      txt(ellipsize(itemLabel(nx), F12, W - 2 * PAD), PAD + 4, y, F12, k == 1 ? DIM : FAINT);
    }
  }
  const char* hint = !inButtons ? "A open  B next  hold B exit" : "A send  B next  hold B back";
  txt(hint, W / 2, H - 9, F9, FAINT, middle_center);
  canvas.pushSprite(0, 0);
}

static void drawLearn() {
  dockEnd();
  const LearnView& v = learnView();
  canvas.fillScreen(GROUND);
  txt(v.listening ? "Listening" : "Learn", PAD, BAR_H / 2, F12, AMBER);
  canvas.drawFastHLine(0, BAR_H, W, LINE);
  const bool result = !v.active;
  if (!v.listening) {
    txt(ellipsize(title(v.dev), F14, W - 2 * PAD), W / 2, BAR_H + 18, F14, INK, middle_center);
    txt(ellipsize(v.btn.length() ? v.btn : String("new button"), F12, W - 2 * PAD), W / 2, BAR_H + 36, F12, DIM,
        middle_center);
  }
  // the receiver's "eye": pulses while it listens
  const float t = (millis() % 1200) / 1200.0f;
  const int cx = W / 2, cy = BAR_H + 92;
  uint16_t c = result ? (v.ok ? ui::OK : ui::ERR) : AMBER;
  if (!result) glow(cx, cy, 20 + (int)(10 * t), AMBER, 1 - t);
  canvas.fillCircle(cx, cy, 16, c);
  canvas.fillCircle(cx, cy, 7, GROUND);
  String msg;
  if (result) msg = v.last + (v.ok ? ": " + v.btn : "");
  else if (v.listening) msg = v.last.length() ? "Heard " + v.last : "Press any remote button";
  else if (v.last.length() && millis() - v.lastAt < 1500) msg = v.last + (v.step == 2 ? ". Again" : "");
  else msg = v.step == 2 ? "Press it again" : "Point the remote at the stick and press the button";
  paragraph(msg, F12, PAD, BAR_H + 130, W - 2 * PAD, 4, result ? INK : DIM, 16);
  txt(result ? "" : "B cancels", W / 2, H - 9, F9, FAINT, middle_center);
  canvas.pushSprite(0, 0);
}

static void move(int step) {
  if (!count) return;
  sel = (sel + step + count) % count;
}

static void learnNew() {
  if (!inButtons) {  // a new remote: "remote-1", "remote-2", ...
    char ids[irnames::MAX_DEVICES][irnames::DEVICE_LEN + 1];
    const int n = store::list(ids, irnames::MAX_DEVICES);
    int k = 1;
    for (;; k++) {
      bool taken = false;
      for (int i = 0; i < n; i++) taken |= String(ids[i]) == "remote-" + String(k);
      if (!taken) break;
    }
    curDev = "remote-" + String(k);
  }
  const char* why = startLearn(curDev.c_str(), "", false);
  if (why) {
    flashAt = millis();
    flashOk = false;
    flashText = why;
  }
}

bool screenLoop(const Input& in) {
  const LearnView& v = learnView();
  if (v.active) {
    learnShown = true;
    ui::wake(IDLE_MS);
    ui::keepAwake();
    if (in.bSingle || in.bHold || in.bDouble) cancelLearn("stick");
    if (millis() - lastDraw > 40) {
      lastDraw = millis();
      drawLearn();
    }
    return true;
  }
  if (learnShown) {  // the result for a moment, then back to the list (or Home)
    if (millis() - v.lastAt < RESULT_MS && !in.aDown && !in.bSingle) {
      if (millis() - lastDraw > 100) {
        lastDraw = millis();
        drawLearn();
      }
      return true;
    }
    learnShown = false;
    if (!open_) return false;
    if (v.ok && v.dev.length()) {  // show the remote it went into
      curDev = v.dev;
      inButtons = true;
      favView = false;
      loadButtons();
      for (int i = 0; i < count; i++)
        if (items[i].btn == v.btn) sel = i;
    } else {
      inButtons ? loadButtons() : loadDevices();
    }
    lastDraw = 0;
  }
  if (!open_) return false;

  const bool any = in.aDown || in.bSingle || in.bDouble || in.bTriple || in.bHold;
  if (any) {
    lastInput = millis();
    ui::wake(IDLE_MS);
  }
  if (millis() - lastInput > IDLE_MS || !ui::awake()) {
    open_ = false;
    releaseHeld();
    return false;
  }
  ui::keepAwake();
  bool redraw = any || in.aUp;
  // tilting sideways scrolls
  if (fabsf(in.tiltX) > 0.5f && millis() - lastTilt > TILT_EVERY_MS && !in.aHeld) {
    lastTilt = millis();
    lastInput = millis();
    move(in.tiltX > 0 ? 1 : -1);
    redraw = true;
  }
  if (!inButtons) {
    if (in.bHold) {
      open_ = false;
      return false;
    }
    if (in.bSingle) move(1);
    if (in.bDouble) move(-1);
    if (in.aDown && count) {
      const Item& it = items[sel];
      devSel = sel;
      if (it.kind == 2) learnNew();
      else {
        favView = it.kind == 1;
        curDev = it.dev;
        inButtons = true;
        sel = 0;
        loadButtons();
      }
    }
  } else {
    if (in.bHold) {
      releaseHeld();
      inButtons = false;
      loadDevices();
    }
    if (in.bSingle) move(1);
    if (in.bDouble) move(-1);
    if (in.bTriple && count && items[sel].kind == 0) {  // favourite on / off
      Item it = items[sel];
      String dev = it.dev, btn = it.btn;
      protocol::Result r = run([&](const char* n) -> String {
        return !strcmp(n, "op") ? String("fav") : !strcmp(n, "dev") ? dev : !strcmp(n, "btn") ? btn : String();
      });
      loadButtons();
      for (int i = 0; i < count; i++)
        if (items[i].dev == dev && items[i].btn == btn) sel = i;
      flashAt = millis();
      flashOk = r.status == 200;
      flashText = it.fav ? "Unstarred" : "Starred";
    }
    if (in.aDown && count) {
      const Item& it = items[sel];
      if (it.kind == 2) learnNew();
      else {
        const char* why = sendButton(it.dev.c_str(), it.btn.c_str(), 0, true);
        flashAt = millis();
        flashOk = !why;
        flashText = why ? why : "Sent";
      }
    }
    if (in.aUp) releaseHeld();
    if (in.aHeld && sending()) {  // keeps the flash going while it repeats
      flashAt = millis() - FLASH_MS / 3;
      flashText = "Sending";
      redraw = true;
    }
  }
  if (flashAt && millis() - flashAt < FLASH_MS + 50) redraw = redraw || millis() - lastDraw > 40;
  if (redraw || millis() - lastDraw > 1000) {
    lastDraw = millis();
    drawList();
  }
  return true;
}

}  // namespace ir
