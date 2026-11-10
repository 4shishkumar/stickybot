// Remote: the stick's infrared remote (stick firmware 2.5.0+). The remotes learned on the stick with
// their buttons (send, star, rename, delete), learning a new button with live status, built-in TV
// code sets that work without learning, and import / export of everything as JSON.
import {h, clear, card, field, toggle, get, post, report, toast, store, ask} from "../lib.js";

export const title = "Remote";
export const subtitle = "The stick as an infrared remote: learn your TV / AC / soundbar buttons, send them from here, the stick or by voice.";

let root = null, data = null, off = null, learnBox = null;

export function actions() {
  return [
    h("button", {class: "btn", type: "button", onclick: importFile}, "Import…"),
    h("a", {class: "btn", href: "/api/ir/export", download: "stickybot-remotes.json"}, "Export"),
  ];
}

export async function render(el) {
  root = el;
  off = store.on((type, ev) => { if (type === "event" && ev.kind === "ir") onLearn(ev); });
  await load();
}

export function leave() { if (off) off(); off = null; root = null; }

async function load() {
  if (!root) return;
  const r = await get("/api/ir");
  if (!root) return;
  data = r;
  clear(root);
  if (r.error) { root.appendChild(h("div", {class: "notice err"}, r.error)); return; }
  if (!r.supported) {
    root.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"},
      h("b", null, r.connected ? "This stick can't be a remote yet" : "The stick isn't connected"),
      h("span", {class: "dim"}, r.why || ""))));
  } else {
    root.appendChild(learnCard());
    root.appendChild(remotesCard(r));
  }
  root.appendChild(brandsCard(r));
  if (r.supported) root.appendChild(diagCard());
}

// ---------- learning ----------
function learnCard() {
  const dev = h("input", {id: "irLearnDev", placeholder: "tv", value: "tv", autocomplete: "off"});
  const btn = h("input", {id: "irLearnBtn", placeholder: "power (empty: button 1, 2…)", autocomplete: "off"});
  let once = false;
  learnBox = h("div", {class: "verdict dim"}, "Point the remote at the top of the stick (where the IR window is), about 5–20 cm away.");
  const start = h("button", {class: "btn primary", type: "button", onclick: async () => {
    const r = await post("/api/ir/learn", {dev: dev.value, btn: btn.value, once});
    if (report(r)) showLearn({state: "learning", step: 1, dev: r.dev, btn: r.btn});
  }}, "Learn");
  const cancel = h("button", {class: "btn", type: "button", onclick: async () => report(await post("/api/ir/learn/cancel"), "Cancelled")}, "Cancel");
  return card("Learn a button",
    h("div", {class: "row"}, field("Remote", dev), field("Button", btn)),
    field("", toggle("irOnce", "Air conditioner / state remote: one press is enough", false, e => { once = e.target.checked; }),
      "AC remotes send their whole state (mode, temperature…) with every press, so set the remote the way you want it (e.g. Cool 24) and press any button once."),
    h("div", {class: "row"}, start, cancel), learnBox);
}

const LEARN_TEXT = {
  learning: e => e.step === 2 ? "Got it. Press the same button again." : `Press the button on the remote now (${e.dev}${e.btn ? " / " + e.btn : ""}).`,
  mismatch: () => "That didn't match the first press. Press it again.",
  saved: e => `Saved ${e.dev} / ${e.btn} (${e.p === "raw" ? "raw timings, " + e.n + " pulses" : e.p}).`,
  timeout: () => "Nothing heard for 30 s. Is the remote pointed at the stick's top?",
  error: e => `Couldn't save: ${e.error}.`,
  cancelled: () => "Cancelled.",
};

function showLearn(e) {
  if (!learnBox || !LEARN_TEXT[e.state]) return;
  learnBox.className = "verdict " + (e.state === "saved" ? "ok" : e.state === "learning" ? "" : "dim");
  learnBox.replaceChildren(h("span", {class: "tag " + (e.state === "saved" ? "ok" : e.state === "learning" ? "agent" : "ask")}, e.state), " ", LEARN_TEXT[e.state](e));
}

function onLearn(ev) {
  showLearn(ev);
  if (ev.state === "saved") { toast(`Learned ${ev.dev} / ${ev.btn}`); load(); }
}

// ---------- the remotes on the stick ----------
function remotesCard(r) {
  const box = h("div", {style: "display:flex;flex-direction:column;gap:14px"});
  if (!r.devices.length) box.appendChild(h("div", {class: "empty"}, "Nothing learned yet. Learn a button above, or install a code set below."));
  for (const d of r.devices) {
    const rows = h("div", {class: "list"});
    for (const b of d.buttons) {
      rows.appendChild(h("div", {class: "item"},
        h("button", {class: "btn icon ghost", title: b.fav ? "Unstar" : "Star (shown first on the stick)", "aria-label": "Favourite",
          onclick: async () => { if (report(await post("/api/ir/fav", {dev: d.id, btn: b.name, on: !b.fav}))) load(); }}, b.fav ? "★" : "☆"),
        h("div", {class: "what"}, h("b", null, b.name), h("span", {class: "sub"}, b.proto)),
        h("button", {class: "btn sm primary", onclick: async () => report(await post("/api/ir/send", {dev: d.id, btn: b.name}), "Sent")}, "Send"),
        h("button", {class: "btn sm", onclick: async () => {
          const to = await ask(`Rename “${b.name}” to:`, {value: b.name, ok: "Rename"});
          if (to && to !== b.name && report(await post("/api/ir/rename", {dev: d.id, btn: b.name, to}), "Renamed")) load();
        }}, "Rename"),
        h("button", {class: "btn sm danger", onclick: async () => {
          if (await ask(`Delete “${b.name}” from ${d.id}?`, {ok: "Delete", danger: true}) && report(await post("/api/ir/remove", {dev: d.id, btn: b.name}), "Deleted")) load();
        }}, "Delete")));
    }
    const brand = (data.mapped || {})[d.id];
    box.appendChild(h("section", {class: "cmd-card"},
      h("div", {class: "top"}, h("b", {class: "grow"}, d.id), h("span", {class: "small faint"}, `${d.buttons.length} buttons` + (brand ? ` · falls back to ${data.brands[brand]?.title || brand}` : "")),
        h("button", {class: "btn sm", onclick: async () => {
          const to = await ask(`Rename the remote “${d.id}” to:`, {value: d.id, ok: "Rename"});
          if (to && to !== d.id && report(await post("/api/ir/rename", {dev: d.id, to}), "Renamed")) load();
        }}, "Rename"),
        h("button", {class: "btn sm danger", onclick: async () => {
          if (await ask(`Delete the remote “${d.id}” and all its buttons?`, {ok: "Delete", danger: true}) && report(await post("/api/ir/remove", {dev: d.id}), "Deleted")) load();
        }}, "Delete")),
      rows));
  }
  const room = r.max ? ` · ${r.devices.length} of ${r.max} remotes, ${r.free ?? "?"} KB free` : "";
  return card(h("div", {class: "row grow"}, h("h2", null, "On the stick"), h("span", {class: "small faint"}, room)),
    h("p", {class: "lead"}, "On the stick: menu → Remote. A sends (hold to repeat), B next, B-B back one, B-B-B stars, hold B goes back."), box);
}

// ---------- built-in code sets ----------
function brandsCard(r) {
  const list = h("div", {style: "display:flex;flex-direction:column;gap:12px"});
  for (const [id, b] of Object.entries(r.brands || {})) {
    const dev = h("input", {value: "tv", "aria-label": "Remote name", style: "width:90px"});
    list.appendChild(h("section", {class: "cmd-card"},
      h("div", {class: "top"}, h("b", {class: "grow"}, b.title),
        Object.entries(r.mapped || {}).filter(([, v]) => v === id).map(([k]) => h("span", {class: "tag agent"}, k))),
      h("div", {class: "chips"}, b.buttons.map(btn => h("button", {class: "btn sm", title: "Send it from the stick (test)",
        onclick: async () => report(await post("/api/ir/send", {brand: id, btn}), "Sent")}, btn))),
      h("div", {class: "row"}, h("span", {class: "small dim"}, "Use for remote"), dev,
        h("button", {class: "btn sm", onclick: async () => {
          if (report(await post("/api/ir/brand", {dev: dev.value, brand: id}), `“${dev.value}” uses ${b.title} codes`)) load();
        }}, "Use for voice"),
        r.supported ? h("button", {class: "btn sm", onclick: async () => {
          if (report(await post("/api/ir/brand", {dev: dev.value, brand: id, install: true}), `Added to the stick as “${dev.value}”`)) load();
        }}, "Add to the stick") : null)));
  }
  const maps = Object.entries(r.mapped || {});
  return card("Built-in TV codes",
    h("p", {class: "lead"}, "Well-known codes for a few TV brands, no learning needed. Test a button first: if your TV reacts, use it for the voice commands (“turn off the TV”, “TV volume up”, “mute the TV”) or add it to the stick's Remote screen."),
    maps.length ? h("div", {class: "row"}, maps.map(([k, v]) => h("span", {class: "chip"}, `${k} → ${r.brands[v]?.title || v}`,
      h("button", {type: "button", "aria-label": "Stop using it", onclick: async () => { if (report(await post("/api/ir/brand", {dev: k, brand: ""}))) load(); }}, "×")))) : null,
    list);
}

// ---------- diagnostics ----------
function diagCard() {
  const out = h("div", {class: "verdict dim"}, "Checks which pin is the receiver (it idles high).");
  return card("Check the IR hardware",
    h("div", {class: "row"}, h("button", {class: "btn", onclick: async () => {
      const r = await get("/api/ir/diag");
      if (r.error) { out.replaceChildren(r.error); return; }
      out.replaceChildren(`TX G${r.tx} idle ${r.txIdle}, RX G${r.rx} idle ${r.rxIdle}: ` +
        ({ok: "the pins look right", swapped: "the pins look swapped (build with -D IR_PINS_SWAPPED)", unknown: "can't tell"}[r.pins] || r.pins));
    }}, "Check pins")), out);
}

// ---------- import ----------
function importFile() {
  const inp = h("input", {type: "file", accept: ".json,application/json"});
  inp.addEventListener("change", async () => {
    const f = inp.files[0];
    if (!f) return;
    let doc;
    try { doc = JSON.parse(await f.text()); } catch (e) { toast("That file isn't JSON", "err"); return; }
    const r = await post("/api/ir/import", {doc});
    if (report(r, `Imported ${r.added} buttons`)) {
      if (r.skipped && r.skipped.length) toast(`Skipped: ${r.skipped.slice(0, 3).join("; ")}`, "err");
      load();
    }
  });
  inp.click();
}
