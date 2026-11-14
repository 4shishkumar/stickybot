// The Connections page's plain logic (the Rust app's desktop/crates/stickybot-server/assets/connections_logic.js):
// run with `node --test` (pytest runs it with the rest of the dashboard's JS tests).
import test from "node:test";
import assert from "node:assert/strict";
import {
  statusPill, permissionHelp, showSettingsLink, SETTINGS_LINK, remindOptions, remindLabel, eventRow, emptyDay,
  nextCalendars, settingsDiff, testAlertBody, REMIND, REFRESH_MS,
} from "../../../desktop/crates/stickybot-server/assets/connections_logic.js";

test("the status pill says what the API's four statuses mean", () => {
  assert.deepEqual(statusPill("off"), {cls: "", label: "Off"});
  assert.deepEqual(statusPill("needs_permission"), {cls: "warn", label: "Needs permission"});
  assert.deepEqual(statusPill("connected"), {cls: "ok", label: "Connected"});
  assert.deepEqual(statusPill("error"), {cls: "err", label: "Error"});
  // a status a newer app invents is shown as it is, not hidden
  assert.deepEqual(statusPill("paused"), {cls: "", label: "paused"});
  assert.deepEqual(statusPill(undefined), {cls: "", label: "Unknown"});
});

test("a denied permission says where to turn it on", () => {
  const denied = permissionHelp("denied");
  assert.match(denied, /isn't allowed to read your calendar/);
  assert.match(denied, /System Settings → Privacy & Security → Calendars/);
  assert.match(denied, /Full Access/);
  assert.match(denied, /press Connect again/);
  assert.match(permissionHelp("write_only"), /Full Access/);
  assert.match(permissionHelp("restricted"), /doesn't allow/);
  assert.match(permissionHelp("not_asked"), /never changes it/);
  assert.equal(permissionHelp(""), "");
  assert.equal(permissionHelp(undefined), "");
});

test("the System Settings link is offered only after macOS said no", () => {
  assert.equal(showSettingsLink("denied"), true);
  assert.equal(showSettingsLink("restricted"), true);
  assert.equal(showSettingsLink("write_only"), true);
  assert.equal(showSettingsLink("not_asked"), false);
  assert.equal(showSettingsLink(""), false);
  assert.match(SETTINGS_LINK, /^x-apple\.systempreferences:.*Privacy_Calendars$/);
});

test("the reminder choices: off, then minutes before; a saved odd value stays listed", () => {
  assert.deepEqual(remindOptions(5).map(o => o.value), REMIND);
  assert.deepEqual(remindOptions(5).map(o => o.label), [
    "Off", "1 minute before", "2 minutes before", "5 minutes before", "10 minutes before", "15 minutes before"]);
  assert.deepEqual(remindOptions(3).map(o => o.value), [0, 1, 2, 3, 5, 10, 15]);
  assert.deepEqual(remindOptions(120).map(o => o.value), [...REMIND, 120]);
  assert.equal(remindOptions(3).find(o => o.value === 3).label, "3 minutes before");
  assert.deepEqual(remindOptions(undefined).map(o => o.value), REMIND, "nothing saved: just the choices");
  assert.equal(remindLabel(0), "Off");
  assert.equal(remindLabel(1), "1 minute before");
});

test("a meeting row: time, title, where, and the tags", () => {
  const standup = {id: "s", title: "Standup", from: "10:30", to: "11:00", all_day: false, declined: false, now: true,
    calendar: "Work", color: "#4a7bd0", where: "", join: "Google Meet"};
  assert.deepEqual(eventRow(standup), {time: "10:30–11:00", title: "Standup", color: "#4a7bd0", sub: "Work",
    tags: [{text: "now", cls: "ok"}, {text: "Join", cls: "agent", title: "Google Meet"}], dim: false});
  const one = {title: "1:1 with a teammate", from: "16:00", to: "16:30", calendar: "Work", where: "Room 2", color: "#4a7bd0"};
  assert.deepEqual(eventRow(one), {time: "16:00–16:30", title: "1:1 with a teammate", color: "#4a7bd0", sub: "Work · Room 2", tags: [], dim: false});
  const all = eventRow({title: "Team offsite", all_day: true, from: "", to: "", calendar: "Home"});
  assert.equal(all.time, "All day");
  const declined = eventRow({title: "Planning", from: "12:00", to: "12:45", declined: true});
  assert.deepEqual(declined.tags, [{text: "declined", cls: ""}]);
  assert.equal(declined.dim, true);
  assert.equal(eventRow({from: "09:00", to: "09:15"}).title, "(no title)");
  assert.equal(eventRow({title: "x", from: "09:00", to: "09:15"}).color, "");
});

test("an empty day says so", () => {
  assert.equal(emptyDay("today"), "Nothing today");
  assert.equal(emptyDay("tomorrow"), "Nothing tomorrow");
});

test("ticking calendars: all or none ticked is saved as [] (every calendar)", () => {
  const all = [{id: "a", selected: true}, {id: "b", selected: true}, {id: "c", selected: true}];
  // from "all": unticking one picks the rest
  assert.deepEqual(nextCalendars(all, "b", false), ["a", "c"]);
  const two = [{id: "a", selected: true}, {id: "b", selected: false}, {id: "c", selected: true}];
  // ticking the last missing one means all again
  assert.deepEqual(nextCalendars(two, "b", true), []);
  // unticking down to one, and to none
  assert.deepEqual(nextCalendars(two, "c", false), ["a"]);
  assert.deepEqual(nextCalendars([{id: "a", selected: true}, {id: "b", selected: false}], "a", false), []);
  // order follows the list, not the clicks
  const some = [{id: "a", selected: false}, {id: "b", selected: false}, {id: "c", selected: true}];
  assert.deepEqual(nextCalendars(some, "a", true), ["a", "c"]);
  // a single calendar stays "all"
  assert.deepEqual(nextCalendars([{id: "a", selected: true}], "a", false), []);
});

test("saving sends only what changed", () => {
  const saved = {on: true, calendars: ["a", "b"], skip_all_day: true, skip_declined: true, alert_min: 5};
  assert.deepEqual(settingsDiff(saved, {alert_min: 10}), {alert_min: 10});
  assert.deepEqual(settingsDiff(saved, {alert_min: 5, skip_all_day: true}), {});
  assert.deepEqual(settingsDiff(saved, {skip_declined: false, alert_min: 5}), {skip_declined: false});
  // the same calendars in another order are no change
  assert.deepEqual(settingsDiff(saved, {calendars: ["b", "a"]}), {});
  assert.deepEqual(settingsDiff(saved, {calendars: ["a"]}), {calendars: ["a"]});
  assert.deepEqual(settingsDiff(saved, {calendars: []}), {calendars: []});
  assert.deepEqual(settingsDiff({calendars: []}, {calendars: []}), {});
  // nothing saved yet: everything wanted is a change
  assert.deepEqual(settingsDiff(undefined, {alert_min: 5}), {alert_min: 5});
  assert.deepEqual(settingsDiff(saved, undefined), {});
});

test("the test alert is the Events page's meeting reminder sample", () => {
  assert.deepEqual(testAlertBody(), {kind: "meeting_soon"});
  assert.ok(REFRESH_MS >= 10000 && REFRESH_MS <= 60000);
});
