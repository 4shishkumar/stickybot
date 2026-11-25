import {field, get, h, post, put} from "./lib.js";

const endpoint = "/api/mac-voice";
const modifiers = [["Control", "ctrlKey"], ["Option", "altKey"], ["Shift", "shiftKey"], ["Command", "metaKey"]];
const permissionText = {
  authorized: "Allowed", denied: "Allow StickyBot in System Settings → Privacy & Security → Microphone.",
  restricted: "Microphone access is restricted on this Mac.", undetermined: "StickyBot asks the first time you talk.",
};
const TEST_SECONDS = 6;

export function shortcutFromEvent(event) {
  if (!(event.ctrlKey || event.altKey || event.metaKey) || !/^(Space|Key[A-Z]|Digit[0-9])$/.test(event.code)) return null;
  return [...modifiers.filter(([, flag]) => event[flag]).map(([name]) => name), event.code].join("+");
}

function shortcutLabel(chord) {
  return chord.split("+").map(key => key.replace(/^(?:Key|Digit)/, "")).join(" + ");
}

// The microphone list: the system default first, then every input; a saved microphone that
// is not connected stays listed so the choice isn't lost.
export function microphoneOptions(state) {
  const inputs = state.inputs || {};
  const devices = Array.isArray(inputs.devices) ? inputs.devices : [];
  const selected = state.input || "";
  const options = [{value: "", label: `System default${inputs.default_name ? ` (${inputs.default_name})` : ""}`}];
  for (const d of devices) options.push({value: d.uid, label: d.name + (d.bluetooth ? " · Bluetooth" : "")});
  if (selected && !devices.some(d => d.uid === selected)) options.push({value: selected, label: "Saved microphone (not connected)"});
  return options.map(o => ({...o, selected: o.value === selected}));
}

export function microphoneNote(state) {
  const inputs = state.inputs || {};
  if (inputs.fallback) return `The saved microphone isn't connected, so StickyBot uses ${inputs.using}.`;
  if (inputs.using_bluetooth) return "Bluetooth headset microphones sound worse and can lower your headphones' audio while you talk. The built-in microphone is usually clearer.";
  return "Used from your next request. Test it to check the level.";
}

export function macVoiceFields() {
  const checkbox = h("input", {type: "checkbox", id: "macVoiceEnabled"});
  const record = h("button", {class: "btn", type: "button", id: "macVoiceShortcut"});
  const mic = h("select", {id: "macVoiceInput"});
  const test = h("button", {class: "btn", type: "button", id: "macVoiceTest"}, "Test");
  const bar = h("div", {class: "mic-level-bar", style: "width:0%"});
  const meter = h("div", {class: "mic-level", role: "meter", "aria-label": "Microphone level", "aria-valuemin": "0", "aria-valuemax": "100"}, bar);
  const micNote = h("div", {class: "help"});
  const sounds = h("input", {type: "checkbox", id: "macVoiceSounds"});
  const permission = h("span");
  const message = h("div", {class: "small", role: "status", "aria-live": "polite"});
  const el = h("div", {hidden: true},
    field("Mac keyboard", h("label", {class: "switch"}, checkbox, h("span", null, "Talk from Mac keyboard"))),
    field("Talk shortcut", record, "Hold it in any app and speak; release to send. Double-tap it to talk hands-free, then press it again to send. Esc cancels."),
    field("Microphone", h("div", null, h("div", {class: "row mic-row"}, mic, test, meter), micNote)),
    field("Sounds", h("label", {class: "switch"}, sounds, h("span", null, "Play a sound when listening starts and stops"))),
    field("Microphone access", permission), message);
  let current, editing = false, saving = false, closed = false, revision = 0, timer, testing = null;

  function draw() {
    el.hidden = false;
    checkbox.checked = current.enabled;
    sounds.checked = current.sounds !== false;
    checkbox.disabled = record.disabled = mic.disabled = sounds.disabled = saving;
    test.disabled = saving || current.permission === "denied" || current.permission === "restricted";
    record.textContent = editing ? "Press a shortcut…" : shortcutLabel(current.shortcut);
    record.setAttribute("aria-label", editing ? "Press a shortcut, or Escape to cancel" : "Change shortcut: " + record.textContent);
    while (mic.firstChild) mic.removeChild(mic.firstChild);
    for (const o of microphoneOptions(current)) mic.appendChild(h("option", {value: o.value, selected: o.selected}, o.label));
    mic.value = current.input || "";
    micNote.textContent = microphoneNote(current);
    permission.textContent = permissionText[current.permission] || "Unavailable";
    test.textContent = testing ? "Stop" : "Test";
    const level = testing && typeof current.test_level === "number" ? current.test_level : 0;
    bar.setAttribute("style", `width:${Math.round(Math.min(1, level * 1.4) * 100)}%`);
    meter.setAttribute("aria-valuenow", String(Math.round(level * 100)));
  }

  function accept(data) {
    current = data;
    draw();
    clearTimeout(timer);
    if (current.enabled && current.permission === "undetermined") timer = setTimeout(refresh, 2000);
  }

  async function refresh() {
    if (closed || saving) return;
    const version = ++revision;
    const data = await get(endpoint);
    if (closed || version !== revision || !data.supported) return;
    accept(data);
    if (!editing && !testing) message.textContent = data.detail || "";
  }

  async function save(change) {
    if (saving || closed) return;
    ++revision;
    saving = true;
    editing = false;
    stopListening();
    draw();
    message.textContent = "Saving…";
    const body = {enabled: current.enabled, shortcut: current.shortcut, input: current.input || "", sounds: current.sounds !== false, ...change};
    const data = await put(endpoint, body);
    if (closed) return;
    saving = false;
    if (data.error) {
      draw();
      message.textContent = data.error;
    } else {
      accept(data);
      message.textContent = data.detail || "Saved.";
    }
  }

  // The microphone test polls the level while the Mac listens (it stops by itself).
  async function startTest() {
    const data = await post(endpoint + "/test", {on: true});
    if (closed) return;
    if (data.error) { message.textContent = data.error; return; }
    testing = {until: Date.now() + TEST_SECONDS * 1000};
    message.textContent = "Say something. The bar should move while you speak.";
    accept(data);
    poll();
  }
  async function poll() {
    if (closed || !testing) return;
    if (Date.now() > testing.until) return stopTest(false);
    const data = await get(endpoint);
    if (closed || !testing) return;
    if (data.supported) {
      current = data;
      draw();
      if (data.test_level === null || data.test_level === undefined) return stopTest(false);
    }
    testing.timer = setTimeout(poll, 120);
  }
  async function stopTest(tell) {
    if (!testing) return;
    clearTimeout(testing.timer);
    testing = null;
    if (tell) await post(endpoint + "/test", {on: false});
    if (closed) return;
    message.textContent = "";
    draw();
  }

  // Keys go to the document while recording a shortcut: a clicked button doesn't always get focus.
  function onKey(event) {
    if (!editing || event.code === "Tab") return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") return stopEditing();
    if (event.repeat || ["Alt", "Control", "Meta", "Shift"].includes(event.key)) return;
    const shortcut = shortcutFromEvent(event);
    if (shortcut) save({shortcut});
    else message.textContent = "Use Control, Option or Command with a letter, number or Space.";
  }
  function stopListening() {
    document.removeEventListener?.("keydown", onKey, true);
  }
  function stopEditing() {
    if (!editing) return;
    editing = false;
    stopListening();
    message.textContent = "";
    draw();
  }

  checkbox.addEventListener("change", () => save({enabled: checkbox.checked}));
  sounds.addEventListener("change", () => save({sounds: sounds.checked}));
  mic.addEventListener("change", () => save({input: mic.value}));
  test.addEventListener("click", () => (testing ? stopTest(true) : startTest()));
  record.addEventListener("click", () => {
    if (editing) return stopEditing();
    editing = true;
    record.focus?.();
    document.addEventListener?.("keydown", onKey, true);
    message.textContent = "Press Control, Option or Command with a letter, number or Space. Escape cancels.";
    draw();
  });
  record.addEventListener("keydown", onKey);
  window.addEventListener("focus", refresh);
  refresh();
  return {el, leave() {
    closed = true;
    clearTimeout(timer);
    stopListening();
    if (testing) { clearTimeout(testing.timer); testing = null; post(endpoint + "/test", {on: false}); }
    window.removeEventListener("focus", refresh);
  }};
}
