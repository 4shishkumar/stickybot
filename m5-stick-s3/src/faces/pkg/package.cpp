#include "package.h"
#include <Preferences.h>
#include <esp_system.h>
#include "engine.h"
#include "install.h"
#include "store.h"
#include "../common.h"
#include "../../netlog.h"
#include "../../settings.h"

namespace faces {
namespace pkg {

constexpr uint32_t CRASH_CLEAR_MS = 60000;
constexpr int      CRASHES = 2;      // restarts in a row from a crash with a package face on Home
constexpr int      PAGE = 6;         // @faces entries per page (each ~55 bytes)

class PackageFace : public Face {
 public:
  Info     info;
  uint32_t size = 0;
  char     sha8[9] = {0};   // the sha256's first 8 hex digits (@faces), "" until worked out
  String   err;             // why it didn't load ("" fine, or not tried yet)
  bool     checked = false; // loaded once since it was listed (err is the answer)

  const char* id() const override { return info.id; }
  const char* title() const override { return info.name[0] ? info.name : info.id; }
  const char* about() const override { return info.about; }
  bool clockOnly() const override { return info.motion == facefmt::STATIC; }
  void draw(const ui::Dash& d, bool wide, uint32_t now) override;
  void kick(world::Kick k, uint32_t now) override;
  uint32_t frameMs(bool onBattery, uint32_t idleMs) const override;
  void reset() override;
};

static PackageFace slots_[store::MAX_FACES];
static int n_ = 0;
static Engine engine_;                       // the face on screen (one at a time)
static const PackageFace* loadedFor_ = nullptr;
static bool loadFailed_ = false;
static Preferences guard_;
static String crashed_;                      // the face the crash guard took off Home
static bool guardCleared_ = false;

static void markBroken(PackageFace& f, const String& why) {
  f.err = why;
  f.checked = true;
}

// The face's engine, loaded (true) or not loadable (false: draw Classic).
static bool ensure(PackageFace& f) {
  if (loadedFor_ == &f) return !loadFailed_;
  engine_.unload();
  loadedFor_ = &f;
  loadFailed_ = true;
  if (crashed_ == f.info.id) {
    markBroken(f, "it crashed the stick: install it again");
    return false;
  }
  size_t n = 0;
  uint8_t* buf = store::read(f.info.id, n);
  if (!buf) {
    markBroken(f, "can't read it");
    Log.printf("Faces: %s invalid: can't read it\n", f.info.id);
    return false;
  }
  String why;
  const uint32_t t0 = millis();
  const LoadError e = engine_.load(buf, n, why);
  free(buf);
  if (e != LoadError::NONE) {
    markBroken(f, why);
    Log.printf("Faces: %s invalid: %s\n", f.info.id, why.c_str());
    return false;
  }
  f.err = "";
  f.checked = true;
  loadFailed_ = false;
  Log.printf("Faces: %s %s loaded (%u KB, %lu ms)\n", f.info.id, f.info.version, (unsigned)(engine_.memory() / 1024),
             (unsigned long)(millis() - t0));
  return true;
}

void PackageFace::draw(const ui::Dash& d, bool wide, uint32_t now) {
  if (!ensure(*this)) {  // never half-drawn: Classic instead
    faces::at(0).draw(d, wide, now);
    return;
  }
  float gx, gy;
  ui::gravity(gx, gy);  // tilt parallax: 0, 0 held upright
  engine_.setTilt(constrain(gx, -1.0f, 1.0f), constrain(gy - 1, -1.0f, 1.0f));
  const uint32_t t0 = micros();
  engine_.draw(d, faces::world(), wide, now);
  if (engine_.frameTook(micros() - t0, now))
    Log.printf("Faces: %s too slow, tier lowered (%d)\n", info.id, engine_.tier());
}

void PackageFace::kick(world::Kick k, uint32_t now) {
  if (loadedFor_ == this && !loadFailed_) engine_.kick(k, now);
}

uint32_t PackageFace::frameMs(bool onBattery, uint32_t idleMs) const {
  return loadedFor_ == this && !loadFailed_ ? engine_.frameMs(onBattery, idleMs, millis()) : 0;
}

void PackageFace::reset() {
  if (loadedFor_ == this && !loadFailed_) engine_.reset();
}

// --- The list -------------------------------------------------------------------------------

int count() { return n_; }

Face& at(int i) { return slots_[constrain(i, 0, max(0, n_ - 1))]; }

bool isPackage(const Face& f) { return &f >= slots_ && &f < slots_ + store::MAX_FACES; }

void rescan() {
  if (faces::demoActive()) faces::demoStop();  // its face indices are about to change
  engine_.unload();
  loadedFor_ = nullptr;
  char ids[store::MAX_FACES][13];
  const int n = store::list(ids, store::MAX_FACES);
  n_ = 0;
  for (int i = 0; i < n; i++) {
    if (faces::find(ids[i]) >= 0) continue;  // never shadow a built-in (n_ counts only the ones listed so far)
    PackageFace& f = slots_[n_];
    f.info = Info{};
    f.err = "";
    f.checked = false;
    f.sha8[0] = 0;
    snprintf(f.info.id, sizeof f.info.id, "%s", ids[i]);
    size_t len = 0;
    uint8_t* buf = store::read(ids[i], len);
    f.size = len;
    String why;
    if (!buf || !peek(buf, len, f.info, why)) {
      snprintf(f.info.id, sizeof f.info.id, "%s", ids[i]);  // (peek clears it)
      markBroken(f, buf ? why : String("can't read it"));
    } else if (strcmp(f.info.id, ids[i])) {
      snprintf(f.info.id, sizeof f.info.id, "%s", ids[i]);
      markBroken(f, "its id doesn't match its file name");
    }
    free(buf);
    n_++;
  }
  faces::refresh();
}

void installed(const char* id) {
  if (crashed_ == id) {
    crashed_ = "";
    guard_.remove("bad");
  }
  rescan();
}

// --- The crash guard ---------------------------------------------------------------------------

static bool crashReset() {
  switch (esp_reset_reason()) {
    case ESP_RST_PANIC: case ESP_RST_INT_WDT: case ESP_RST_TASK_WDT: case ESP_RST_WDT: return true;
    default: return false;
  }
}

void begin() {
  guard_.begin("pkgfaces", false);
  crashed_ = guard_.isKey("bad") ? guard_.getString("bad") : String();
  store::begin();
  const String face = settings::face();
  int crashes = guard_.getUChar("crash", 0);
  if (!crashReset()) crashes = 0;
  else if (store::fileSize(face.c_str())) crashes++;  // a package face was on Home
  guard_.putUChar("crash", crashes);
  if (crashes >= CRASHES && store::fileSize(face.c_str())) {
    crashed_ = face;
    guard_.putString("bad", face);
    guard_.putUChar("crash", 0);
    Log.printf("Faces: %s crashed the stick %d times in a row: Classic until it's installed again\n", face.c_str(),
               crashes);
  }
  rescan();
  Log.printf("Faces: %d package face%s installed\n", n_, n_ == 1 ? "" : "s");
}

void loop() {
  if (guardCleared_ || millis() < CRASH_CLEAR_MS) return;
  guardCleared_ = true;
  if (guard_.getUChar("crash", 0)) guard_.putUChar("crash", 0);
}

// --- @faces, @state, removing -----------------------------------------------------------------

static String esc(const String& s) {
  String o;
  for (char c : s) {
    if (c == '"' || c == '\\') o += '\\';
    if ((uint8_t)c >= 0x20) o += c;
  }
  return o;
}

// Loads a face once (into its own engine, so the one on screen stays) to know whether it works
static void check(PackageFace& f) {
  if (f.checked) return;
  if (crashed_ == f.info.id) {
    markBroken(f, "it crashed the stick: install it again");
    return;
  }
  size_t n = 0;
  uint8_t* buf = store::read(f.info.id, n);
  if (!buf) {
    markBroken(f, "can't read it");
    return;
  }
  Engine* e = new Engine();
  String why;
  if (e->load(buf, n, why) != LoadError::NONE) markBroken(f, why);
  else f.checked = true;
  delete e;
  free(buf);
}

String listJson(int page) {
  page = max(0, page);
  String j = "\"faces\":[";
  const int from = page * PAGE, to = min(n_, from + PAGE);
  for (int i = from; i < to; i++) {
    PackageFace& f = slots_[i];
    check(f);
    if (!f.sha8[0]) {
      uint8_t sha[32];
      if (store::sha256(f.info.id, sha))
        for (int k = 0; k < 4; k++) snprintf(f.sha8 + k * 2, 3, "%02x", sha[k]);
    }
    j += String(i > from ? "," : "") + "{\"id\":\"" + f.info.id + "\",\"v\":\"" + esc(f.info.version) +
         "\",\"sz\":" + f.size + ",\"sha\":\"" + f.sha8 + "\"";
    if (f.err.length()) j += ",\"err\":\"" + esc(f.err) + "\"";
    j += "}";
  }
  j += "],\"free\":" + String(store::freeBytes() / 1024) + ",\"max\":" + store::MAX_FACES +
       ",\"more\":" + (to < n_ ? "true" : "false");
  return j;
}

String stateJson() { return String("\"pkgFaces\":{\"n\":") + n_ + ",\"fmt\":[" + sfb::FORMAT + "]}"; }

int remove(const String& id) {
  const String saved = settings::face();
  const int si = faces::find(saved);
  const bool savedIsPackage = si >= 0 && isPackage(faces::at(si));
  int gone = 0;
  if (id == "all") {
    gone = store::wipe();
  } else {
    for (int i = 0; i < n_; i++)
      if (id == slots_[i].info.id) gone = store::remove(id.c_str()) ? 1 : 0;
  }
  if (!gone) return 0;
  if (crashed_.length() && (id == "all" || id == crashed_)) {
    crashed_ = "";
    guard_.remove("bad");
  }
  rescan();
  if (savedIsPackage && faces::find(saved) < 0) {  // the Home face is gone: Classic
    settings::saveFace("classic");
    faces::select("classic");
  }
  Log.printf("Faces: removed %s (%d)\n", id.c_str(), gone);
  return gone;
}

// --- The install toast -------------------------------------------------------------------------

void drawOverlay(bool wide) {
  String line;
  uint16_t bg;
  if (!install::toast(line, bg)) return;
  using namespace ui;
  const lgfx::IFont* f = fit(line, W - 20, {F12, F9});
  canvas.setFont(f);
  const int w = min(W - 8, (int)canvas.textWidth(line) + 16), y = H - (wide ? 20 : 24);
  canvas.fillRoundRect((W - w) / 2, y - 9, w, 19, 9, bg);
  txt(ellipsize(line, f, w - 12), W / 2, y, f, INK, middle_center);
}

uint32_t overlayFrameMs() {
  String line;
  uint16_t bg;
  return install::toast(line, bg) ? 250 : 0;
}

}  // namespace pkg
}  // namespace faces
