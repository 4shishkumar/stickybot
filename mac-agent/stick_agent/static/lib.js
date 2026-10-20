// Shared helpers for the dashboard pages: API calls, a tiny element builder,
// the shared store (agent status + live events), toasts and formatting.

export const TOKEN = window.STICK_TOKEN || "";

// Every call resolves (never throws): on failure the result has an `error`
// string, plus `missing: true` for endpoints this agent doesn't have yet.
export async function api(path, opts = {}) {
  let r;
  try {
    r = await fetch(path, {
      ...opts,
      headers: {"Content-Type": "application/json", "X-Stick-Token": TOKEN, ...(opts.headers || {})},
    });
  } catch (e) {
    return {error: "The agent isn't responding. Is it running? (stick-agent status)", offline: true};
  }
  let data = null;
  if ((r.headers.get("content-type") || "").includes("json")) {
    try { data = await r.json(); } catch (e) { data = null; }
  }
  if (!r.ok) {
    if (r.status === 404) return {error: "Not available in this version of the agent yet.", missing: true, status: 404};
    const base = data && typeof data === "object" && !Array.isArray(data) ? data : {};
    return {...base, error: base.error || `The agent answered ${r.status}.`, status: r.status};
  }
  return data === null ? {} : data;
}
export const get = path => api(path);
export const post = (path, body) => api(path, {method: "POST", body: JSON.stringify(body || {})});
export const put = (path, body) => api(path, {method: "PUT", body: JSON.stringify(body || {})});

// Runs a stick protocol command ("@state", "@brightness pct=60") over Bluetooth or USB.
export async function proto(action, args) {
  let line = "@" + action;
  if (args) {
    const q = Object.entries(args).map(([k, v]) => `${k}=${encodeURIComponent(v)}`).join("&");
    if (q) line += " " + q;
  }
  return post("/api/stick/proto", {line});
}

// h("div", {class: "x", onclick: fn}, "text", childNode, [more])
export function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "html") el.innerHTML = v;
    else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v);
    else if (k === "value") el.value = v;
    else if (k === "checked") el.checked = !!v;
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, v);
  }
  add(el, kids);
  return el;
}
function add(el, kids) {
  for (const k of kids) {
    if (k === null || k === undefined || k === false) continue;
    if (Array.isArray(k)) add(el, k);
    else el.appendChild(k instanceof Node ? k : document.createTextNode(String(k)));
  }
}
export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
export function clear(el) { while (el.firstChild) el.removeChild(el.firstChild); return el; }
export function esc(s) {
  return String(s ?? "").replace(/[&<>"]/g, c => ({"&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;"}[c]));
}

// A labelled field row: field("Model", control, "help text")
export function field(label, control, help, id) {
  if (id && control instanceof HTMLElement && !control.id) control.id = id;
  return h("div", {class: "field"},
    h("label", {for: control && control.id ? control.id : null}, label),
    control,
    help ? h("div", {class: "help"}, help) : null);
}
export function toggle(id, label, checked, onchange) {
  return h("label", {class: "switch"}, h("input", {type: "checkbox", id, checked, onchange}), h("span", null, label));
}
export function card(title, ...kids) {
  return h("section", {class: "card"}, title ? h("div", {class: "card-head"}, typeof title === "string" ? h("h2", null, title) : title) : null, ...kids);
}

// ---------- formatting ----------
export function fmtTime(t) {
  return new Date(t * 1000).toLocaleTimeString([], {hour: "numeric", minute: "2-digit"});
}
export function fmtAgo(t) {
  const s = Math.max(0, Date.now() / 1000 - t);
  if (s < 60) return "just now";
  if (s < 3600) return Math.round(s / 60) + " min ago";
  if (s < 86400) return Math.round(s / 3600) + " h ago";
  return Math.round(s / 86400) + " d ago";
}
export function fmtDay(t) {
  const d = new Date(t * 1000), today = new Date();
  if (d.toDateString() === today.toDateString()) return "Today";
  return d.toLocaleDateString([], {weekday: "short", day: "numeric", month: "short"});
}
export const STATE_LABEL = {
  ready: "Ready", listening: "Listening", transcribing: "Transcribing", thinking: "Thinking", working: "Working",
  error: "Error", paused: "Paused", "no-stick": "No stick chosen", searching: "Looking for the stick",
  starting: "Starting", connected: "Connected", offline: "Agent offline",
};

// ---------- toasts ----------
export function toast(msg, kind) {
  let box = $(".toasts");
  if (!box) { box = h("div", {class: "toasts", role: "status", "aria-live": "polite"}); document.body.appendChild(box); }
  const t = h("div", {class: "toast" + (kind === "err" ? " err" : "")}, msg);
  box.appendChild(t);
  setTimeout(() => t.remove(), kind === "err" ? 6000 : 3000);
}
// Shows the result of an API call: a toast with `ok` text, or the error.
export function report(res, ok) {
  if (res && res.error) toast(res.error, "err");
  else if (ok) toast(ok);
  return res && !res.error;
}

// ---------- storage (per-browser conveniences only) ----------
export function remember(key, value) { try { localStorage.setItem("stick." + key, JSON.stringify(value)); } catch (e) { /* private mode */ } }
export function recall(key, fallback) {
  try { const v = localStorage.getItem("stick." + key); return v === null ? fallback : JSON.parse(v); } catch (e) { return fallback; }
}

// ---------- shared store ----------
// status: the latest /api/status; events: live feed (from /api/events, newest last).
export const store = {
  status: {},
  events: [],
  online: false,
  listeners: new Set(),
  on(fn) { this.listeners.add(fn); return () => this.listeners.delete(fn); },
  emit(type, data) { for (const fn of [...this.listeners]) { try { fn(type, data); } catch (e) { console.error(e); } } },
};

export async function refreshStatus() {
  const s = await get("/api/status");
  store.online = !s.error;
  if (!s.error) store.status = s;
  store.emit("status", store.status);
  return s;
}

// Approve / deny a pending Claude Code prompt.
export async function decide(id, allow) {
  const r = await post("/api/permission/decide", {id, allow});
  report(r, allow ? "Approved" : "Denied");
  setTimeout(refreshStatus, 300);
}
export function approvalCard(p) {
  return h("div", {class: "approval"},
    h("div", null,
      h("div", {class: "row"}, h("span", {class: "tag ask"}, "Needs you"), h("b", null, p.tool), h("span", {class: "who"}, "from " + (p.from || "Claude Code"))),
    ),
    h("div", {class: "btns"},
      h("button", {class: "btn good", onclick: () => decide(p.id, true)}, "Approve"),
      h("button", {class: "btn danger", onclick: () => decide(p.id, false)}, "Deny")),
    h("div", {class: "cmd"}, p.hint || ""));
}
