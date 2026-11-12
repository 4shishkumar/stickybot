/**
 * StickyBot feedback: a Google Apps Script web app bound to the team's Google Sheet.
 *
 * The StickyBot app (its Feedback page / "Report an issue…") POSTs one JSON report here; this
 * script checks it and appends a row to the "Feedback" sheet. No keys are involved: the app is
 * public, the web app takes anonymous posts ("Who has access: Anyone"), and it can only ever add
 * rows. Setup and triage: docs/FEEDBACK.md.
 *
 *   POST  {v, id, install, type, what, expected, steps, contact, app, fw, macos, stick,
 *          diagnostics, created, hp}            -> {ok: true, id} or {ok: false, error}
 *   GET                                         -> {ok: true, service: "stickybot-feedback"}
 */

var SHEET_NAME = "Feedback";
var HEADERS = [
  "Received", "ID", "Type", "What happened", "Expected", "Steps", "Contact", "App", "Firmware",
  "macOS", "Stick", "Diagnostics", "Status", "Area", "Owner", "Fix/Release", "Notes",
];
var TYPES = ["Bug", "Idea", "Question"];
var MAX_TEXT = 4000;        // what / expected / steps (characters)
var MAX_DIAGNOSTICS = 30000;
var MAX_SHORT = 200;        // contact, app, firmware, macOS, stick
var MAX_BODY = 60000;
var PER_INSTALL_HOUR = 10;  // reports an hour from one install (the app allows 5)
var ALL_HOUR = 300;         // reports an hour from everyone together

/** The health check: says nothing about the Sheet. */
function doGet() {
  return json_({ok: true, service: "stickybot-feedback"});
}

function doPost(e) {
  try {
    var raw = (e && e.postData && e.postData.contents) || "";
    if (raw.length > MAX_BODY) return json_({ok: false, error: "the report is too large"});
    var d;
    try {
      d = JSON.parse(raw);
    } catch (err) {
      return json_({ok: false, error: "not a JSON report"});
    }
    var bad = check_(d);
    if (bad) return json_({ok: false, error: bad});
    // the honeypot: the app always sends it empty; a form-filling bot doesn't. Look accepted, keep nothing.
    if (d.hp) return json_({ok: true, id: d.id});

    var lock = LockService.getScriptLock();
    lock.waitLock(15000);
    try {
      var cache = CacheService.getScriptCache();
      // the same report again (the app retried after a timeout that had in fact gone through)
      if (cache.get("id:" + d.id)) return json_({ok: true, id: d.id});
      if (!allow_(cache, "install:" + d.install, PER_INSTALL_HOUR) || !allow_(cache, "all", ALL_HOUR)) {
        return json_({ok: false, error: "too many reports right now — please try again in an hour"});
      }
      sheet_().appendRow([
        new Date(), d.id, d.type, cell_(d.what), cell_(d.expected), cell_(d.steps), cell_(d.contact),
        cell_(d.app), cell_(d.fw), cell_(d.macos), cell_(d.stick), cell_(d.diagnostics),
        "New", "", "", "", "",
      ]);
      cache.put("id:" + d.id, "1", 21600);
    } finally {
      lock.releaseLock();
    }
    return json_({ok: true, id: d.id});
  } catch (err) {
    console.error(err);
    return json_({ok: false, error: "the feedback service had a problem — please try again later"});
  }
}

/** What's wrong with a report, or "". */
function check_(d) {
  if (!d || typeof d !== "object" || Array.isArray(d)) return "not a report";
  if (d.v !== 1) return "unknown report version";
  if (!/^SB-[A-HJ-NP-Z2-9]{6}$/.test(String(d.id))) return "bad id";
  if (!/^[0-9a-f]{16}$/.test(String(d.install))) return "bad install id";
  if (TYPES.indexOf(d.type) < 0) return "type must be one of: " + TYPES.join(", ");
  var texts = {what: MAX_TEXT, expected: MAX_TEXT, steps: MAX_TEXT, contact: MAX_SHORT, app: MAX_SHORT,
               fw: MAX_SHORT, macos: MAX_SHORT, stick: MAX_SHORT, diagnostics: MAX_DIAGNOSTICS,
               created: 40, hp: MAX_SHORT};
  for (var k in texts) {
    var v = d[k];
    if (v === undefined || v === null) {
      d[k] = "";
      continue;
    }
    if (typeof v !== "string") return k + " must be text";
    if (v.length > texts[k]) return k + " is too long";
  }
  if (!d.what.trim()) return "what happened is empty";
  return "";
}

/** A fixed-hour counter in the cache -> may one more go? */
function allow_(cache, key, max) {
  var hour = Math.floor(Date.now() / 3600000);
  var k = "rl:" + key + ":" + hour;
  var n = Number(cache.get(k) || 0);
  if (n >= max) return false;
  cache.put(k, String(n + 1), 3700);
  return true;
}

/** The "Feedback" sheet, made with its header row if missing. */
function sheet_() {
  var ss = SpreadsheetApp.getActiveSpreadsheet();
  var sh = ss.getSheetByName(SHEET_NAME) || ss.insertSheet(SHEET_NAME);
  if (sh.getLastRow() === 0) {
    sh.appendRow(HEADERS);
    sh.setFrozenRows(1);
    sh.getRange(1, 1, 1, HEADERS.length).setFontWeight("bold");
  }
  return sh;
}

/** Text as a plain cell: never a formula (=, +, -, @ at the start). */
function cell_(s) {
  s = String(s || "");
  return /^[=+\-@]/.test(s) ? "'" + s : s;
}

function json_(o) {
  return ContentService.createTextOutput(JSON.stringify(o)).setMimeType(ContentService.MimeType.JSON);
}
