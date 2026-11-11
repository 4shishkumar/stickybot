// Events: what the stick tells you about (GitHub, Claude Code), how loudly,
// quiet hours, and what happened recently.
import {h, clear, card, field, toggle, get, put, post, report, fmtTime, fmtDay, fmtAgo, store} from "../lib.js";
import {soundOptions, testBody, audition, serial} from "./events_sounds.js";

export const title = "Events";
export const subtitle = "When the stick celebrates, nudges you, or interrupts. ▶ sends a sample to the stick.";

const LABELS = {pr_merged: "PR merged", release: "Release (primary repo)", ci_fixed: "CI green again", pr_created: "PR opened",
  pr_approved: "PR approved", claude_done: "Claude finished", devin_review: "Devin review", review_requested: "Review requested",
  changes_requested: "Changes requested", ci_failed: "CI failed", claude_tool: "From a Claude session",
  update: "Update ready", test: "Test"};
const LEVEL = {celebrate: "Celebrate — confetti + jingle", nice: "Nice — toast + chime", headsup: "Heads-up — until you press a button", off: "Off"};
let root = null, saveBtn = null, kinds = null;

export function actions() {
  saveBtn = h("button", {class: "btn primary", type: "button", onclick: save}, "Save");
  return [saveBtn];
}

export async function render(el) {
  root = el;
  clear(root).appendChild(h("div", {class: "empty"}, "Loading…"));
  const [k, c, hist] = await Promise.all([get("/api/events/kinds"), get("/api/config"), get("/api/events/history?n=40")]);
  if (!root) return;
  clear(root);
  if (k.error || c.error) { root.appendChild(h("div", {class: "notice err"}, k.error || c.error)); return; }
  kinds = k;
  const rules = h("div", {class: "list", id: "evRules"});
  for (const kind of Object.keys(k.kinds || {})) rules.appendChild(ruleRow(kind));
  const g = c.github || {}, q = c.quiet_hours || {}, gs = store.status.github || {};
  root.append(
    card("What the stick shows",
      h("p", {class: "lead"}, "A level and a sound for each event, saved as you pick them. ▶ sends a sample as shown.",
        k.tunes_known ? null : h("span", {class: "dim"}, " (The stick hasn't listed its sounds yet: the celebrations need firmware 2.7.0; older firmware plays the level's sound.)")),
      rules),
    card("GitHub",
      field("Watch my PRs", toggle("ghOn", "Uses your gh login, read-only", g.on !== false)),
      field("Check every", h("div", {class: "row"}, h("input", {type: "number", id: "ghEvery", min: 30, max: 3600, value: g.interval_s || 120, style: "width:90px"}), h("span", {class: "dim"}, "seconds"))),
      field("Primary repo", h("input", {id: "ghRepo", class: "mono", placeholder: "owner/repo", value: g.primary_repo || ""}), "Review requests are only watched here; it's listed first."),
      field("Status", h("span", {class: gs.error ? "tag err" : gs.ok ? "tag ok" : "tag"},
        gs.error ? gs.error : gs.ok ? `${gs.tracked} PRs tracked · checked ${fmtAgo(gs.ok)}` : "not checked yet"))),
    card("Claude Code",
      field("Claude finished", h("div", {class: "row"}, h("span", {class: "dim"}, "after tasks of at least"),
        h("input", {type: "number", id: "evClaudeMin", min: 0, max: 3600, value: c.claude_done_min_s ?? 120, style: "width:90px"}), h("span", {class: "dim"}, "seconds")),
        "Needs the Claude Code hooks (UserPromptSubmit + Stop) — see the agent README.")),
    card("Quiet hours",
      field("Quiet hours", toggle("qOn", "No sounds in this window; events still show on the screen", !!q.on)),
      field("From – to", h("div", {class: "row"}, h("input", {type: "time", id: "qFrom", value: q.from || "19:00"}), h("span", {class: "dim"}, "to"), h("input", {type: "time", id: "qTo", value: q.to || "09:00"})))),
    card("Recent", historyList(Array.isArray(hist) ? hist : [])));
}

export function leave() { root = null; }

// One event kind: its level and sound (each saved at once, nothing redrawn) and ▶ (a sample with
// the level and sound shown, saved or not).
function ruleRow(kind) {
  const label = LABELS[kind] || kind;
  const level = h("select", {"data-kind": kind, "data-saved": kinds.rules[kind], class: "ev-level", id: "ev-" + kind, "aria-label": `${label}: level`},
    (kinds.levels || []).map(l => h("option", {value: l, selected: kinds.rules[kind] === l}, LEVEL[l] || l)));
  const saved = (kinds.sounds || {})[kind] ?? "";
  const sound = h("select", {"data-kind": kind, "data-saved": saved, class: "ev-sound", id: "evs-" + kind, "aria-label": `${label}: sound`});
  const play = h("button", {class: "btn icon", type: "button", "aria-label": `Send a sample: ${label}`, title: "Send a sample to the stick",
    onclick: async () => report(await post("/api/events/test", testBody(kind, level.value, sound.value)), "Sent to the stick")}, "▶");
  const fillSounds = () => {
    const v = sound.options.length ? sound.value : saved;
    clear(sound).append(...soundOptions(kinds.tunes, v, level.value, kinds.level_sounds)
      .map(o => h("option", {value: o.value, selected: o.value === v}, o.label)));
    sound.disabled = play.disabled = level.value === "off";
  };
  fillSounds();
  level.addEventListener("change", async () => {
    fillSounds();  // "Level default (…)" names the new level's sound
    if (!await saveNow(level, "event_rules", ".ev-level")) fillSounds();
  });
  sound.addEventListener("change", async () => {
    if (!await saveNow(sound, "event_sounds", ".ev-sound")) return;
    const name = audition(sound.value, level.value, kinds.level_sounds);
    if (name) post("/api/sounds/play", {name});  // hear it (just the sound, no event screen)
  });
  return h("div", {class: "item ev-row"}, h("div", {class: "what"}, h("b", null, label)), level, sound, play);
}

const putConfig = serial(body => put("/api/config", body));

// Saves the whole map `key` from the page's `cls` selects; on an error the select goes back.
async function saveNow(sel, key, cls) {
  const map = {};
  for (const s of document.querySelectorAll(`#evRules ${cls}`)) map[s.dataset.kind] = s.value;
  const r = await putConfig({[key]: map});
  if (report(r, "Saved")) { sel.dataset.saved = sel.value; return true; }
  sel.value = sel.dataset.saved;
  return false;
}

function historyList(hist) {
  if (!hist.length) return h("div", {class: "empty"}, "Nothing yet. GitHub and Claude Code events show up here.");
  const list = h("div", {class: "list"});
  let day = null;
  for (const e of hist) {
    const d = e.t ? fmtDay(e.t) : null;
    if (d && d !== day) { list.appendChild(h("div", {class: "eyebrow", style: "padding-top:10px"}, d)); day = d; }
    list.appendChild(h("div", {class: "item"},
      h("span", {class: "tag lvl-" + e.level}, e.level === "celebrate" ? "Celebrate" : e.level === "headsup" ? "Heads-up" : "Nice"),
      h("div", {class: "what"}, h("b", null, e.title || ""), e.sub ? h("span", {class: "sub"}, e.sub) : null),
      h("span", {class: "small faint num"}, e.t ? fmtTime(e.t) : "")));
  }
  return list;
}

async function save() {
  const v = id => document.getElementById(id);
  const rules = {}, sounds = {};
  for (const s of document.querySelectorAll("#evRules .ev-level")) rules[s.dataset.kind] = s.value;
  for (const s of document.querySelectorAll("#evRules .ev-sound")) sounds[s.dataset.kind] = s.value;
  const r = await putConfig({
    event_rules: rules,
    event_sounds: sounds,
    claude_done_min_s: +v("evClaudeMin").value || 0,
    github: {on: v("ghOn").checked, interval_s: Math.max(30, +v("ghEvery").value || 120), primary_repo: v("ghRepo").value.trim()},
    quiet_hours: {on: v("qOn").checked, from: v("qFrom").value || "19:00", to: v("qTo").value || "09:00"},
  });
  report(r, "Event settings saved");
}
