// Connections: the places StickyBot reads from to know about your day. Today one: the Mac's own
// Calendar (EventKit, read-only; connections/calendar.rs) -> the next meeting on the stick's home
// card and a reminder before it starts. Rust app only (the Python app doesn't serve this file).
//
// The card is built once and then updated in place: saving a setting, connecting or the 30 s refresh
// change the pieces that changed (the pill, the lists), never the whole page, so nothing jumps or
// loses its scroll. The plain logic (words, rows, what changed) is connections_logic.js.
import {h, clear, get, post, put, toast, card} from "/static/lib.js";
import {
  statusPill, permissionHelp, showSettingsLink, SETTINGS_LINK, remindOptions, eventRow, emptyDay,
  nextCalendars, settingsDiff, testAlertBody, REFRESH_MS,
} from "/static-rs/connections_logic.js";

export const title = "Connections";
export const subtitle = "Where StickyBot learns about your day. It only reads; nothing leaves this Mac except to your stick.";

let ui = null;     // the page's pieces while it is shown
let timer = null;

const dot = color => h("span", {style: `width:9px;height:9px;border-radius:50%;flex:none;background:${color || "var(--faint)"}`, "aria-hidden": "true"});

function build() {
  const pill = h("span", {class: "pill", id: "calStatus"}, h("span", {class: "dot"}), h("span", {class: "label"}));
  const detail = h("div", {class: "lead", id: "calDetail", style: "margin-bottom:0"});
  const connect = h("button", {class: "btn primary", type: "button", id: "calConnect", onclick: connectClick}, "Connect");
  const disconnect = h("button", {class: "btn", type: "button", id: "calDisconnect", onclick: disconnectClick}, "Disconnect");
  const permHelp = h("div", {class: "grow", id: "calPermission-text"});
  const settingsLink = h("a", {class: "btn sm", id: "calSettingsLink", href: SETTINGS_LINK}, "Open System Settings");
  const permission = h("div", {class: "notice warn", id: "calPermission", hidden: true, role: "status"}, permHelp, settingsLink);
  const errorText = h("div", {class: "grow", id: "calError-text"});
  const error = h("div", {class: "notice err", id: "calError", hidden: true, role: "status"}, errorText);

  const cals = h("div", {id: "calCalendars", style: "display:flex;flex-direction:column;gap:9px"});
  const skipAllDay = h("input", {type: "checkbox", id: "calSkipAllDay", onchange: () => save({skip_all_day: skipAllDay.checked})});
  const skipDeclined = h("input", {type: "checkbox", id: "calSkipDeclined", onchange: () => save({skip_declined: skipDeclined.checked})});
  const remind = h("select", {id: "calRemind", "aria-label": "Remind me", onchange: () => save({alert_min: Number(remind.value)})});
  const test = h("button", {class: "btn sm", type: "button", id: "calTest", onclick: testAlert}, "Send a test alert");

  const today = h("div", {id: "calToday"});
  const settings = h("div", {id: "calSettings", hidden: true},
    h("div", {class: "field"}, h("label", null, "Calendars"), cals,
      h("div", {class: "help"}, "Leave them all ticked to use every calendar on this Mac.")),
    h("div", {class: "field"}, h("label", {for: "calSkipAllDay"}, "All-day events"),
      h("label", {class: "switch"}, skipAllDay, h("span", null, "Skip all-day events")),
      h("div", {class: "help"}, "Birthdays, holidays and \"out of office\" blocks. They never count as a meeting.")),
    h("div", {class: "field"}, h("label", {for: "calSkipDeclined"}, "Declined"),
      h("label", {class: "switch"}, skipDeclined, h("span", null, "Skip meetings I declined"))),
    h("div", {class: "field"}, h("label", {for: "calRemind"}, "Remind me"), remind,
      h("div", {class: "help"}, "The stick shows a heads-up this long before a meeting starts.")),
    h("div", {class: "field"}, h("label", null, "Try it"), h("div", {class: "row"}, test),
      h("div", {class: "help"}, "Sends the sample reminder to the stick, like ▶ on the Events page.")),
    h("div", {style: "margin-top:18px"}, h("div", {class: "eyebrow", style: "margin-bottom:6px"}, "Today"), today));

  const head = h("div", {class: "row", style: "gap:10px;width:100%"}, h("h2", null, "Calendar"), pill);
  const el = h("div", {class: "stack", style: "display:flex;flex-direction:column;gap:18px"},
    card(head,
      detail,
      h("div", {class: "row", style: "margin:14px 0"}, connect, disconnect),
      permission, error, settings),
    h("p", {class: "small faint", id: "moreConnections", style: "margin:0"}, "More connections are coming."));
  return {el, pill, detail, connect, disconnect, permission, permHelp, settingsLink, error, errorText, settings, cals, skipAllDay,
    skipDeclined, remind, today, test, conn: null, calendars: [], calendarsKey: null, dayKey: null, busy: false};
}

// Shows `conn` (an item of GET /api/connections) without rebuilding anything.
function apply(conn) {
  const u = ui;
  if (!u || !conn) return;
  u.conn = conn;
  const {cls, label} = statusPill(conn.status);
  u.pill.className = "pill" + (cls ? " " + cls : "");
  u.pill.querySelector(".label").textContent = label;
  u.detail.textContent = conn.status === "off"
    ? "Shows your next meeting on the stick and reminds you before it starts. It only reads your calendar; it never changes it."
    : conn.detail || "";
  const on = conn.status === "connected";
  u.connect.hidden = on;
  u.disconnect.hidden = !on;
  u.connect.textContent = conn.status === "needs_permission" ? "Connect again" : "Connect";
  if (!u.busy) u.connect.disabled = false;
  const help = conn.status === "needs_permission" ? permissionHelp(conn.permission) : "";
  u.permission.hidden = !help;
  u.permHelp.textContent = help;
  u.settingsLink.hidden = !showSettingsLink(conn.permission);
  u.error.hidden = conn.status !== "error";
  u.errorText.textContent = conn.status === "error" ? `Couldn't read your calendar: ${conn.detail}` : "";
  u.settings.hidden = !on;
  const s = conn.settings || {};
  u.skipAllDay.checked = !!s.skip_all_day;
  u.skipDeclined.checked = !!s.skip_declined;
  const choices = remindOptions(s.alert_min);
  if (u.remind.options.length !== choices.length || choices.some((o, i) => u.remind.options[i].value !== String(o.value))) {
    clear(u.remind).append(...choices.map(o => h("option", {value: o.value}, o.label)));  // not while it is open and unchanged
  }
  u.remind.value = String(s.alert_min);
}

function paintCalendars(list) {
  const u = ui;
  if (!u) return;
  const key = JSON.stringify(list);
  if (key === u.calendarsKey) return;  // unchanged: leave the boxes (and their focus) alone
  u.calendarsKey = key;
  u.calendars = list;
  clear(u.cals);
  if (!list.length) { u.cals.appendChild(h("div", {class: "small faint"}, "No calendars found on this Mac.")); return; }
  for (const c of list) {
    const box = h("input", {type: "checkbox", checked: c.selected, "aria-label": c.title, onchange: () => {
      save({calendars: nextCalendars(ui.calendars, c.id, box.checked)});
    }});
    u.cals.appendChild(h("label", {class: "switch", style: "gap:9px"}, box, dot(c.color), h("span", null, c.title),
      c.source ? h("span", {class: "small faint"}, c.source) : null));
  }
}

function paintDay(day, events) {
  const u = ui;
  if (!u) return;
  const key = JSON.stringify([day, events]);
  if (key === u.dayKey) return;
  u.dayKey = key;
  clear(u.today);
  if (!events.length) { u.today.appendChild(h("div", {class: "empty"}, emptyDay(day))); return; }
  u.today.appendChild(h("div", {class: "list"}, events.map(e => {
    const r = eventRow(e);
    return h("div", {class: "item", style: r.dim ? "opacity:.6" : ""},
      h("span", {class: "small dim", style: "min-width:96px;font-variant-numeric:tabular-nums"}, r.time),
      dot(r.color),
      h("div", {class: "what"}, h("span", null, r.title), r.sub ? h("span", {class: "sub"}, r.sub) : null),
      r.tags.map(t => h("span", {class: "tag" + (t.cls ? " " + t.cls : ""), title: t.title || null}, t.text)));
  })));
}

// What is listed under the settings: the calendars and today's meetings (only when connected).
async function loadLists() {
  if (!ui || ui.conn?.status !== "connected") return;
  const [cals, day] = await Promise.all([get("/api/connections/calendar/calendars"), get("/api/connections/calendar/events?day=today")]);
  if (!ui) return;
  if (!cals.error) paintCalendars(cals.calendars || []);
  if (!day.error) paintDay("today", day.events || []);
}

async function refresh() {
  const r = await get("/api/connections");
  if (!ui) return;
  const conn = (r.connections || []).find(c => c.id === "calendar");
  if (r.error || !conn) {
    if (!ui.conn) ui.detail.textContent = r.error || "Connections aren't available in this version.";
    return;
  }
  apply(conn);
  await loadLists();
}

async function connectClick() {
  const u = ui;
  u.busy = true;
  u.connect.disabled = true;
  u.connect.textContent = "Asking macOS…";
  const r = await post("/api/connections/calendar/connect");
  u.busy = false;
  if (ui !== u) return;
  if (r.error) { toast(r.error, "err"); u.connect.disabled = false; u.connect.textContent = "Connect"; return; }
  apply(r);
  if (r.status === "connected") toast("Calendar connected");
  await loadLists();
}

async function disconnectClick() {
  const u = ui;
  u.disconnect.disabled = true;
  const r = await post("/api/connections/calendar/disconnect");
  u.disconnect.disabled = false;
  if (ui !== u) return;
  if (r.error) { toast(r.error, "err"); return; }
  apply(r);
  clear(u.today);
  u.dayKey = u.calendarsKey = null;
  toast("Calendar disconnected");
}

// Saves what changed; on a refusal the controls go back to what is saved.
async function save(wanted) {
  const u = ui;
  const change = settingsDiff(u.conn && u.conn.settings, wanted);
  if (!Object.keys(change).length) return;
  const r = await put("/api/connections/calendar", change);
  if (ui !== u) return;
  if (r.error) { toast(r.error, "err"); apply(u.conn); u.calendarsKey = null; await loadLists(); return; }
  apply(r);
  if ("calendars" in change || "skip_all_day" in change || "skip_declined" in change) await loadLists();
}

async function testAlert() {
  const r = await post("/api/events/test", testAlertBody());
  if (r.error) { toast(r.error, "err"); return; }
  toast(r.event ? "Sent a test alert to the stick" : "Meeting reminders are turned off (Events page)", r.event ? null : "err");
}

export function render(root) {
  ui = build();
  clear(root).appendChild(ui.el);
  refresh();
  timer = setInterval(refresh, REFRESH_MS);
}

export function leave() {
  ui = null;
  if (timer) clearInterval(timer);
  timer = null;
}
