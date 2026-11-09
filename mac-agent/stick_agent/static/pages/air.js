// The Stick page's "Mouse & presenter" card (stick firmware 2.5.0+: its state has `air`).
// Starts a remote mode on the stick (@mode) and edits its settings (@air), both saved on
// the stick. `send(action, args, okText)` is the Stick page's own (it refreshes the page).
import {h, card, field, toggle} from "../lib.js";

const MODES = [["mouse", "Air mouse"], ["presenter", "Presenter"], ["media", "Media remote"]];

function slider(id, min, max, value, unit, onchange) {
  const input = h("input", {type: "range", id, min, max, step: 1, value});
  const out = h("span", {class: "num small", style: "width:56px"}, value + unit);
  input.addEventListener("input", () => { out.textContent = input.value + unit; });
  input.addEventListener("change", () => onchange(+input.value));
  return h("div", {class: "row"}, input, out);
}

export function airCard(st, send) {
  const a = st.air;
  const running = a.mode && a.mode !== "home";
  const set = args => send("air", args, "Saved");
  const talk = h("select", {id: "airTalk", onchange: e => set({talk: e.target.value})},
    [[0, "No target"], [5, "5 min"], [10, "10 min"], [15, "15 min"], [20, "20 min"], [30, "30 min"],
      [45, "45 min"], [60, "1 hour"], [90, "1.5 hours"]]
      .concat([[a.talkMin, `${a.talkMin} min`]].filter(([v]) => ![0, 5, 10, 15, 20, 30, 45, 60, 90].includes(v)))
      .map(([v, l]) => h("option", {value: v, selected: a.talkMin === v}, l)));
  return card("Mouse & presenter",
    h("p", {class: "lead"},
      "The stick as an air mouse (turn it to point; A clicks, B right-clicks, hold B and tilt to scroll), ",
      "a slide clicker with a talk timer (A next, B back, hold A for a laser pointer, double-press B for a black screen), ",
      "or a media remote (twist for volume, flick for the next track). Hold A and B together on the stick to go back to Home."),
    field("Start", h("div", {class: "row"},
      MODES.map(([id, label]) => h("button", {class: "btn sm" + (a.mode === id ? " primary" : ""), type: "button",
        onclick: () => send("mode", {name: id}, `${label} on the stick`)}, label)),
      running ? h("button", {class: "btn sm ghost", type: "button", onclick: () => send("mode", {name: "home"}, "Back to Home")}, "Stop") : null),
      running ? `Running now: ${(MODES.find(m => m[0] === a.mode) || [, a.mode])[1]}.` : "Also in the stick's menu (triple-press B)."),
    st.connected && a.mouseKnown === false && a.mode === "mouse"
      ? h("div", {class: "notice warn"}, h("div", {class: "grow"},
          h("b", null, "The Mac hasn't picked up the mouse yet"),
          h("span", {class: "dim"}, " Once after updating to 2.5.0: in the Mac's Bluetooth settings, forget the stick, then connect it again (and accept the code on the stick)."))) : null,
    field("Pointer speed", slider("airSens", 1, 10, a.sens, "", v => set({sens: v}))),
    field("Invert", h("div", {class: "row", style: "gap:16px"},
      toggle("airInvX", "Left / right", a.invX, e => set({invx: e.target.checked ? 1 : 0})),
      toggle("airInvY", "Up / down", a.invY, e => set({invy: e.target.checked ? 1 : 0}))),
      "If the pointer goes the wrong way."),
    field("Scroll speed", slider("airScroll", 1, 10, a.scroll, "", v => set({scroll: v}))),
    field("Scroll direction", toggle("airInvS", "Reverse (tilt up scrolls down)", a.invScroll,
      e => set({invscroll: e.target.checked ? 1 : 0}))),
    field("Talk length", talk, "The presenter's timer turns orange near the end and red past it."),
    field("Slide keys", h("select", {id: "airKeys", onchange: e => set({keys: e.target.value})},
      h("option", {value: "arrows", selected: a.keys !== "pages"}, "Arrow keys (Keynote, PowerPoint, Google Slides)"),
      h("option", {value: "pages", selected: a.keys === "pages"}, "Page Down / Page Up (like a clicker)"))));
}
