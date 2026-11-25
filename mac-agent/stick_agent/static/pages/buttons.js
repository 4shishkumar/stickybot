// Global gestures belong to the physical stick. Capture the dashboard's device when entering
// Settings and discard late state / save replies on navigation, disconnection or device switch.
import {h, card, field, proto, store, deviceApi, deviceId} from "../lib.js";
import {TILES} from "./stick_layout.js";

export const title = "Buttons & gestures";
export const subtitle = "Choose what the buttons do on this stick.";
export const actions = () => [h("a", {class: "btn", href: "#settings"}, "Back to Settings")];

const GESTURES = [["b2", "Double-press B"], ["b3", "Triple-press B"], ["ab", "A + B"], ["bh", "Hold B"]];
const DISCRETE = [["unlock", "Unlock Mac"], ["launcher", "Launcher"], ["reply", "Reopen last reply"],
  ...TILES.map(([, id, name]) => ["tile:" + id, name]), ["app", "Open a Mac app…"],
  ["quick", "Run a quick command…"], ["talk", "Talk"], ["none", "Nothing"]];
const HOLD = [["talk", "Talk"], ["dictation", "Dictation"]];
const named = kind => kind === "app" || kind === "quick";
const choices = key => key === "bh" ? HOLD : DISCRETE;

function split(value) {
  for (const kind of ["app", "quick"]) {
    if (value.startsWith(kind + ":")) return [kind, value.slice(kind.length + 1)];
  }
  return [value, ""];
}

function nameError(name) {
  if (!name.trim()) return "Enter a name.";
  if (/[\p{Cc}\p{Cs}/\\]/u.test(name)) return "Use a name without slashes or control characters.";
  if (new TextEncoder().encode(name.trim()).length > 32) return "The name must fit in 32 UTF-8 bytes.";
  return "";
}

function readBindings(reply) {
  const bindings = reply?.buttons;
  for (const [key] of GESTURES) {
    if (typeof bindings?.[key] !== "string") return null;
    const [kind, name] = split(bindings[key]);
    if (!choices(key).some(([id]) => id === kind) || (named(kind) && nameError(name))) return null;
  }
  return Object.fromEntries(GESTURES.map(([key]) => [key, bindings[key]]));
}

function watchBindings(target, changed) {
  let active = true, sequence = 0, link = store.status.link, address = store.status.stick?.address;
  let online = store.online;
  // The Rust dashboard chooses an explicit device. Only the single-stick Python API uses
  // the unscoped route. Its /api/devices returns 404.
  const canRoute = target.id ? Promise.resolve(true) : target.get("/api/devices").then(r => r.missing === true);
  const current = () => active && deviceId() === target.id;
  async function read() {
    const version = ++sequence;
    if (!online || link !== "connected") { changed(null); return; }
    const routable = await canRoute;
    if (!current() || version !== sequence) return;
    if (!routable) { changed(null); return; }
    const r = await proto("state", null, target);
    if (!current() || version !== sequence) return;
    // A generic @state request can fall back to USB. Button writes are Bluetooth-only
    // so a fallback reply must never enable the editor.
    const source = r.device_address;
    changed(r.ok && r.via === "ble" && typeof source === "string" && source ? readBindings(r.reply) : null, source);
  }
  function reset() {
    ++sequence;
    changed(null);
    if (online) read();
  }
  const off = store.on((type, data) => {
    if (!current()) return;
    if (type === "event" && data.kind === "link" && data.state !== link) {
      link = data.state;
      reset();
    } else if (type === "status") {
      if (link !== data.link || online !== store.online || address !== data.stick?.address) {
        link = data.link; online = store.online; address = data.stick?.address;
        reset();
      }
    }
  });
  read();
  return {invalidate() { ++sequence; }, leave() { active = false; off(); }};
}

export function buttonSettingsLink() {
  const el = h("a", {class: "btn", href: "#settings/buttons", hidden: true}, title);
  const watch = watchBindings(deviceApi(), values => { el.hidden = !values; });
  return {el, leave: watch.leave};
}

let stop;
export function leave() { stop?.(); stop = null; }

export function render(root) {
  leave();
  const target = deviceApi();
  const content = h("div", {class: "empty"}, "Reading the stick’s buttons…");
  root.appendChild(content);
  let saved, savedAddress, revision = 0;
  const current = version => revision === version && deviceId() === target.id;
  function show(values, source) {
    if (savedAddress === source && JSON.stringify(saved) === JSON.stringify(values)) return;
    savedAddress = source;
    saved = values;
    const version = ++revision;
    if (!values) {
      content.className = "empty";
      content.textContent = "Connect a stick with configurable buttons over Bluetooth to edit its gestures.";
      return;
    }
    content.className = "";
    const fields = new Map();
    const save = h("button", {class: "btn primary", type: "button", onclick: submit}, "Save to stick");
    const message = h("span", {role: "status", class: "small dim", "aria-live": "polite"});
    const names = h("datalist", {id: "button-quick-names"});
    target.get("/api/commands").then(doc => {
      if (!current(version)) return;
      for (const cmd of doc.commands || []) {
        if (typeof cmd.name === "string" && !nameError(cmd.name)) names.appendChild(h("option", {value: cmd.name}));
      }
    });
    let busy = false;
    for (const [key, label] of GESTURES) {
      const selector = h("select", {id: "gesture-" + key}, choices(key).map(([value, text]) => h("option", {value}, text)));
      const name = h("input", {type: "text", autocomplete: "off", id: "gesture-name-" + key});
      const help = h("span", {}, "Choose a saved command that needs no arguments.");
      const extra = field("", name, help);
      fields.set(key, {selector, name, extra, help, label});
      selector.addEventListener("change", () => { name.value = ""; adjust(); });
      name.addEventListener("input", adjust);
    }
    function adjust() {
      let error = "", dirty = false;
      for (const [key, {selector, name, extra, help, label}] of fields) {
        const kind = selector.value;
        extra.hidden = !named(kind);
        help.hidden = kind !== "quick";
        extra.firstChild.textContent = kind === "app" ? "Mac app name" : "Quick command name";
        if (kind === "quick") name.setAttribute("list", names.id);
        else name.removeAttribute("list");
        const invalid = named(kind) && nameError(name.value);
        if (invalid) error ||= label + " — " + invalid;
        dirty ||= binding(kind, name.value) !== saved[key];
        selector.disabled = name.disabled = busy;
      }
      save.disabled = busy || !dirty || !!error;
      message.textContent = error;
    }
    function binding(kind, name) { return named(kind) ? kind + ":" + name.trim() : kind; }
    function display(bindings) {
      saved = bindings;
      for (const [key, {selector, name}] of fields) [selector.value, name.value] = split(saved[key]);
      adjust();
    }
    async function submit() {
      if (!current(version) || save.disabled) return;
      const requested = Object.fromEntries([...fields].map(([key, {selector, name}]) => [key, binding(selector.value, name.value)]));
      watch.invalidate();
      busy = true;
      adjust();
      message.textContent = "Saving…";
      const r = await proto("buttons", requested, target, source);
      if (!current(version)) return;
      watch.invalidate();
      busy = false;
      const confirmed = r.ok && r.device_address === source && readBindings(r.reply);
      if (!confirmed) {
        adjust();
        message.textContent = r.error || "The stick did not confirm the new bindings. Try again.";
        return;
      }
      display(confirmed);
      message.textContent = GESTURES.every(([key]) => confirmed[key] === requested[key]) ?
        "Saved to stick." : "The stick reported different bindings. Its current settings are shown.";
    }
    const rows = [...fields].flatMap(([key, {selector, extra, label}]) => [
      field(label, selector, key === "ab" ? "Press together, or hold B briefly and add A." : null), extra]);
    display(values);
    content.replaceChildren(card(store.status.stick?.name || "Stick buttons", ...rows, names,
      h("p", {class: "small dim"}, "Talk on a click starts recording. Press B again to send."),
      h("div", {class: "row"}, save, message)));
  }
  const watch = watchBindings(target, show);
  stop = () => { ++revision; watch.leave(); };
}
