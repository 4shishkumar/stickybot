// Events: what the stick tells you about (GitHub, Claude Code), how loudly,
// quiet hours, and what happened recently.
import {h, clear, card, field, toggle, get, put, post, report, fmtTime, fmtDay, fmtAgo, store} from "../lib.js";

export const title = "Events";
export const subtitle = "When the stick celebrates, nudges you, or interrupts. ▶ sends a sample to the stick.";

const LABELS = {pr_merged: "PR merged", release: "Release (primary repo)", ci_fixed: "CI green again", pr_created: "PR opened",
  pr_approved: "PR approved", claude_done: "Claude finished", devin_review: "Devin review", review_requested: "Review requested",
  changes_requested: "Changes requested", ci_failed: "CI failed", test: "Test"};
const LEVEL = {celebrate: "Celebrate — confetti + jingle", nice: "Nice — toast + chime", headsup: "Heads-up — until you press a button", off: "Off"};
let root = null, saveBtn = null;

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
  const rules = h("div", {class: "list", id: "evRules"});
  for (const kind of Object.keys(k.kinds || {})) {
    const sel = h("select", {"data-kind": kind, id: "ev-" + kind, "aria-label": LABELS[kind] || kind},
      (k.levels || []).map(l => h("option", {value: l, selected: (k.rules || {})[kind] === l}, LEVEL[l] || l)));
    rules.appendChild(h("div", {class: "item"},
      h("div", {class: "what"}, h("b", null, LABELS[kind] || kind)), sel,
      h("button", {class: "btn icon", "aria-label": "Send a sample", onclick: async () => report(await post("/api/events/test", {kind}), "Sent to the stick")}, "▶")));
  }
  const g = c.github || {}, q = c.quiet_hours || {}, gs = store.status.github || {};
  root.append(
    card("What the stick shows", rules),
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
  const rules = {};
  for (const s of document.querySelectorAll("#evRules select")) rules[s.dataset.kind] = s.value;
  const r = await put("/api/config", {
    event_rules: rules,
    claude_done_min_s: +v("evClaudeMin").value || 0,
    github: {on: v("ghOn").checked, interval_s: Math.max(30, +v("ghEvery").value || 120), primary_repo: v("ghRepo").value.trim()},
    quiet_hours: {on: v("qOn").checked, from: v("qFrom").value || "19:00", to: v("qTo").value || "09:00"},
  });
  report(r, "Event settings saved");
}
