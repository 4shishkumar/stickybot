// Stick: the stick's own settings (sent over Bluetooth, or USB when plugged in)
// and its firmware.
import {h, clear, card, field, toggle, proto, get, put, post, report, toast, store} from "../lib.js";
import {firmwarePanel} from "../firmware.js";

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
    root.append(identityCard(), ...(st.faces ? [faceCard()] : []), comboCard(), unlockCard(), slotsCard(), screenCard(),
      motionCard(), soundCard());
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
