// Settings → Brain (brains.py / brain_api.py): who answers voice requests — Claude Code (your
// subscription) or OpenRouter (your own key, pay per use). The key goes straight to the agent, which
// keeps it in the macOS Keychain; the page never gets it back (only "saved …abcd").
// The choice and the model are saved with the page's Save button (config "brain", "openrouter_model").
import {h, clear, get, put, post, api, field, report} from "./lib.js";

const CHOICES = [
  ["claude", "Claude Code", "Your Claude subscription, no key. Can also search the web and run commands on this Mac (with your approval)."],
  ["openrouter", "OpenRouter", "Your own OpenRouter API key, pay per use. Pick any model; it can use the stick tools (opening apps and URLs asks you first)."],
];

const price = v => v == null ? "?" : v === 0 ? "free" : `$${v < 1 ? +v.toPrecision(2) : +v.toFixed(2)}`;
const label = m => `${m.name} — ${price(m.in)} in / ${price(m.out)} out per M tokens${m.tools ? "" : " · no tools"}`;

export function brainPanel(c) {
  const picked = c.brain === "openrouter" ? "openrouter" : "claude";
  const orBox = h("div", {style: `display:${picked === "openrouter" ? "flex" : "none"};flex-direction:column;gap:12px;margin-top:12px`});
  const radios = h("div", {style: "display:flex;flex-direction:column;gap:8px"}, CHOICES.map(([v, name, help]) =>
    h("label", {style: "display:flex;gap:9px;align-items:flex-start;cursor:pointer"},
      h("input", {type: "radio", name: "brain", value: v, checked: picked === v, style: "margin-top:3px",
        onchange: () => { orBox.style.display = v === "openrouter" ? "flex" : "none"; if (v === "openrouter") models.load(); }}),
      h("span", null, h("b", null, name), h("div", {class: "small dim"}, help)))));

  orBox.append(
    h("div", {class: "notice warn"}, h("div", {class: "grow"},
      h("b", null, "Privacy: "),
      h("span", null, "with OpenRouter, what you say (and the conversation so far) is sent to OpenRouter and to the model's provider. Each request costs a little from your OpenRouter credits."))),
    keyField(),
    models.field(c),
    testField());
  return h("div", null, field("Answers from", radios), orBox);
}

// --- the key: write-only ------------------------------------------------------------------

function keyField() {
  const input = h("input", {type: "password", id: "orKey", autocomplete: "off", spellcheck: "false", placeholder: "sk-or-…", style: "max-width:320px"});
  const state = h("span", {class: "small dim"}, "Checking…");
  const remove = h("button", {class: "btn", type: "button", onclick: async () => {
    const r = await api("/api/brain/openrouter/key", {method: "DELETE"});
    if (report(r, "Key removed from the Keychain")) paint(r.key);
  }}, "Remove");
  const save = h("button", {class: "btn primary", type: "button", onclick: async () => {
    if (!input.value.trim()) { input.focus(); return; }
    const r = await put("/api/brain/openrouter/key", {key: input.value});
    if (report(r, "Key saved in the Keychain")) { input.value = ""; paint(r.key); }
  }}, "Save key");
  function paint(k) {
    if (!k) { state.textContent = ""; return; }
    remove.disabled = !k.saved || k.source === "env";
    state.textContent = k.source === "env" ? `Using ${k.env_var} from the environment (${k.hint})`
      : k.saved ? `Saved in the Keychain (${k.hint})` : "No key saved yet.";
  }
  get("/api/brain").then(b => paint(b && b.openrouter ? b.openrouter.key : null));
  return field("API key", h("div", {style: "display:flex;flex-direction:column;gap:6px"}, h("div", {class: "row"}, input, save, remove), state),
    h("span", null, "Create one at ", h("a", {href: "https://openrouter.ai/keys", target: "_blank", rel: "noopener"}, "openrouter.ai/keys"),
      ". It's kept in your Mac's Keychain, never in config.json, and this page can't read it back."));
}

// --- the model picker ---------------------------------------------------------------------

const models = {
  list: null, chosen: "", select: null, info: null, search: null, toolsOnly: null, loading: false,

  field(c) {
    this.list = null;
    this.chosen = c.openrouter_model || "";
    this.select = h("select", {id: "orModel", size: 8, style: "width:100%;max-width:560px",
      onchange: () => { this.chosen = this.select.value; this.describe(); }});
    this.search = h("input", {type: "search", placeholder: "Search models (name or id)", style: "max-width:320px", oninput: () => this.paint()});
    this.toolsOnly = h("input", {type: "checkbox", checked: true, onchange: () => this.paint()});
    this.info = h("div", {class: "small dim"});
    const reload = h("button", {class: "btn sm", type: "button", onclick: () => this.load(true)}, "Reload list");
    if (c.brain === "openrouter") this.load();
    else this.info.textContent = this.chosen ? `Model: ${this.chosen}` : "";
    return field("Model", h("div", {style: "display:flex;flex-direction:column;gap:6px"},
      h("div", {class: "row"}, this.search, h("label", {class: "small"}, this.toolsOnly, " can use tools"), reload),
      this.select, this.info),
      "Fast, cheap models suit short spoken answers. Prices are OpenRouter's, per million tokens; a voice request is usually well under 2,000 tokens.");
  },

  async load(refresh) {
    if (this.loading || (this.list && !refresh)) return;
    this.loading = true;
    this.info.textContent = "Loading the model list from OpenRouter…";
    const r = await get("/api/brain/openrouter/models" + (refresh ? "?refresh=1" : ""));
    this.loading = false;
    this.list = Array.isArray(r.models) ? r.models : [];
    if (!this.chosen && r.suggested) this.chosen = r.suggested;  // a suggestion: saved only with Save
    this.suggested = r.suggested;
    this.error = r.error;
    this.paint();
  },

  paint() {
    if (!this.select) return;
    const q = (this.search.value || "").trim().toLowerCase();
    const shown = (this.list || []).filter(m => (!this.toolsOnly.checked || m.tools || m.id === this.chosen)
      && (!q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q)));
    clear(this.select);
    if (this.chosen && !shown.some(m => m.id === this.chosen)) {
      const known = (this.list || []).find(m => m.id === this.chosen);
      this.select.appendChild(h("option", {value: this.chosen}, known ? label(known) : `${this.chosen} (not in the list)`));
    }
    for (const m of shown.slice(0, 300))
      this.select.appendChild(h("option", {value: m.id}, label(m) + (m.id === this.suggested ? " · suggested" : "")));
    this.select.value = this.chosen;
    this.describe();
  },

  describe() {
    const m = (this.list || []).find(x => x.id === this.chosen);
    clear(this.info);
    if (this.error) this.info.appendChild(h("div", null, `${this.error} ${this.list && this.list.length ? "(showing the last list)" : "Check the internet connection, then Reload."}`));
    if (!this.chosen) { this.info.appendChild(h("div", null, "Pick a model, then Save.")); return; }
    this.info.appendChild(h("div", null, h("span", {class: "mono"}, this.chosen),
      m ? ` · ${price(m.in)} / ${price(m.out)} per M tokens${m.context ? ` · ${Math.round(m.context / 1000)}k context` : ""}${m.tools ? "" : " · can't use the stick tools"}` : ""));
  },
};

export function pickedModel() { return models.chosen || ""; }

// --- one short test request -------------------------------------------------------------------

function testField() {
  const out = h("div", {class: "small dim"});
  const btn = h("button", {class: "btn", type: "button", onclick: async () => {
    btn.disabled = true;
    out.textContent = "Asking…";
    const r = await post("/api/brain/test", models.chosen ? {model: models.chosen} : {});
    btn.disabled = false;
    clear(out);
    if (!r.ok) { out.appendChild(h("span", null, r.error || "Failed.")); return; }
    const s = ms => ms == null ? "–" : `${(ms / 1000).toFixed(1)} s`;
    out.append(h("div", null, `“${r.reply}”`),
      h("div", null, `First words after ${s(r.first_token_ms)}, done after ${s(r.total_ms)}${r.cost != null ? ` · cost $${r.cost.toFixed(6)}` : ""}`));
  }}, "Test");
  return field("Try it", h("div", {style: "display:flex;flex-direction:column;gap:6px"}, h("div", {class: "row"}, btn), out),
    "One short request with the picked model (and the saved key), without the stick. Costs a fraction of a cent.");
}
