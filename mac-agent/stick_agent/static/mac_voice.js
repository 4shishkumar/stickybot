import {field, get, h, put} from "./lib.js";

const endpoint = "/api/mac-voice";
const modifiers = [["Control", "ctrlKey"], ["Option", "altKey"], ["Shift", "shiftKey"], ["Command", "metaKey"]];
const permissionText = {
  authorized: "Allowed", denied: "Allow StickyBot in System Settings → Privacy & Security → Microphone.",
  restricted: "Microphone access is restricted on this Mac.", undetermined: "Permission has not been requested yet.",
};

export function shortcutFromEvent(event) {
  if (!(event.ctrlKey || event.altKey || event.metaKey) || !/^(Space|Key[A-Z]|Digit[0-9])$/.test(event.code)) return null;
  return [...modifiers.filter(([, flag]) => event[flag]).map(([name]) => name), event.code].join("+");
}

function shortcutLabel(chord) {
  return chord.split("+").map(key => key.replace(/^(?:Key|Digit)/, "")).join(" + ");
}

export function macVoiceFields() {
  const checkbox = h("input", {type: "checkbox", id: "macVoiceEnabled"});
  const record = h("button", {class: "btn", type: "button", id: "macVoiceShortcut"});
  const inputName = h("span"), permission = h("span");
  const message = h("div", {class: "small", role: "status", "aria-live": "polite"});
  const el = h("div", {hidden: true},
    field("Mac keyboard", h("label", {class: "switch"}, checkbox, h("span", null, "Talk from Mac keyboard"))),
    field("Hold-to-talk shortcut", record, "Click to change. Hold the shortcut in any app, then release to ask."),
    field("Mac microphone", inputName), field("Microphone access", permission), message);
  let current, editing = false, saving = false, closed = false, revision = 0, timer;

  function draw() {
    el.hidden = false;
    checkbox.checked = current.enabled;
    checkbox.disabled = record.disabled = saving;
    record.textContent = editing ? "Press a shortcut…" : shortcutLabel(current.shortcut);
    record.setAttribute("aria-label", editing ? "Press a shortcut, or Escape to cancel" : "Change shortcut: " + record.textContent);
    inputName.textContent = current.input_name || "System default";
    permission.textContent = permissionText[current.permission] || "Unavailable";
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
    if (!editing) message.textContent = data.detail || "";
  }

  async function save(enabled, shortcut) {
    if (saving || closed) return;
    ++revision;
    saving = true;
    editing = false;
    draw();
    message.textContent = "Saving…";
    const data = await put(endpoint, {enabled, shortcut});
    if (closed) return;
    saving = false;
    if (data.error) {
      draw();
      message.textContent = data.error;
    } else {
      accept(data);
      message.textContent = data.detail || "Shortcut saved.";
    }
  }

  checkbox.addEventListener("change", () => save(checkbox.checked, current.shortcut));
  record.addEventListener("click", () => {
    editing = true;
    message.textContent = "Press Control, Option or Command with a letter, number or Space. Escape cancels.";
    draw();
  });
  function stopEditing() {
    if (!editing) return;
    editing = false;
    message.textContent = "";
    draw();
  }
  record.addEventListener("blur", stopEditing);
  record.addEventListener("keydown", event => {
    if (!editing || event.code === "Tab") return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") return stopEditing();
    if (event.repeat || ["Alt", "Control", "Meta", "Shift"].includes(event.key)) return;
    const shortcut = shortcutFromEvent(event);
    if (shortcut) save(current.enabled, shortcut);
    else message.textContent = "Use Control, Option or Command with a letter, number or Space.";
  });
  window.addEventListener("focus", refresh);
  refresh();
  return {el, leave() {closed = true; clearTimeout(timer); window.removeEventListener("focus", refresh);}};
}
