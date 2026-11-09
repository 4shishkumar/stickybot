// Stick: the stick's own settings (sent over Bluetooth, or USB when plugged in)
// and its firmware.
import {h, clear, card, field, toggle, proto, get, put, post, report, toast, store, fmtAgo, recall, remember} from "../lib.js";
import {firmwarePanel} from "../firmware.js";
import {airCard} from "./air.js";

export const title = "Stick";
export const subtitle = "The stick's own settings. They're saved on the stick, so they stay with it.";

// Fallback for older firmware; newer sticks send their own list (with Fn) in `keys`.
const MODS = [["Ctrl", 1], ["Option", 4], ["Shift", 2], ["Cmd", 8]];
let root = null, st = null, keys = null, via = null, snd = null, petInfo = null;

export function actions() {
  return [h("button", {class: "btn", type: "button", onclick: load}, "Refresh"),
    h("button", {class: "btn", type: "button", title: "Tap the combo once on the Mac", onclick: async () => report(await proto("test"), "Combo sent to the Mac")}, "Test combo")];
}

export function render(el) { root = el; load(); }
export function leave() { root = null; }

async function load() {
  if (!root) return;
  clear(root).appendChild(h("div", {class: "empty"}, "Asking the stick…"));
  const [r, k, s, pt] = await Promise.all([proto("state"), keys ? Promise.resolve(null) : proto("keys"), get("/api/sounds"),
    get("/api/pet").catch(() => null)]);
  petInfo = pt && pt.pet ? pt.pet : null;
  if (k && k.ok && k.reply) keys = k.reply;
  st = r.ok && r.reply ? r.reply : null;
  via = r.via;
  snd = s && !s.error ? s : null;
  paint(r);
}

async function send(action, args, okText, restarts) {
  const r = await proto(action, args);
  if (!report(r, okText)) return false;
  if (r.reply && r.reply.restarting || restarts) {
    toast(r.reply && r.reply.repair
      ? "The stick is restarting with a new keyboard identity (for Fn): forget it in the Mac's Bluetooth settings and pair again."
      : "The stick is restarting…");
    setTimeout(load, 6000);
  } else load();
  return true;
}

function paint(res) {
  if (!root) return;
  clear(root);
  root.appendChild(hero());
  if (!st) {
    root.appendChild(h("div", {class: "notice " + (res && res.missing ? "" : "warn")}, h("div", {class: "grow"},
      h("b", null, res && res.missing ? "Stick settings aren't available yet" : "Can't reach the stick"),
      h("span", {class: "dim"}, res && res.missing
        ? "This version of the agent can't change the stick's settings yet. The stick's own menu (triple-press B) still works."
        : (res && res.error) || "Check it's switched on and connected (Overview shows the link)."))));
  } else {
    root.append(identityCard(), ...(st.faces ? [faceCard(), marketCard()] : []), comboCard(), unlockCard(), slotsCard(),
      screenCard(), motionCard(), ...(st.air ? [airCard(st, send)] : []), soundCard());  // air: firmware 2.5.0+
  }
  root.appendChild(card("Firmware", firmwarePanel()));
}

function hero() {
  const s = store.status || {};
  const name = (st && st.name) || (s.stick && s.stick.name) || "Your stick";
  const facts = [];
  if (st) {
    if (st.battery >= 0) facts.push(`${st.battery}%${st.charging ? " · charging" : ""}`);
    if (st.fw) facts.push(`firmware ${st.fw}`);
    facts.push(st.bt === "connected" ? "connected to the Mac" : st.bt || "");
  }
  return h("section", {class: "card stick-hero"},
    h("div", {class: "stick-device", "aria-hidden": "true"}, h("div", {class: "scr"}, h("div", {class: "eyes"}, h("i"), h("i"))), h("div", {class: "btnA"})),
    h("div", {style: "display:flex;flex-direction:column;gap:6px"},
      h("h2", {style: "font-size:20px"}, name),
      h("div", {class: "dim"}, facts.filter(Boolean).join(" · ") || "—"),
      via ? h("div", {class: "small faint"}, via === "usb" ? "Talking to it over USB" : "Talking to it over Bluetooth") : null));
}

function identityCard() {
  const name = h("input", {id: "stickName", value: st.name || "", maxlength: 20});
  return card("Name",
    h("p", {class: "lead"}, "Shown in the Mac's Bluetooth settings. Renaming restarts the stick; a Mac that's already paired may keep the old name until you re-pair."),
    field("Bluetooth name", h("div", {class: "row"}, name,
      h("button", {class: "btn", onclick: () => {
        const v = name.value.trim();
        if (!v || v === st.name) return;
        send("name", {name: v}, "Renamed", true);
      }}, "Rename")), null));
}

// Mac password for double-press-B unlock. Write-only: the stick never sends it
// back (state only says macPassSet), nothing logs it, and the field is cleared
// as soon as it's sent.
// The Home screen's face (firmware 2.1.0+ sends `faces` in its state; older firmware: no card).
// Preview plays a short demo of a face on the stick (A or B on the stick ends it).
function faceCard() {
  const p = petInfo;
  const plural = (n, w) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const worn = p ? [p.acc & 1 ? "hat" : null, p.acc & 2 ? "sunglasses" : null, p.acc & 4 ? "crown" : null].filter(Boolean) : [];
  const facts = p ? h("div", {class: "pet-facts small dim"},
    h("span", null, `StickyBot level ${p.level} (${p.into}/${p.need} XP)`),
    h("span", null, `streak: ${plural(p.streak, "day")}`),
    h("span", null, `city: ${plural(p.merged, "building")}${p.releases ? ", " + plural(p.releases, "tower") : ""}`),
    h("span", null, worn.length ? "wears: " + worn.join(", ") : "no accessories yet")) : null;
  const tiles = st.faces.map(f => h("div", {class: "face" + (f.id === st.face ? " on" : "")},
    h("div", {class: "shots"},
      h("img", {class: "tall", src: `/static/faces/${f.id}.png`, alt: "", onerror: e => e.target.remove()}),
      h("img", {class: "wide", src: `/static/faces/${f.id}-wide.png`, alt: "", onerror: e => e.target.remove()})),
    h("b", null, f.title),
    h("span", {class: "small dim"}, f.about),
    h("div", {class: "row", style: "margin-top:4px"},
      f.id === st.face ? h("span", {class: "ok-text small"}, "✓ on the stick")
        : h("button", {class: "btn sm primary", type: "button", onclick: () => send("face", {id: f.id}, `Face: ${f.title}`)}, "Use"),
      h("button", {class: "btn sm", type: "button", title: "Plays a short demo on the stick (A or B ends it)",
        onclick: async () => report(await proto("face", {demo: f.id}), `Preview of ${f.title} on the stick`)}, "Preview"))));
  return card("Face",
    h("p", {class: "lead"}, "How the stick's Home screen looks. Approvals, questions and alerts still take over the screen, ",
      "and every face keeps the time and anything urgent in view."),
    facts, h("div", {class: "faces"}, tiles));
}

// Face packages (firmware 2.4.0+, docs/faces/CATALOG.md): the faces installed on the stick
// (Use / Update / Remove, or "didn't load"), "Get more faces" from the catalog with filters,
// and "Install from file…" for people making faces. The agent fetches and caches the catalog
// (/api/faces/catalog), checks every file, and sends it over Bluetooth (/api/faces/install, a
// job with progress like firmware updates).
const MIN_FACES_FW = "2.4.0";
const KINDS = {  // the filters, from a face's tags and motion (GUIDELINES.md, Naming)
  minimal: f => tagged(f, "minimal") || (f.motion || "static") === "static" && !tagged(f, "animated", "scene", "cute", "pixel-art"),
  fun: f => tagged(f, "animated", "scene", "cute", "pixel-art", "retro") || (f.motion || "static") !== "static",
  utility: f => tagged(f, "data", "clock", "analog", "digital"),
};
const MOTION = {static: "Static", calm: "Calm", lively: "Lively · uses a little more battery"};
let market = null, marketBox = null, marketOpen = false, marketJob = null, marketMsg = null;
let marketFilter = recall("faceFilter", {kind: "all", channel: "all"});
const tagged = (f, ...tags) => (f.tags || []).some(t => tags.includes(t));
const cap = s => s ? s[0].toUpperCase() + s.slice(1) : s;
const kb = n => n >= 1024 ? `${Math.round(n / 1024)} KB` : `${n} bytes`;

function marketCard() {
  marketBox = h("div", {class: "market"}, h("div", {class: "empty"}, "Looking at the faces on the stick…"));
  loadMarket(false);
  return card(h("div", {class: "row grow"}, h("h2", null, "More faces"),
    h("span", {class: "small faint", style: "margin-left:auto"}, "made by people, checked before they're added")), marketBox);
}

async function loadMarket(refresh) {
  const r = await get("/api/faces/catalog" + (refresh ? "?refresh=1" : ""));
  market = r;
  paintMarket();
}

function paintMarket() {
  if (!marketBox || !marketBox.isConnected) return;
  const m = market;
  clear(marketBox);
  if (!m) return;
  if (m.missing) { marketBox.appendChild(h("div", {class: "empty"}, "This version of the agent can't add faces yet.")); return; }
  const s = m.stick || {};
  if (marketMsg) marketBox.appendChild(marketMsg);
  if (marketJob) marketBox.appendChild(marketJob.el);
  if (s.connected && !s.supported && s.fw) {
    marketBox.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"},
      h("b", null, `Update the stick to ${MIN_FACES_FW} to add faces`),
      h("span", {class: "dim"}, `It runs ${s.fw}. The Firmware section below updates it over Bluetooth or USB; you can look through the faces meanwhile.`))));
  } else if (!s.connected) {
    marketBox.appendChild(h("div", {class: "notice"}, h("div", {class: "grow"},
      h("b", null, "The stick isn't connected"), h("span", {class: "dim"}, "Faces go on over Bluetooth. You can still look through them."))));
  } else if (s.error) {
    marketBox.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"}, h("b", null, "Couldn't list the stick's faces"), h("span", {class: "dim"}, cap(s.error)))));
  }
  if (m.installed) marketBox.appendChild(installedList(m.installed, s));
  const head = h("div", {class: "row"},
    h("button", {class: "btn" + (marketOpen ? "" : " primary"), type: "button", "aria-expanded": String(marketOpen), onclick: () => {
      marketOpen = !marketOpen;
      if (marketOpen) loadMarket(true); else paintMarket();
    }}, marketOpen ? "Hide the catalog" : "Get more faces"),
    fileButton(s),
    m.fetched ? h("span", {class: "small faint"}, `catalog ${m.offline ? "saved" : "checked"} ${fmtAgo(m.fetched)}${m.signed ? "" : " · unsigned (custom catalog)"}`) : null);
  marketBox.appendChild(head);
  if (!marketOpen) return;
  if (m.error) {
    marketBox.appendChild(h("div", {class: "notice " + (m.faces && m.faces.length ? "" : "warn")}, h("div", {class: "grow"},
      h("b", null, m.offline ? "Showing the last copy of the catalog" : "Can't load the faces catalog"), h("span", {class: "dim"}, cap(m.error)))));
  }
  const all = m.faces || [];
  if (!all.length) { if (!m.error) marketBox.appendChild(h("div", {class: "empty"}, "The catalog has no faces yet.")); return; }
  marketBox.appendChild(filters());
  const shown = all.filter(f => (marketFilter.kind === "all" || KINDS[marketFilter.kind](f)) &&
    (marketFilter.channel === "all" || (marketFilter.channel === "official") === (f.channel === "examples")));
  marketBox.appendChild(shown.length ? h("div", {class: "faces market-grid"}, shown.map(f => catalogTile(f, s)))
    : h("div", {class: "empty"}, "No faces match these filters."));
}

function filters() {
  const seg = (key, opts) => h("div", {class: "seg", role: "group"}, opts.map(([v, label]) =>
    h("button", {type: "button", class: marketFilter[key] === v ? "on" : null, "aria-pressed": String(marketFilter[key] === v),
      onclick: () => { marketFilter = {...marketFilter, [key]: v}; remember("faceFilter", marketFilter); paintMarket(); }}, label)));
  return h("div", {class: "row market-filters"},
    seg("kind", [["all", "All"], ["minimal", "Minimal"], ["fun", "Fun"], ["utility", "Utility"]]),
    seg("channel", [["all", "Everyone's"], ["official", "Official"], ["community", "Community"]]));
}

function shots(id, previews) {
  const img = (cls, side) => h("img", {class: cls, src: previews ? previews[side] : `/api/faces/preview/${encodeURIComponent(id)}/${side}`,
    alt: "", loading: "lazy", onerror: e => e.target.remove()});
  return h("div", {class: "shots"}, img("tall", "portrait"), img("wide", "landscape"));
}

function catalogTile(f, s) {
  const busy = !!(marketJob && !marketJob.done) || s.busy;
  const author = f.author && (f.author.github ? "@" + f.author.github : f.author.name);
  let action;
  if (f.installed && !f.update) action = h("span", {class: "ok-text small"}, st && st.face === f.id ? "✓ on the stick, in use" : "✓ on the stick");
  else if (f.fits) action = h("span", {class: "small dim"}, cap(f.fits));
  else if (!s.connected || !s.supported) action = h("span", {class: "small faint"}, s.connected ? `Needs firmware ${MIN_FACES_FW}` : "Connect the stick to install");
  else action = h("button", {class: "btn sm primary", type: "button", disabled: busy, onclick: () => install({id: f.id}, f.name)},
    f.update ? `Update to ${f.version}` : "Install");
  return h("div", {class: "face" + (f.installed ? " on" : "")},
    shots(f.id, f.previews),
    h("b", null, f.name),
    h("span", {class: "small dim"}, author ? `by ${author}` : ""),
    h("span", {class: "small dim"}, f.about),
    h("div", {class: "chips small"},
      f.channel === "examples" ? h("span", {class: "tag agent"}, "Official") : h("span", {class: "tag you"}, "Community"),
      h("span", {class: "tag"}, MOTION[f.motion || "static"] || f.motion),
      h("span", {class: "faint"}, kb(f.size))),
    (f.tags || []).length ? h("span", {class: "small faint"}, f.tags.join(" · ")) : null,
    h("div", {class: "row", style: "margin-top:auto;padding-top:4px"}, action));
}

function installedList(list, s) {
  if (!list.length) return h("div", {class: "small dim", style: "margin-bottom:12px"},
    `No added faces on the stick yet${s.max ? ` (room for ${s.max})` : ""}.`);
  const busy = !!(marketJob && !marketJob.done) || s.busy;
  return h("div", {style: "margin-bottom:14px"},
    h("div", {class: "eyebrow", style: "margin-bottom:4px"}, `On the stick · ${list.length}${s.max ? ` of ${s.max}` : ""}`),
    h("div", {class: "list"}, list.map(f => {
      const using = st && st.face === f.id;
      const note = f.err ? h("span", {class: "tag err"}, "Didn't load on your stick")
        : f.update ? h("span", {class: "tag ask"}, `Update: ${f.latest}`)
        : !f.inCatalog ? h("span", {class: "tag"}, f.retired ? "Removed from the catalog" : "Not in the catalog")
        : using ? h("span", {class: "tag ok"}, "In use") : null;
      const rm = h("button", {class: "btn sm danger", type: "button", disabled: busy,
        onclick: e => confirmTwice(e.currentTarget, () => removeFace(f))}, "Remove");
      return h("div", {class: "item"},
        h("img", {class: "mini", src: `/api/faces/preview/${encodeURIComponent(f.id)}/portrait`, alt: "",
          onerror: e => { e.target.style.visibility = "hidden"; }}),
        h("div", {class: "what"},
          h("b", null, f.name, " ", note),
          h("span", {class: "sub"}, [f.version, f.size ? kb(f.size) : null, f.author && f.author.github ? "@" + f.author.github : null,
            f.err ? `the stick says: ${f.err}` : null].filter(Boolean).join(" · "))),
        h("div", {class: "row"},
          !using && !f.err ? h("button", {class: "btn sm primary", type: "button", disabled: busy, onclick: () => useFace(f)}, "Use") : null,
          f.update || (f.err && f.inCatalog) ? h("button", {class: "btn sm", type: "button", disabled: busy || !s.supported,
            onclick: () => install({id: f.id}, f.name)}, f.err ? "Reinstall" : "Update") : null,
          rm));
    })));
}

function fileButton(s) {
  const input = h("input", {type: "file", accept: ".sfb", style: "display:none", onchange: async () => {
    const file = input.files && input.files[0];
    input.value = "";
    if (!file) return;
    if (file.size > 64 * 1024) { showMarketMsg("err", "That file is too big", "A face is 64 KB at most."); return; }
    const bytes = new Uint8Array(await file.arrayBuffer());
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    install({file: btoa(bin), name: file.name}, file.name.replace(/\.sfb$/, ""));
  }});
  return h("span", null, input, h("button", {class: "btn ghost", type: "button", disabled: !s.supported || !!(marketJob && !marketJob.done),
    title: "A compiled face (.sfb) from tools/faces/validate.py --compile, for trying your own face before you share it",
    onclick: () => input.click()}, "Install from file…"));
}

function showMarketMsg(kind, title, text, ...extra) {
  marketMsg = h("div", {class: "notice " + kind}, h("div", {class: "grow"}, h("b", null, title), text ? h("span", {class: "dim"}, text) : null,
    extra.length ? h("div", {class: "row", style: "margin-top:8px"}, ...extra) : null),
    h("button", {class: "btn sm ghost", type: "button", "aria-label": "Dismiss", onclick: () => { marketMsg = null; paintMarket(); }}, "×"));
  paintMarket();
}

const FACE_STAGES = {download: "Downloading and checking", send: "Sending over Bluetooth", verify: "The stick is checking it"};

async function install(body, name) {
  marketMsg = null;
  const r = await post("/api/faces/install", body);
  if (r.error) { showMarketMsg("err", `Couldn't install ${name}`, r.error); return; }
  const bar = h("div", {class: "progress"}, h("i"));
  const stage = h("div", {class: "small dim"}, "Starting…");
  const stop = h("button", {class: "btn sm ghost", type: "button", onclick: async () => {
    stop.disabled = true;
    const c = await post(`/api/faces/jobs/${encodeURIComponent(r.job)}/cancel`, {});
    if (c.error) { toast(c.error, "err"); stop.disabled = false; }
  }}, "Cancel");
  marketJob = {id: r.job, done: false, el: h("div", {class: "notice market-job"}, h("div", {class: "grow"},
    h("b", null, `Installing ${name}`), h("span", {class: "dim"}, "Keep the stick near this Mac; it takes a few seconds."), stage, bar), stop)};
  paintMarket();
  const es = new EventSource(`/api/faces/jobs/${encodeURIComponent(r.job)}`);
  es.onmessage = e => {
    let m;
    try { m = JSON.parse(e.data); } catch (err) { return; }
    if (m.progress !== undefined) bar.firstChild.style.width = Math.max(0, Math.min(100, m.progress)) + "%";
    if (m.stage) stage.textContent = (FACE_STAGES[m.stage] || m.stage) + (m.progress !== undefined && m.progress < 100 ? ` · ${m.progress}%` : "");
    if (!m.done) return;
    es.close();
    marketJob = null;
    const res = m.result || {};
    if (m.ok) {
      const useBtn = h("button", {class: "btn sm primary", type: "button", onclick: () => { marketMsg = null; useFace({id: res.id, name: res.name}); }}, "Use it now");
      showMarketMsg("ok", `${res.name || name} is on your stick`, null, useBtn,
        h("button", {class: "btn sm ghost", type: "button", onclick: () => { marketMsg = null; paintMarket(); }}, "Not now"));
    } else if (/^cancelled/i.test(m.error || "")) {
      showMarketMsg("", "Install cancelled", "Nothing changed on the stick.");
    } else {
      showMarketMsg("err", `Couldn't install ${name}`, m.error || "Something went wrong.");
    }
    loadMarket(false);
  };
  es.onerror = () => { if (es.readyState !== EventSource.CLOSED) stage.textContent = "Lost the connection to the agent. The install may still be running."; };
}

async function useFace(f) {
  const r = await post("/api/faces/use", {id: f.id});
  if (!report(r, `Face: ${f.name || f.id}`)) return;
  if (st) st.face = f.id;
  load();
}

async function removeFace(f) {
  const r = await post("/api/faces/remove", {id: f.id});
  if (report(r, `Removed ${f.name || f.id} from the stick`)) loadMarket(false);
}

function unlockCard() {
  const set = !!st.macPassSet;
  const pw = h("input", {type: "password", id: "macPass", autocomplete: "new-password", spellcheck: "false",
                         placeholder: set ? "•••••••• (saved — type to replace)" : "your Mac login password"});
  const show = h("button", {class: "btn sm", type: "button", title: "Show / hide",
    onclick: () => { pw.type = pw.type === "password" ? "text" : "password"; show.textContent = pw.type === "password" ? "Show" : "Hide"; }}, "Show");
  const save = async () => {
    const v = pw.value;
    if (!v) return;
    pw.value = "";
    pw.type = "password";
    await send("macpass", {pass: v}, "Mac password saved");
  };
  pw.addEventListener("keydown", e => { if (e.key === "Enter") save(); });
  const clearBtn = h("button", {class: "btn sm danger", type: "button",
    onclick: () => confirmTwice(clearBtn, () => send("macpass", {pass: ""}, "Mac password removed"))}, "Remove");
  return card("Unlock the Mac",
    h("p", {class: "lead"}, "Double-press B on the lock screen and the stick types this password and presses Return. ",
      "It's stored only on the stick, can't be read back, and is sent over the encrypted Bluetooth link."),
    field("Mac password", h("div", {class: "row"}, pw, show, h("button", {class: "btn primary", type: "button", onclick: save}, "Save")),
      set ? h("span", {class: "ok-text"}, "✓ saved on the stick") : h("span", {class: "dim"}, "not set — double-press B does nothing yet")),
    set ? h("div", {class: "row"}, clearBtn) : null);
}

function comboCard() {
  const mods = st.mods || 0;
  const boxes = (keys && keys.mods ? keys.mods : MODS).map(([label, bit]) => h("label", {class: "switch"},
    h("input", {type: "checkbox", id: "mod" + label, checked: !!(mods & bit), "data-bit": bit}), h("span", null, label)));
  const keySel = h("select", {id: "comboKey"},
    h("option", {value: 0, selected: !st.key}, "(none — modifiers only)"),
    (keys && keys.keys ? keys.keys : []).map(([n, code]) => h("option", {value: code, selected: code === st.key}, n)));
  const parts = (st.combo || "").split("+").filter(Boolean);
  return card("Button A",
    h("p", {class: "lead"}, "The key combo button A holds down on the Mac (for Wispr Flow: Option + Cmd, or Fn). Adding or removing Fn restarts the stick, and the Mac needs to pair with it again."),
    h("div", {class: "combo-keys", style: "margin-bottom:14px"}, parts.map(p => h("span", {class: "kbd"}, p))),
    field("Modifiers", h("div", {class: "row", style: "gap:16px"}, boxes)),
    field("Key", keySel, keys ? null : "Key list unavailable — modifiers only."),
    field("Enter after a hold", h("select", {id: "enterDelay", onchange: e => send("enter", {delay: e.target.value}, "Saved")},
      [[0, "Off"], [500, "after 0.5 s"], [1000, "after 1 s"], [1500, "after 1.5 s"], [2000, "after 2 s"], [3000, "after 3 s"]]
        .map(([v, l]) => h("option", {value: v, selected: (st.enterDelay || 0) === v}, l))),
      "For dictation: sends Enter once the app has typed the text. Only holds over 1 s."),
    h("div", {class: "row", style: "margin-top:14px"}, h("button", {class: "btn primary", onclick: () => {
      let m = 0;
      for (const b of boxes) { const i = b.querySelector("input"); if (i.checked) m |= +i.dataset.bit; }
      if (!m && !+keySel.value) { toast("Pick at least one modifier or a key.", "err"); return; }
      send("combo", {mods: m, key: keySel.value}, "Combo saved");
    }}, "Save combo")));
}

function slotsCard() {
  const slots = st.slots || [];
  return card("Macs",
    h("p", {class: "lead"}, "The stick can be paired with up to three Macs. Switching restarts it; pair it on the new Mac the first time."),
    h("div", {class: "slots"}, slots.map(sl => h("div", {class: "slot" + (sl.n === st.slot ? " on" : "")},
      h("b", null, `Mac ${sl.n}`),
      h("span", {class: "small dim"}, sl.n === st.slot ? (sl.paired ? "this one · paired" : "this one · not paired yet") : sl.paired ? "paired" : "empty"),
      h("div", {class: "row", style: "margin-top:6px"},
        sl.n !== st.slot ? h("button", {class: "btn sm", onclick: () => send("slot", {n: sl.n}, `Switching to Mac ${sl.n}`, true)}, "Switch") : null,
        sl.paired ? h("button", {class: "btn sm danger", onclick: e => confirmTwice(e.currentTarget, () => send("forget", {n: sl.n}, `Forgot Mac ${sl.n}`))}, "Forget") : null)))));
}

// Two-step confirm inside the page (dialogs don't work everywhere).
function confirmTwice(btn, fn) {
  if (btn.dataset.armed) { fn(); return; }
  btn.dataset.armed = "1";
  const old = btn.textContent;
  btn.textContent = "Click again to confirm";
  setTimeout(() => { if (btn.isConnected) { delete btn.dataset.armed; btn.textContent = old; } }, 3000);
}

function screenCard() {
  const bright = h("input", {type: "range", id: "brightness", min: 10, max: 100, step: 5, value: st.brightness || 50});
  const out = h("span", {class: "num small", style: "width:44px"}, (st.brightness || 50) + "%");
  bright.addEventListener("input", () => { out.textContent = bright.value + "%"; });
  bright.addEventListener("change", () => send("brightness", {pct: bright.value}, "Brightness saved"));
  return card("Screen & power",
    field("Brightness", h("div", {class: "row"}, bright, out)),
    field("Standby on battery", h("select", {id: "standby", onchange: e => send("standby", {min: e.target.value}, "Saved")},
      [[0, "Never"], [5, "after 5 min idle"], [10, "after 10 min idle"], [30, "after 30 min idle"], [60, "after 1 hour idle"]]
        .map(([v, l]) => h("option", {value: v, selected: (st.standby || 0) === v}, l))), "On USB power it never sleeps."),
    field("Auto-rotate", toggle("autoRotate", "Turn the screen to how you hold the stick", st.autoRotate,
      e => send("autorotate", {on: e.target.checked ? 1 : 0}, "Saved"))),
    st.landscape === undefined ? null :  // firmware 1.9.0+
      field("Landscape screens", toggle("landscape", "Held sideways, approvals, questions, alerts and replies turn landscape (needs auto-rotate)",
        st.landscape, e => send("landscape", {on: e.target.checked ? 1 : 0}, "Saved"))),
    st.clock24 === undefined ? null :  // firmware 2.4.0+
      field("24-hour clock", toggle("clock24", "Show the time as 15:05 instead of 3:05 PM (on every face that shows the time)",
        st.clock24, e => send("clock24", {on: e.target.checked ? 1 : 0}, "Saved"))),
    field("Developer mode", toggle("devMode", "Keep Wi-Fi on while on USB (for wireless updates and logs)", st.devMode,
      e => send("devmode", {on: e.target.checked ? 1 : 0}, "Saved"))));
}

function motionCard() {
  const m = st.motion || {};
  const sendMotion = () => send("motion", {
    lift: document.getElementById("mLift").checked ? 1 : 0,
    face: document.getElementById("mFace").checked ? 1 : 0,
    shake: document.getElementById("mShake").checked ? 1 : 0}, "Saved");
  return card("Gestures",
    field("Lift to wake", toggle("mLift", "Picking the stick up turns the screen on", m.lift, sendMotion)),
    field("Face down", toggle("mFace", "Lying face down for 3 s puts it in standby", m.face, sendMotion)),
    field("Shake", toggle("mShake", "Shaking sends Escape, or cancels / denies on the agent screens", m.shake, sendMotion)));
}

function soundCard() {
  if (!snd) return card("Sounds", h("div", {class: "empty"}, "Sound settings come from the stick; it didn't answer."));
  const vol = h("input", {type: "range", id: "stickVol", min: 0, max: 255, value: snd.volume});
  const out = h("span", {class: "num small", style: "width:44px"}, Math.round(snd.volume / 2.55) + "%");
  vol.addEventListener("input", () => { out.textContent = Math.round(vol.value / 2.55) + "%"; });
  vol.addEventListener("change", async () => { report(await put("/api/sounds", {volume: +vol.value})); post("/api/sounds/play", {name: "test"}); });
  return card(h("div", {class: "row grow"}, h("h2", null, "Sounds"), h("a", {class: "btn sm", href: "#sounds", style: "margin-left:auto"}, "Choose sounds")),
    field("Sounds", toggle("stickSnd", "Play cues on the stick's speaker", snd.on, async e => report(await put("/api/sounds", {on: e.target.checked}), "Saved"))),
    field("Volume", h("div", {class: "row"}, vol, out)));
}
