// Quick commands: spoken phrase -> action, no AI. Edited as a form (or as JSON),
// saved to ~/.stick-agent/commands.json through PUT /api/commands.
import {h, clear, get, put, post, report, toast} from "../lib.js";

export const title = "Quick commands";
export const subtitle = "Phrases that run straight away, without Claude. {n} is a number, {app} and {q} are any words.";

const ACTIONS = [["open_url", "Open a link"], ["open_app", "Open an app"], ["shell", "Run a command"]];
let doc = null, list = null, jsonMode = false, dirty = false, saveBtn = null, jsonBox = null;

export function actions() {
  saveBtn = h("button", {class: "btn primary", type: "button", disabled: true, onclick: save}, "Save");
  return [
    h("button", {class: "btn", type: "button", onclick: () => { if (syncFromJson()) { jsonMode = !jsonMode; paint(); } }}, "Edit as JSON"),
    saveBtn,
  ];
}

export async function render(root) {
  const testIn = h("input", {id: "cmdTry", placeholder: "Type what you'd say, e.g. “pull request twelve thirty four”", autocomplete: "off"});
  const verdict = h("div", {class: "verdict dim"}, "Matches show here as you type.");
  let tmr = null;
  testIn.addEventListener("input", () => {
    clearTimeout(tmr);
    tmr = setTimeout(async () => {
      const text = testIn.value.trim();
      if (!text) { verdict.replaceChildren("Matches show here as you type."); return; }
      const r = await post("/api/commands/test", {text});
      if (r.error) { verdict.replaceChildren(r.error); return; }
      const parts = [h("span", {class: "dim"}, "Heard as"), h("code", null, r.normalized), h("span", {class: "dim"}, "→")];
      if (r.match) {
        parts.push(h("span", {class: "tag agent"}, r.match.name));
        for (const [k, v] of Object.entries(r.match.slots || {})) parts.push(h("span", {class: "tag"}, `${k} = ${v}`));
      } else parts.push(h("span", {class: "tag"}, "no quick command — goes to Claude"));
      verdict.replaceChildren(...parts);
    }, 250);
  });
  root.appendChild(h("section", {class: "card tester"}, h("h2", null, "Try a phrase"), testIn, verdict,
    h("p", {class: "small faint"}, "This tests your saved commands. Save first to test changes.")));
  list = h("div", {style: "display:flex;flex-direction:column;gap:12px"});
  root.appendChild(list);
  const res = await get("/api/commands");
  if (res.error) { list.appendChild(h("div", {class: "notice err"}, res.error)); return; }
  doc = res;
  if (!Array.isArray(doc.commands)) doc.commands = [];
  dirty = false;
  paint();
}

export function leave() {
  if (dirty) toast("Unsaved changes to quick commands were left behind.", "err");
  list = null;
}

function markDirty() { dirty = true; if (saveBtn) saveBtn.disabled = false; }

function actionOf(c) { return ACTIONS.find(([k]) => k in c)?.[0] || "open_url"; }

function paint() {
  if (!list) return;
  clear(list);
  if (jsonMode) {
    jsonBox = h("textarea", {id: "cmdJson", rows: 24, spellcheck: "false", oninput: markDirty});
    jsonBox.value = JSON.stringify(doc, null, 2);
    list.append(h("section", {class: "card"}, h("div", {class: "card-head"}, h("h2", null, "commands.json"),
      h("div", {class: "actions"}, h("button", {class: "btn sm", onclick: () => { if (syncFromJson()) { jsonMode = false; paint(); } }}, "Back to the form"))),
      jsonBox));
    return;
  }
  doc.commands.forEach((c, i) => list.appendChild(cmdCard(c, i)));
  list.appendChild(h("button", {class: "btn", style: "align-self:flex-start", type: "button", onclick: () => {
    doc.commands.push({name: "New command", phrases: ["open my notes"], open_url: "https://"});
    markDirty(); paint();
  }}, "+ Add a command"));
}

function syncFromJson() {
  if (!jsonMode || !jsonBox) return true;
  try {
    const v = JSON.parse(jsonBox.value);
    if (!Array.isArray(v.commands)) throw new Error('expected {"commands": [...]}');
    doc = v;
    return true;
  } catch (e) { toast("That JSON isn't valid: " + e.message, "err"); return false; }
}

function cmdCard(c, i) {
  const act = actionOf(c);
  const name = h("input", {class: "name grow", id: `cmdName${i}`, value: c.name || "", "aria-label": "Command name",
    oninput: e => { c.name = e.target.value; markDirty(); }});
  const move = (d) => { const j = i + d; if (j < 0 || j >= doc.commands.length) return;
    [doc.commands[i], doc.commands[j]] = [doc.commands[j], doc.commands[i]]; markDirty(); paint(); };
  const top = h("div", {class: "top"}, name,
    h("button", {class: "btn icon ghost", title: "Move up (checked first)", "aria-label": "Move up", disabled: i === 0, onclick: () => move(-1)}, "↑"),
    h("button", {class: "btn icon ghost", title: "Move down", "aria-label": "Move down", disabled: i === doc.commands.length - 1, onclick: () => move(1)}, "↓"),
    h("button", {class: "btn sm danger", onclick: () => { doc.commands.splice(i, 1); markDirty(); paint(); }}, "Delete"));

  // phrases as chips
  const chips = h("div", {class: "chips"});
  const paintChips = () => {
    clear(chips);
    (c.phrases || []).forEach((p, k) => chips.appendChild(h("span", {class: "chip"},
      h("span", {html: p.replace(/[&<>]/g, "").replace(/\{(\w+)\}/g, "<b>{$1}</b>")}),
      h("button", {type: "button", "aria-label": `Remove “${p}”`, onclick: () => { c.phrases.splice(k, 1); markDirty(); paintChips(); }}, "×"))));
    const add = h("input", {placeholder: "+ add a phrase", "aria-label": "Add a phrase", onkeydown: e => {
      if (e.key === "Enter" && e.target.value.trim()) {
        c.phrases = c.phrases || [];
        c.phrases.push(e.target.value.trim().toLowerCase());
        markDirty(); paintChips();
        chips.querySelector("input").focus();
      }
    }});
    chips.appendChild(add);
  };
  paintChips();

  const sel = h("select", {id: `cmdAct${i}`, "aria-label": "Action", onchange: e => {
    const target = c[act] || "";
    for (const [k] of ACTIONS) delete c[k];
    c[e.target.value] = target;
    if (e.target.value !== "open_url") delete c.browser;
    markDirty(); paint();
  }}, ACTIONS.map(([k, label]) => h("option", {value: k, selected: k === act}, label)));
  const placeholder = {open_url: "https://github.com/<org>/<repo>/pull/{n}", open_app: "{app}  or  Slack", shell: "say hello {q}"}[act];
  const target = h("input", {id: `cmdTarget${i}`, class: "mono", value: c[act] || "", placeholder, "aria-label": "Target",
    oninput: e => { c[act] = e.target.value; markDirty(); }});
  const browser = act === "open_url"
    ? h("input", {id: `cmdBrowser${i}`, value: c.browser || "", placeholder: "Browser (default)", "aria-label": "Browser",
        oninput: e => { if (e.target.value) c.browser = e.target.value; else delete c.browser; markDirty(); }})
    : h("span", {class: "small faint", style: "align-self:center"}, act === "shell" ? "Runs in a shell; output shows on the stick." : "Spoken app names are matched to installed apps.");

  return h("section", {class: "cmd-card"}, top,
    h("div", null, h("div", {class: "eyebrow", style: "margin-bottom:6px"}, "When I say"), chips),
    h("div", null, h("div", {class: "eyebrow", style: "margin-bottom:6px"}, "Do this"), h("div", {class: "cmd-grid"}, sel, target, browser)));
}

async function save() {
  if (!syncFromJson()) return;
  const r = await put("/api/commands", doc);
  if (report(r, "Quick commands saved")) { dirty = false; if (saveBtn) saveBtn.disabled = true; }
}
