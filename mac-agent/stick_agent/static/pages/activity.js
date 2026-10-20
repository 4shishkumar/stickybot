// Activity: the conversation with the agent, live. Your words on the right,
// the agent's replies on the left, what it's doing in between.
import {h, clear, store, get, post, report, fmtTime, fmtDay, recall, remember, decide} from "../lib.js";

export const title = "Activity";
export const subtitle = "What you asked and what the agent did. Typing here works like holding B and speaking.";

let off = null, chat = null, showLogs = false, oldestShown = null, lastDay = null, histN = 0;

export function actions() {
  return [
    h("label", {class: "switch", title: "Show the agent's log lines in the conversation"},
      h("input", {type: "checkbox", id: "showLogs", checked: recall("showLogs", false), onchange: e => {
        showLogs = e.target.checked; remember("showLogs", showLogs); rebuild();
      }}), h("span", {class: "small dim"}, "Log lines")),
    h("button", {class: "btn", type: "button", title: "The next request starts a fresh Claude conversation", onclick: async () => {
      report(await post("/api/new-chat"), "The next request starts a new conversation");
    }}, "New conversation"),
  ];
}

export function render(root) {
  showLogs = recall("showLogs", false);
  const more = h("button", {class: "btn sm ghost", type: "button", style: "align-self:center", onclick: loadEarlier}, "Show earlier");
  chat = h("div", {class: "chat", "aria-live": "polite"});
  const input = h("input", {id: "askInput", placeholder: "Ask anything — “open PR 1234”, “what's the weather?”", autocomplete: "off"});
  const send = async () => {
    const text = input.value.trim();
    if (!text) return;
    input.value = "";
    report(await post("/api/ask", {text}));
  };
  input.addEventListener("keydown", e => { if (e.key === "Enter") send(); });
  const composer = h("div", {class: "composer"}, input, h("button", {class: "btn primary", type: "button", onclick: send}, "Send"));
  root.appendChild(h("div", {class: "chat-wrap"}, more, chat, composer));
  rebuild();
  off = store.on((type, ev) => {
    if (type === "reset") rebuild();
    else if (type === "event") { append(ev); scrollDown(); }
    else if (type === "status") refreshPrompts();
  });
  setTimeout(() => input.focus(), 50);
}

export function leave() { if (off) off(); chat = null; }

function rebuild() {
  if (!chat) return;
  clear(chat);
  lastDay = null;
  oldestShown = null;
  histN = 0;
  const evs = store.events;
  if (!evs.some(e => ["heard", "reply", "event", "prompt"].includes(e.kind))) {
    chat.appendChild(h("div", {class: "empty", style: "align-self:center;text-align:center"},
      "Nothing yet this session. Hold B on the stick and speak, or type below. “Show earlier” loads your history."));
  }
  for (const ev of evs) append(ev);
  scrollDown(true);
}

function scrollDown(force) {
  const nearBottom = window.innerHeight + window.scrollY > document.body.scrollHeight - 160;
  if (force || nearBottom) window.scrollTo(0, document.body.scrollHeight);
}

function dayMark(t) {
  const d = fmtDay(t);
  if (d === lastDay) return null;
  lastDay = d;
  return h("div", {class: "day"}, d);
}

function node(ev) {
  const time = ev.t ? fmtTime(ev.t) : "";
  switch (ev.kind) {
    case "heard":
      return h("div", {class: "msg you"},
        h("div", {class: "bubble"}, ev.text),
        h("div", {class: "meta"}, ev.source === "typed" ? "typed" : "said on the stick", "·", time));
    case "reply": {
      const via = ev.via === "quick" ? "quick command" : ev.via === "claude" ? "Claude" : ev.via || "";
      return h("div", {class: "msg agent" + (ev.ok === false ? " err" : "")},
        h("div", {class: "bubble"}, ev.text),
        h("div", {class: "meta"}, via, ev.ms ? `· ${(ev.ms / 1000).toFixed(1)} s` : "", "·", time));
    }
    case "progress":
      return h("div", {class: "line work"}, ev.text);
    case "prompt": {
      const el = h("div", {class: "approval", "data-prompt": ev.id, style: "align-self:stretch"},
        h("div", null, h("div", {class: "row"}, h("span", {class: "tag ask"}, "Needs you"), h("b", null, ev.tool),
          h("span", {class: "who"}, "from " + (ev.origin || "Claude Code")))),
        h("div", {class: "btns"},
          h("button", {class: "btn good", onclick: () => decide(ev.id, true)}, "Approve"),
          h("button", {class: "btn danger", onclick: () => decide(ev.id, false)}, "Deny")),
        h("div", {class: "cmd"}, ev.hint || ""));
      return el;
    }
    case "prompt_done": {
      const card = chat && chat.querySelector(`[data-prompt="${CSS.escape(ev.id || "")}"]`);
      if (card) card.querySelector(".btns").replaceChildren(h("span", {class: "tag " + (ev.decision === "allow" ? "ok" : ev.decision === "deny" ? "err" : "")},
        ev.decision === "allow" ? "Approved" : ev.decision === "deny" ? "Denied" : "No answer — asked in the terminal"));
      return null;
    }
    case "event": {
      const ico = {celebrate: "🎉", nice: "✓", headsup: "!"}[ev.level] || "•";
      return h("div", {class: "sys " + (ev.level || "")},
        h("span", {class: "ico"}, ico),
        h("span", null, h("b", null, ev.title || ""), ev.sub ? h("span", {class: "dim"}, " · " + ev.sub) : null),
        ev.sent === false ? h("span", {class: "tag"}, "stick offline") : null);
    }
    case "spoken":
      return h("div", {class: "line"}, "Read aloud");
    case "info":
      return h("div", {class: "line"}, ev.text);
    case "log":
      return showLogs ? h("div", {class: "line logl"}, ev.text) : null;
    default:
      return null;
  }
}

function append(ev) {
  if (!chat) return;
  const el = node(ev);
  if (!el) return;
  const empty = chat.querySelector(".empty");
  if (empty) empty.remove();
  if (ev.t && ["heard", "event", "prompt"].includes(ev.kind)) { const d = dayMark(ev.t); if (d) chat.appendChild(d); }
  if (ev.t && (oldestShown === null || ev.t < oldestShown)) oldestShown = ev.t;
  chat.appendChild(el);
}

function refreshPrompts() {
  if (!chat) return;
  const open = new Set((store.status.prompts || []).map(p => p.id));
  for (const card of chat.querySelectorAll("[data-prompt]")) {
    if (!open.has(card.dataset.prompt) && card.querySelector(".btn.good")) {
      card.querySelector(".btns").replaceChildren(h("span", {class: "tag"}, "Answered"));
    }
  }
}

async function loadEarlier(e) {
  const btn = e.currentTarget;
  btn.disabled = true;
  histN += 60;
  const rows = await get("/api/history?n=" + histN);
  btn.disabled = false;
  if (!Array.isArray(rows)) return report(rows);
  const before = oldestShown === null ? Infinity : oldestShown - 1;
  const older = rows.filter(r => r.t < before).reverse();  // oldest first
  if (!older.length) { btn.textContent = "No earlier history"; btn.disabled = true; return; }
  const frag = document.createDocumentFragment();
  let day = null;
  for (const r of older) {
    const d = fmtDay(r.t);
    if (d !== day) { frag.appendChild(h("div", {class: "day"}, d)); day = d; }
    frag.appendChild(node({kind: "heard", t: r.t, text: r.heard, source: r.source}));
    frag.appendChild(node({kind: "reply", t: r.t, text: r.reply, via: r.via === "quick" && r.command ? "quick" : r.via, ok: r.ok}));
  }
  const empty = chat.querySelector(".empty");
  if (empty) empty.remove();
  chat.insertBefore(frag, chat.firstChild);
  oldestShown = older[0].t;
  btn.textContent = "Show earlier";
}
