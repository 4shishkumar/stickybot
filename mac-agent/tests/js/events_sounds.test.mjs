// The Events page's level + sound per kind (static/pages/events_sounds.js): run with `node --test`.
import test from "node:test";
import assert from "node:assert/strict";
import {soundOptions, testBody, audition, serial} from "../../stick_agent/static/pages/events_sounds.js";

const LEVEL_SOUNDS = {celebrate: "jingle_win", nice: "confirmation_002", headsup: "question_001"};
const TUNES = ["drop_002", "confirmation_002", "question_001", "jingle_win", "applause", "clap", "cheer", "party_horn",
  "tada", "drumroll", "level_up", "coins", "off", "beep-1"];

test("the list: the level's own sound (named), off, then the celebrations first", () => {
  const o = soundOptions(TUNES, "applause", "celebrate", LEVEL_SOUNDS);
  assert.deepEqual(o.slice(0, 2), [{value: "", label: "Level default (Victory jingle)"}, {value: "off", label: "Off (silent)"}]);
  assert.deepEqual(o.slice(2, 10).map(x => x.value), ["applause", "clap", "cheer", "party_horn", "tada", "drumroll", "level_up", "coins"]);
  assert.equal(o.filter(x => x.value === "off").length, 1, "the stick's own 'off' isn't listed twice");
  assert.ok(o.some(x => x.value === "beep-1" && x.label === "beep-1"));
  assert.equal(o.find(x => x.value === "applause").label, "Applause (applause)");
  assert.equal(soundOptions(TUNES, "", "headsup", LEVEL_SOUNDS)[0].label, "Level default (Question)");
});

test("a saved sound the stick doesn't have stays listed, marked", () => {
  const o = soundOptions(["jingle_win"], "applause", "celebrate", LEVEL_SOUNDS);
  assert.deepEqual(o.at(-1), {value: "applause", label: "applause (not on this stick)"});
  assert.equal(soundOptions(["jingle_win"], "off", "nice", LEVEL_SOUNDS).length, 3);
});

test("▶ sends the level and sound shown (saved or not)", () => {
  assert.deepEqual(testBody("pr_merged", "headsup", "tada"), {kind: "pr_merged", level: "headsup", sound: "tada"});
  assert.deepEqual(testBody("ci_failed", "nice", undefined), {kind: "ci_failed", level: "nice", sound: ""});
});

test("auditioning a sound: the level's own for the default, nothing for off", () => {
  assert.equal(audition("cheer", "nice", LEVEL_SOUNDS), "cheer");
  assert.equal(audition("", "nice", LEVEL_SOUNDS), "confirmation_002");
  assert.equal(audition("off", "nice", LEVEL_SOUNDS), null);
});

test("saves run one at a time, in order, and a failure doesn't stop the next", async () => {
  const done = [];
  const save = serial(async (n, ms) => {
    await new Promise(r => setTimeout(r, ms));
    if (n === 2) throw new Error("nope");
    done.push(n);
    return n;
  });
  const a = save(1, 30), b = save(2, 1), c = save(3, 1);
  assert.equal(await a, 1);
  await assert.rejects(b);
  assert.equal(await c, 3);
  assert.deepEqual(done, [1, 3]);
});
