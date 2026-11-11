// Dashboard shell: sidebar navigation (hash routing), the live event stream,
// status polling, and the onboarding wizard.
import {h, $, clear, store, refreshStatus, recall, remember, STATE_LABEL, get} from "./lib.js";
import * as overview from "./pages/overview.js";
import * as activity from "./pages/activity.js";
import * as commands from "./pages/commands.js";
import * as stick from "./pages/stick.js";
import * as sounds from "./pages/sounds.js";
import * as remote from "./pages/remote.js";
import * as events from "./pages/events.js";
import * as settings from "./pages/settings.js";
import * as logs from "./pages/logs.js";
import * as wizard from "./wizard.js";
import * as install from "./install.js";
import {mountBanner} from "./updates.js";

const ICON = {
  overview: '<rect x="3" y="3" width="7" height="8" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="15" width="7" height="6" rx="1.5"/>',
  activity: '<path d="M4 5h16v10H9l-5 4z"/>',
  commands: '<path d="M13 3 5 14h6l-1 7 8-11h-6z"/>',
  stick: '<rect x="7" y="2.5" width="10" height="19" rx="3"/><path d="M10 9.5v1.5M14 9.5v1.5M10.5 17.5h3"/>',
  remote: '<rect x="7" y="2.5" width="10" height="19" rx="3"/><circle cx="12" cy="7.5" r="1.6"/><path d="M10 12.5h4M10 15.5h4"/><path d="M9 1.5a5 5 0 0 1 6 0"/>',
  sounds: '<path d="M4 9.5h3l5-4v13l-5-4H4z"/><path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11"/>',
  events: '<path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M5.3 18.7l2.1-2.1M16.6 7.4l2.1-2.1"/>',
  logs: '<path d="M4 5h16v14H4z"/><path d="m7.5 10 2.5 2-2.5 2M12 14.5h4"/>',
};
const PAGES = [
  ["overview", "Overview", overview],
  ["activity", "Activity", activity],
  ["commands", "Quick commands", commands],
  ["stick", "Stick", stick],
  ["remote", "Remote", remote],
  ["sounds", "Sounds", sounds],
  ["events", "Events", events],
  ["settings", "Settings", settings],
  ["logs", "Logs", logs],
];
const byName = Object.fromEntries(PAGES.map(([n, , m]) => [n, m]));
byName.install = install;  // the setup page (#install/<step>): not in the nav, shown on its own
let currentName = null, current = null;

function navItems(name, label) {
  return [
    h("a", {href: "#" + name, "data-page": name},
      h("span", {html: `<svg viewBox="0 0 24 24" aria-hidden="true">${ICON[name] || ""}</svg>`, style: "display:contents"}),
      h("span", null, label),
      name === "overview" || name === "activity" ? h("span", {class: "badge", "data-badge": "", hidden: true}) : null),
    h("a", {href: "#" + name, "data-page": name}, label),
  ];
}

function buildNav() {
  const nav = $("#nav"), mob = $("#mobileNav");
  for (const [name, label] of PAGES) {
    const [side, top] = navItems(name, label);
    nav.appendChild(side);
    mob.appendChild(top);
  }
}

// A page another script adds to the nav (the Rust app's Feedback page, from /static-rs/):
// stickyAddPage("feedback", "Feedback", module, '<path …/>', "logs") puts it before Logs.
window.stickyAddPage = (name, label, mod, icon, before) => {
  if (byName[name]) return;
  byName[name] = mod;
  ICON[name] = icon || "";
  const [side, top] = navItems(name, label);
  const at = sel => (before ? $(`${sel} [data-page="${before}"]`) : null);
  $("#nav").insertBefore(side, at("#nav"));
  $("#mobileNav").insertBefore(top, at("#mobileNav"));
  side.classList.toggle("on", currentName === name);
  if (currentName !== name && (location.hash.slice(1).split("/")[0] || recall("page", "overview")) === name) route();
};

function setHead(page) {
  $("#pageTitle").textContent = page.title || "";
  $("#pageSub").textContent = page.subtitle || "";
  const acts = clear($("#pageActions"));
  if (page.actions) for (const a of page.actions()) acts.appendChild(a);
}

function show(name) {
  if (!byName[name]) name = "overview";
  if (name === currentName) return;
  if (current && current.leave) current.leave();
  currentName = name;
  current = byName[name];
  document.body.classList.toggle("install-mode", name === "install");
  if (name !== "install") remember("page", name);
  for (const a of document.querySelectorAll("[data-page]")) a.classList.toggle("on", a.dataset.page === name);
  setHead(current);
  const root = clear($("#page"));
  root.scrollTop = 0;
  window.scrollTo(0, 0);
  current.render(root);
}

function route() {
  const hash = location.hash.slice(1);
  if (hash === "setup") {
    if (!currentName) show(recall("page", "overview"));
    wizard.open(() => { history.replaceState(null, "", "#" + (currentName || "overview")); });
    return;
  }
  show(hash.split("/")[0] || recall("page", "overview"));  // #install/<step>: the page follows the step
}

// ---------- status pill + approvals badge ----------
function paintStatus(s) {
  const pill = $("#statePill");
  const state = store.online ? (s.state || "starting") : "offline";
  pill.className = "pill s-" + state;
  $("#stateText").textContent = STATE_LABEL[state] || state;
  pill.title = store.online ? (s.detail || "") : "The agent isn't responding";
  const n = (s.prompts || []).length;
  for (const b of document.querySelectorAll("[data-badge]")) { b.hidden = !n; b.textContent = n; }
  $("#brandSub").textContent = s.stick ? s.stick.name : "your Mac, by voice";
  document.title = n ? `(${n}) StickyBot` : "StickyBot";
}

// ---------- live events ----------
function connectEvents() {
  const es = new EventSource("/api/events");
  es.onopen = () => { store.events = []; store.emit("reset"); refreshStatus(); };
  es.onmessage = e => {
    let ev;
    try { ev = JSON.parse(e.data); } catch (err) { return; }
    if (ev.kind === "state") {
      store.status = {...store.status, state: ev.state, detail: ev.detail};
      store.online = true;
      paintStatus(store.status);
    } else {
      store.events.push(ev);
      if (store.events.length > 600) store.events.splice(0, store.events.length - 600);
    }
    if (["prompt", "prompt_done", "link", "config", "sounds", "reply"].includes(ev.kind)) refreshStatus();
    store.emit("event", ev);
  };
  es.onerror = () => { store.online = false; paintStatus(store.status); };
}

async function maybeOnboard() {
  if (location.hash === "#setup" || location.hash.startsWith("#install")) return;
  const ins = await get("/api/install");  // a new install: the setup page first (it includes the stick)
  if (ins && !ins.error && ins.pending && !recall("installDismissed", false) && !(await refreshStatus()).stick) {
    location.hash = "#install";
    return;
  }
  if (recall("setupDismissed", false)) return;
  const ob = await get("/api/onboarding");
  const s = await refreshStatus();
  const needed = ob && !ob.error ? !ob.done : (!s.error && !s.stick);
  if (needed) location.hash = "#setup";
}

store.on((type, data) => { if (type === "status") paintStatus(data); });
buildNav();
mountBanner($("#updateBanner"));
$("#addStick").addEventListener("click", () => { location.hash = "#setup"; });
window.addEventListener("hashchange", route);
route();
connectEvents();
refreshStatus();
setInterval(refreshStatus, 5000);
maybeOnboard();
