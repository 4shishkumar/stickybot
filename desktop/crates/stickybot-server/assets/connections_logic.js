// The Connections page's plain logic (connections.js draws it): the words for a status, the
// permission help, a meeting row, the reminder choices and what changed in the settings. No imports
// and no DOM, so `node --test` covers it (mac-agent/tests/js/connections_logic.test.mjs).

// status -> [the pill's colour class (app.css), what it says]
export const STATUS = {
  off: ["", "Off"],
  needs_permission: ["warn", "Needs permission"],
  connected: ["ok", "Connected"],
  error: ["err", "Error"],
};

export function statusPill(status) {
  const [cls, label] = STATUS[status] || ["", String(status || "Unknown")];
  return {cls, label};
}

// The macOS pane for Calendars; a link to it opens System Settings from the app window and from a browser.
export const SETTINGS_LINK = "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";

// What to tell the person when macOS has not given StickyBot the Calendar: `permission` is the API's word.
export function permissionHelp(permission) {
  switch (permission) {
    case "denied":
      return "StickyBot isn't allowed to read your calendar. Open System Settings → Privacy & Security → Calendars, turn on StickyBot (Full Access), then press Connect again.";
    case "restricted":
      return "This Mac doesn't allow StickyBot to read the calendar (a profile or Screen Time setting). Ask whoever manages this Mac, or check System Settings → Privacy & Security → Calendars.";
    case "write_only":
      return "StickyBot can only add events, and needs Full Access to read them. Open System Settings → Privacy & Security → Calendars, set StickyBot to Full Access, then press Connect again.";
    case "not_asked":
      return "macOS will ask for your permission when you press Connect. StickyBot only reads your calendar; it never changes it.";
    default:
      return "";
  }
}

// Whether the "Open System Settings" link makes sense: only once macOS has said no.
export function showSettingsLink(permission) {
  return permission === "denied" || permission === "restricted" || permission === "write_only";
}

// The reminder choices, in minutes before the start (0 = none). A saved value that is not listed
// (set through the API) stays in the list, so showing the page never changes it.
export const REMIND = [0, 1, 2, 5, 10, 15];

export function remindLabel(min) {
  if (!min) return "Off";
  return min === 1 ? "1 minute before" : `${min} minutes before`;
}

export function remindOptions(current) {
  const values = REMIND.includes(current) || !Number.isInteger(current) ? [...REMIND] : [...REMIND, current].sort((a, b) => a - b);
  return values.map(value => ({value, label: remindLabel(value)}));
}

// One meeting in the "Today" preview: `e` is an item of GET …/events.
export function eventRow(e) {
  const time = e.all_day ? "All day" : `${e.from}–${e.to}`;
  const tags = [];
  if (e.now) tags.push({text: "now", cls: "ok"});
  if (e.join) tags.push({text: "Join", cls: "agent", title: e.join});
  if (e.declined) tags.push({text: "declined", cls: ""});
  const where = [e.calendar, e.where].filter(Boolean).join(" · ");
  return {time, title: e.title || "(no title)", color: e.color || "", sub: where, tags, dim: !!e.declined};
}

export function emptyDay(day) {
  return day === "tomorrow" ? "Nothing tomorrow" : "Nothing today";
}

// The `calendars` setting after ticking or unticking one box. `list` is GET …/calendars (`selected` as
// shown). No box ticked would mean "none", which the app reads as every calendar: ticking all of them
// or none of them is saved as [] (all).
export function nextCalendars(list, id, checked) {
  const chosen = new Set(list.filter(c => c.selected).map(c => c.id));
  if (checked) chosen.add(id); else chosen.delete(id);
  const ids = list.filter(c => chosen.has(c.id)).map(c => c.id);
  return ids.length === 0 || ids.length === list.length ? [] : ids;
}

// Only what differs between the saved settings and the wanted ones (a PUT sends just that).
export function settingsDiff(saved, wanted) {
  const out = {};
  for (const [k, v] of Object.entries(wanted || {})) {
    const s = (saved || {})[k];
    const same = Array.isArray(v) && Array.isArray(s)
      ? v.length === s.length && v.every(x => s.includes(x))
      : v === s;
    if (!same) out[k] = v;
  }
  return out;
}

// The body of "Send a test alert": the Events page's sample for the meeting reminder.
export function testAlertBody() {
  return {kind: "meeting_soon"};
}

// How often the page looks again while it is open, in milliseconds (the meetings change, "now" moves).
export const REFRESH_MS = 30000;
