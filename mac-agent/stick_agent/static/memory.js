import {api, field, get, h, post, put} from "./lib.js";

// Settings → Memory: the facts the agent keeps (memory.rs), newest first. The agent adds one only
// after you press A on the stick; here you add, edit, delete and export them, and choose whether
// the cloud brains (Claude, OpenRouter) get them too.
const endpoint = "/api/memory";

export function exportHref(items) {
  const text = JSON.stringify({items}, null, 2);
  return "data:application/json;charset=utf-8," + encodeURIComponent(text);
}

export function memoryPanel() {
  const on = h("input", {type: "checkbox", id: "memoryOn"});
  const cloud = h("input", {type: "checkbox", id: "memoryCloud"});
  const input = h("input", {id: "memoryNew", placeholder: "e.g. My standup is at 10:30", maxlength: "200", style: "flex:1;min-width:180px"});
  const add = h("button", {class: "btn", type: "button"}, "Remember");
  const list = h("div", {class: "memory-list"});
  const count = h("div", {class: "help"});
  const message = h("div", {class: "small", role: "status", "aria-live": "polite"});
  const download = h("a", {class: "btn", download: "stickybot-memory.json", href: "#"}, "Export");
  const el = h("div", {hidden: true},
    field("Memory", h("label", {class: "switch"}, on, h("span", null, "Let the agent remember facts you tell it")),
      "Say \"remember that…\" and confirm on the stick (A). It never keeps your conversations."),
    field("Cloud brains", h("label", {class: "switch"}, cloud, h("span", null, "Share these facts with Claude and OpenRouter")),
      "Off: only the local brain on this Mac sees them."),
    field("Add a fact", h("div", {class: "row", style: "gap:8px;flex-wrap:wrap"}, input, add)),
    count, list, h("div", {class: "row", style: "gap:8px"}, download), message);
  let state = null, closed = false;

  function row(item) {
    const text = h("span", {class: "memory-text"}, item.text);
    const edit = h("button", {class: "btn small", type: "button", "aria-label": "Edit: " + item.text}, "Edit");
    const del = h("button", {class: "btn small", type: "button", "aria-label": "Delete: " + item.text}, "Delete");
    const r = h("div", {class: "memory-row"}, text, h("span", {class: "dim small"}, item.source === "agent" ? "from the stick" : ""), edit, del);
    edit.addEventListener("click", () => {
      const box = h("input", {value: item.text, maxlength: "200", style: "flex:1"});
      const save = h("button", {class: "btn small", type: "button"}, "Save");
      r.replaceChildren?.(box, save);
      box.focus?.();
      const done = async () => {
        const res = await put(`${endpoint}/${encodeURIComponent(item.id)}`, {text: box.value});
        if (res.error) { message.textContent = res.error; return; }
        refresh();
      };
      save.addEventListener("click", done);
      box.addEventListener("keydown", e => { if (e.key === "Enter") done(); if (e.key === "Escape") draw(); });
    });
    del.addEventListener("click", async () => {
      const res = await api(`${endpoint}/${encodeURIComponent(item.id)}`, {method: "DELETE"});
      if (res.error) { message.textContent = res.error; return; }
      refresh();
    });
    return r;
  }

  function draw() {
    el.hidden = false;
    on.checked = state.on;
    cloud.checked = state.cloud;
    cloud.disabled = !state.on;
    while (list.firstChild) list.removeChild(list.firstChild);
    for (const item of state.items) list.appendChild(row(item));
    count.textContent = state.items.length
      ? `${state.items.length} of ${state.max} facts.`
      : "Nothing remembered yet.";
    download.setAttribute("href", exportHref(state.items));
  }

  async function refresh() {
    const data = await get(endpoint);
    if (closed) return;
    if (data.error) { if (!data.missing) { el.hidden = false; message.textContent = data.error; } return; }
    state = data;
    draw();
  }

  async function settings(change) {
    const data = await put(endpoint, change);
    if (closed) return;
    if (data.error) { message.textContent = data.error; return; }
    state = data;
    message.textContent = "Saved.";
    draw();
  }

  on.addEventListener("change", () => settings({on: on.checked}));
  cloud.addEventListener("change", () => settings({cloud: cloud.checked}));
  async function remember() {
    const res = await post(endpoint, {text: input.value});
    if (closed) return;
    if (res.error) { message.textContent = res.error; return; }
    input.value = "";
    message.textContent = "Remembered.";
    refresh();
  }
  add.addEventListener("click", remember);
  input.addEventListener("keydown", e => { if (e.key === "Enter") remember(); });
  refresh();
  return {el, leave() { closed = true; }};
}
