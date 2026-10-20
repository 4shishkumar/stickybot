// Sounds: which sound the stick plays for each moment, its volume, and the
// sound library (Kenney's CC0 packs) to audition on the stick.
import {h, clear, card, field, toggle, get, put, post, report} from "../lib.js";

export const title = "Sounds";
export const subtitle = "What the stick plays and when. Saved on the stick, so they work without this agent.";

const MOMENTS = [
  ["listen", "Ready to listen", "hold B registered: speak now"], ["sent", "Request sent", "you release B"],
  ["answer", "Answer ready", "the agent replied"], ["approval", "Approval needed", "Claude Code asks; repeats every 15 s"],
  ["approved", "Approved", "A on a prompt"], ["denied", "Denied", "B on a prompt"],
  ["error", "Error", "didn't catch that / failed"], ["connected", "Mac connected", ""],
  ["disconnected", "Mac disconnected", ""], ["unlock", "Unlock", "double-press B typed the password"],
  ["charging", "Charging", "USB plugged in"], ["lowbattery", "Low battery", "at 15%"],
];
let root = null;

export function actions() {
  return [h("button", {class: "btn", type: "button", onclick: () => post("/api/sounds/play", {name: "test"})}, "Play a test sound")];
}

export async function render(el) {
  root = el;
  const top = h("div", null, h("div", {class: "empty"}, "Asking the stick…"));
  const lib = card("Sound library", h("p", {class: "lead"}, "Kenney's Interface Sounds and UI Audio (public domain). Press a number to hear it on the stick."), h("div", {class: "lib", id: "clipLib"}, h("div", {class: "empty"}, "Loading…")));
  root.append(top, lib);
  loadClips();
  const s = await get("/api/sounds");
  clear(top);
  if (s.error) { top.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"}, h("b", null, "The stick didn't answer"), h("span", {class: "dim"}, s.error)))); return; }
  const vol = h("input", {type: "range", id: "sndVol", min: 0, max: 255, value: s.volume});
  const out = h("span", {class: "num small", style: "width:44px"}, Math.round(s.volume / 2.55) + "%");
  vol.addEventListener("input", () => { out.textContent = Math.round(vol.value / 2.55) + "%"; });
  vol.addEventListener("change", async () => { report(await put("/api/sounds", {volume: +vol.value})); post("/api/sounds/play", {name: "test"}); });
  top.appendChild(card("Sound and volume",
    field("Sounds", toggle("sndOn", "Play cues on the stick's speaker", s.on, async e => report(await put("/api/sounds", {on: e.target.checked}), "Saved"))),
    field("Volume", h("div", {class: "row"}, vol, out))));
  const rows = h("div", {class: "list"});
  for (const [ev, label, when] of MOMENTS) {
    const sel = h("select", {id: "snd-" + ev, "aria-label": label}, ["off", ...(s.tunes || [])].map(t => h("option", {value: t, selected: s.map && s.map[ev] === t}, t)));
    sel.addEventListener("change", async () => {
      const r = await put("/api/sounds", {set: {[ev]: sel.value}});
      if (report(r, "Saved") && sel.value !== "off") post("/api/sounds/play", {name: sel.value});
    });
    rows.appendChild(h("div", {class: "item"},
      h("div", {class: "what"}, h("b", null, label), when ? h("span", {class: "sub"}, when) : null),
      sel, h("button", {class: "btn icon", "aria-label": `Play ${label}`, onclick: () => sel.value !== "off" && post("/api/sounds/play", {name: sel.value})}, "▶")));
  }
  top.appendChild(card("Moments", h("p", {class: "lead"}, "Clips come first in each list, then the synthesized beeps. ▶ plays it on the stick."), rows));
}

export function leave() { root = null; }

async function loadClips() {
  const box = document.getElementById("clipLib");
  const lib = await get("/api/clips");
  if (!box) return;
  clear(box);
  if (!Array.isArray(lib) || !lib.length) { box.appendChild(h("div", {class: "empty"}, lib.error || "No sound packs in ~/.stick-agent/sounds.")); return; }
  const groups = {};
  for (const c of lib) { const g = c.name.replace(/[_-]?\d+$/, "") || c.name; (groups[g] = groups[g] || []).push(c); }
  for (const [g, list] of Object.entries(groups)) {
    box.appendChild(h("div", {class: "grp"}, h("span", {class: "eyebrow"}, g.replace(/[_-]/g, " ")),
      h("div", {class: "btns"}, list.map(c => h("button", {class: "btn sm", title: `${c.pack}/${c.name}`, onclick: async e => {
        e.currentTarget.disabled = true;
        const b = e.currentTarget;
        report(await post("/api/clips/play", {pack: c.pack, name: c.name}));
        b.disabled = false;
      }}, "▶ " + ((c.name.match(/\d+$/) || [c.name])[0]))))));
  }
}
