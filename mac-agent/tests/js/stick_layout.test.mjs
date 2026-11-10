// The Stick page's layout (static/pages/stick_layout.js): run with `node --test` (test_dashboard_js.py does).
import test from "node:test";
import assert from "node:assert/strict";
import {SECTIONS, layout, structureKey, slicesOf, changedCards, openSections, summary} from "../../stick_agent/static/pages/stick_layout.js";

const STATE = {  // a 2.5.1 stick, like the fake one
  name: "StickyBot", fw: "2.5.1", battery: 87, bt: "connected", combo: "Option+Cmd", mods: 12, key: 0, brightness: 50,
  standby: 10, autoRotate: true, landscape: false, clock24: false, devMode: false, enterDelay: 0, macPassSet: true,
  motion: {lift: true, face: true, shake: false}, face: "classic", faces: [{id: "classic", title: "Classic"}],
  slot: 1, slots: [{n: 1, paired: true}, {n: 2, paired: false}], launcher: "grapmfqs",
  air: {mode: "home", sens: 5}, connected: true,
};
const ctx = (st = STATE, extra = {}) => ({st, gamesInfo: [{id: "stack", title: "Stack", best: 3}], snd: {on: true, volume: 128},
  petInfo: {games: {xpToday: 4, cap: 10}}, keys: null, via: "ble", stickName: "StickyBot", ...extra});
const ids = c => layout(c).map(s => s.id);

test("sections come in priority order: screen first, games below the everyday settings, firmware last", () => {
  assert.deepEqual(ids(ctx()), ["status", "screen", "buttons", "sound", "motion", "remote", "games", "faces", "pairing", "firmware"]);
  assert.deepEqual(SECTIONS.map(s => s.id), ids(ctx()));
  const all = SECTIONS.flatMap(s => s.cards);
  assert.equal(new Set(all).size, all.length, "every card is in one section");
});

test("older firmware: the cards it can't do aren't there, nor their sections", () => {
  const old = {...STATE};
  for (const k of ["air", "launcher", "faces", "clock24"]) delete old[k];
  const l = layout(ctx(old, {gamesInfo: null}));
  assert.deepEqual(l.map(s => s.id), ["status", "screen", "buttons", "sound", "motion", "pairing", "firmware"]);
  assert.deepEqual(l.find(s => s.id === "buttons").cards, ["combo", "unlock"]);
  assert.deepEqual(l.find(s => s.id === "screen").cards, ["screen"]);
});

test("a stick that doesn't answer: the status card and the firmware only", () => {
  const l = layout(ctx(null, {gamesInfo: null, snd: null}));
  assert.deepEqual(l.map(s => [s.id, s.cards]), [["status", ["hero"]], ["firmware", ["firmware"]]]);
});

test("a saved setting redraws only the cards drawn from it", () => {
  const before = slicesOf(ctx());
  assert.deepEqual(changedCards(before, slicesOf(ctx({...STATE, brightness: 60}))), ["screen"]);
  assert.deepEqual(changedCards(before, slicesOf(ctx({...STATE, motion: {lift: true, face: true, shake: true}}))), ["gestures"]);
  assert.deepEqual(changedCards(before, slicesOf(ctx({...STATE, face: "aquarium"}))).sort(), ["face", "market"]);
  assert.deepEqual(changedCards(before, slicesOf(ctx({...STATE, battery: 80}))), ["hero"]);
  assert.deepEqual(changedCards(before, slicesOf(ctx())), []);
  assert.equal(structureKey(ctx()), structureKey(ctx({...STATE, brightness: 60})));
  assert.notEqual(structureKey(ctx()), structureKey(ctx(null)));
});

test("folded sections: defaults, then what the browser remembered (junk ignored)", () => {
  const d = openSections(null);
  assert.equal(d.screen, true);
  assert.equal(d.pairing, false);
  assert.equal(d.status, true);
  const saved = openSections({screen: false, pairing: true, status: false, games: "x"});
  assert.equal(saved.screen, false);
  assert.equal(saved.pairing, true);
  assert.equal(saved.status, true, "the status card can't be folded");
  assert.equal(saved.games, true);
});

test("section summaries", () => {
  assert.equal(summary("screen", ctx()), "brightness 50% · Classic face");
  assert.equal(summary("buttons", ctx()), "A holds Option + Cmd");
  assert.equal(summary("sound", ctx()), "on · volume 50%");
  assert.equal(summary("motion", ctx()), "lift to wake, face down");
  assert.equal(summary("remote", ctx({...STATE, air: {mode: "presenter"}})), "presenter on");
  assert.equal(summary("games", ctx()), "4 of 10 XP today");
  assert.equal(summary("pairing", ctx()), "StickyBot · on Mac 1 · 1 paired");
  assert.equal(summary("screen", ctx(null)), "");
});
