// Onboarding wizard (#setup): firmware -> pair -> pick -> name -> repo -> try it.
// Progress lives in the agent (/api/onboarding); without that endpoint it's
// kept in this browser.
import {h, clear, get, post, put, proto, store, refreshStatus, recall, remember, report, toast} from "./lib.js";
import {firmwarePanel} from "./firmware.js";

const STEPS = [
  ["firmware", "Update the firmware"],
  ["paired", "Pair with your Mac"],
  ["picked", "Pick your stick"],
  ["named", "Name it"],
  ["repo", "Your repo"],
  ["tested", "Try it"],
];
let backdrop = null, state = {}, current = 0, onClose = null, off = null, serverSide = true;

export async function open(closeCb) {
  onClose = closeCb;
  if (backdrop) return;
  const ob = await get("/api/onboarding");
  serverSide = !ob.error;
  state = serverSide ? (ob.steps || {}) : recall("onboarding", {});
  current = Math.max(0, STEPS.findIndex(([k]) => !state[k]));
  if (current < 0) current = 0;
  backdrop = h("div", {class: "wiz-backdrop", role: "dialog", "aria-modal": "true", "aria-label": "Set up your stick"});
  backdrop.addEventListener("keydown", e => { if (e.key === "Escape") close(true); });
  document.getElementById("wizardRoot").appendChild(backdrop);
  paint();
}

function close(dismissed) {
  if (off) { off(); off = null; }
  if (backdrop) backdrop.remove();
  backdrop = null;
  if (dismissed) remember("setupDismissed", true);
  if (onClose) onClose();
}

async function mark(step, done = true) {
  state[step] = done;
  if (serverSide) {
    const r = await post("/api/onboarding", {step, done});
    if (!r.error && r.steps) state = r.steps;
  } else remember("onboarding", state);
}

function next() { current = Math.min(current + 1, STEPS.length); paint(); }

function paint() {
  if (!backdrop) return;
  if (off) { off(); off = null; }
  clear(backdrop);
  const allDone = STEPS.every(([k]) => state[k]);
  const steps = h("div", {class: "wiz-steps"}, h("div", {class: "title"}, "Set up your stick"),
    STEPS.map(([k, label], i) => h("button", {type: "button", class: "wiz-step" + (i === current ? " on" : "") + (state[k] ? " done" : ""),
      onclick: () => { current = i; paint(); }}, h("span", {class: "n"}, state[k] ? "✓" : String(i + 1)), h("span", null, label))));
  const body = h("div", {class: "wiz-body"});
  if (current >= STEPS.length) doneStep(body, allDone);
  else [firmwareStep, pairStep, pickStep, nameStep, repoStep, tryStep][current](body);
  backdrop.appendChild(h("div", {class: "wiz"}, steps, body));
  const focus = body.querySelector("input, button.primary");
  if (focus) setTimeout(() => focus.focus(), 30);
}

function foot(body, {primary, primaryLabel = "Next", skipLabel = "Skip this step", skip = true} = {}) {
  body.appendChild(h("div", {class: "wiz-foot"},
    h("button", {class: "btn ghost", type: "button", onclick: () => close(true)}, "Finish later"),
    h("span", {class: "spacer"}),
    current > 0 ? h("button", {class: "btn", type: "button", onclick: () => { current--; paint(); }}, "Back") : null,
    skip ? h("button", {class: "btn", type: "button", onclick: next}, skipLabel) : null,
    primary ? h("button", {class: "btn primary", type: "button", onclick: primary}, primaryLabel) : null));
}

function head(body, title, lead) {
  body.append(h("div", {class: "eyebrow"}, `Step ${current + 1} of ${STEPS.length}`), h("h2", {class: "step-title"}, title), lead ? h("p", {class: "lead"}, lead) : null);
}

// 1 --------------------------------------------------------------------------
function firmwareStep(body) {
  head(body, "Update the firmware", "Plug the stick into this Mac with a USB-C cable. A new stick runs M5Stack's own firmware; this replaces it with the stick-key firmware. A stick that's already up to date can skip this.");
  body.appendChild(firmwarePanel({backupDefault: true, onDone: async ok => { if (ok) { await mark("firmware"); toast("Firmware done. Next: pair it."); } }}));
  foot(body, {primary: async () => { await mark("firmware"); next(); }, primaryLabel: "It's up to date — next"});
}

// 2 --------------------------------------------------------------------------
function pairStep(body) {
  head(body, "Pair with your Mac", "The stick is a Bluetooth keyboard, so the Mac pairs with it once.");
  body.appendChild(h("ol", null,
    h("li", null, "On the Mac, open ", h("b", null, "System Settings → Bluetooth"), " (", h("a", {href: "x-apple.systempreferences:com.apple.Bluetooth"}, "open it"), ")."),
    h("li", null, "Find ", h("b", null, "stick-key"), " (or its name) under Nearby Devices and click ", h("b", null, "Connect"), "."),
    h("li", null, "The stick shows a 6-digit code. If it matches the Mac's, press ", h("b", null, "A"), " on the stick to accept."),
    h("li", null, "When it says Connected, come back here.")));
  body.appendChild(h("p", {class: "small faint"}, "Already paired? Just continue."));
  foot(body, {primary: async () => { await mark("paired"); next(); }, primaryLabel: "It's paired — next", skip: false});
}

// 3 --------------------------------------------------------------------------
function pickStep(body) {
  head(body, "Pick your stick", "The agent only ever talks to the stick you pick, even with other sticks nearby.");
  const s = store.status || {};
  const msg = h("div", {class: "small dim"}, s.stick ? `Currently using ${s.stick.name}.` : "");
  const list = h("div", {class: "list"});
  const scan = async () => {
    msg.textContent = "Looking for sticks (5 s)…";
    clear(list);
    const found = await get("/api/scan");
    if (!Array.isArray(found)) { msg.textContent = found.error; return; }
    msg.textContent = found.length ? `${found.length} found, closest first.` : "None found. Is it switched on and paired?";
    for (const st of found) list.appendChild(h("div", {class: "item"},
      h("div", {class: "what"}, h("b", null, st.name), h("span", {class: "sub"}, `signal ${st.rssi} dBm`)),
      h("button", {class: "btn primary", onclick: async () => {
        if (report(await put("/api/config", {stick: {address: st.address, name: st.name}}), `Using ${st.name}`)) {
          await mark("picked"); refreshStatus(); next();
        }
      }}, "Use this one")));
  };
  body.append(h("div", {class: "row"}, h("button", {class: "btn", onclick: scan}, "Find sticks nearby"), msg), list);
  foot(body, {primary: s.stick ? async () => { await mark("picked"); next(); } : null, primaryLabel: `Keep ${s.stick ? s.stick.name : ""}`});
  scan();
}

// 4 --------------------------------------------------------------------------
function nameStep(body) {
  const cur = (store.status.stick && store.status.stick.name) || "stick-key";
  head(body, "Name it", "This is the name the Mac shows in Bluetooth settings. Handy when a team has several sticks.");
  const name = h("input", {id: "wizName", value: cur, maxlength: 20});
  body.appendChild(h("div", {class: "row"}, name));
  body.appendChild(h("p", {class: "small faint"}, "Renaming restarts the stick. The Mac may keep the old name until you re-pair it."));
  foot(body, {primary: async () => {
    const v = name.value.trim();
    if (v && v !== cur) {
      const r = await proto("name", {name: v});
      if (!report(r, `Renamed to ${v}`)) return;
      const stick = store.status.stick;
      if (stick) await put("/api/config", {stick: {...stick, name: v}});
    }
    await mark("named"); next();
  }, primaryLabel: "Save name"});
}

// 5 --------------------------------------------------------------------------
async function repoStep(body) {
  head(body, "Your repo", "Quick commands like “PR 1234” open pull requests in this repo. GitHub alerts watch it first.");
  const cfg = await get("/api/config");
  const repo = h("input", {id: "wizRepo", class: "mono", placeholder: "owner/repo", value: (cfg.github && cfg.github.primary_repo) || ""});
  body.appendChild(h("div", {class: "row"}, repo));
  body.appendChild(h("p", {class: "small faint"}, "Updates the GitHub PR and Devin review commands, and the primary repo for alerts."));
  foot(body, {primary: async () => {
    const v = repo.value.trim();
    if (!/^[\w.-]+\/[\w.-]+$/.test(v)) { toast("Use the form owner/repo, e.g. wvrdz/loom.", "err"); return; }
    const doc = await get("/api/commands");
    if (!doc.error && Array.isArray(doc.commands)) {
      for (const c of doc.commands) {
        if (typeof c.open_url !== "string") continue;
        c.open_url = c.open_url
          .replace(/(github\.com\/)[^/]+\/[^/]+(\/pull\/)/, `$1${v}$2`)
          .replace(/(app\.devin\.ai\/review\/)[^/]+\/[^/]+(\/pull\/)/, `$1${v}$2`);
      }
      report(await put("/api/commands", doc));
    }
    const gh = {...(cfg.github || {}), primary_repo: v, on: cfg.github ? cfg.github.on !== false : true, interval_s: (cfg.github && cfg.github.interval_s) || 120};
    if (report(await put("/api/config", {github: gh}), "Repo saved")) { await mark("repo"); next(); }
  }, primaryLabel: "Save repo"});
}

// 6 --------------------------------------------------------------------------
function tryStep(body) {
  head(body, "Try it", "Hold B on the stick, wait for the blip, say “what time is it”, and let go.");
  const status = h("div", {class: "notice"}, h("div", {class: "grow"}, h("b", null, "Waiting for you to ask…"), h("span", {class: "dim"}, "The answer shows here and on the stick.")));
  body.append(status, h("div", {class: "row"}, h("span", {class: "small dim"}, "No stick to hand?"),
    h("button", {class: "btn sm", onclick: () => post("/api/ask", {text: "what time is it"})}, "Type it instead")));
  const since = Date.now() / 1000;
  let heard = null;
  off = store.on((type, ev) => {
    if (type !== "event" || !ev.t || ev.t < since - 1) return;
    if (ev.kind === "heard") { heard = ev.text; status.replaceChildren(h("div", {class: "grow"}, h("b", null, `Heard: “${ev.text}”`), h("span", {class: "dim"}, "Working on it…"))); }
    if (ev.kind === "reply") {
      status.className = "notice " + (ev.ok === false ? "err" : "ok");
      status.replaceChildren(h("div", {class: "grow"}, h("b", null, ev.ok === false ? "That didn't work" : "It works!"), h("span", null, ev.text),
        heard ? h("span", {class: "small dim"}, `You asked: “${heard}”`) : null));
      if (ev.ok !== false) mark("tested");
    }
  });
  foot(body, {primary: async () => { await mark("tested"); next(); }, primaryLabel: "Finish"});
}

function doneStep(body, allDone) {
  body.append(h("div", {class: "eyebrow"}, "All set"), h("h2", {class: "step-title"}, allDone ? "Your stick is ready" : "Nearly there"),
    h("p", {class: "lead"}, allDone
      ? "Hold A to dictate with Wispr Flow, hold B to ask the agent, double-press B to unlock the Mac."
      : "Some steps were skipped. You can come back any time with “Add a stick”."),
    h("ul", {class: "small dim", style: "margin:0;padding-left:18px;display:flex;flex-direction:column;gap:4px"},
      h("li", null, "Quick commands: add your own phrases in Quick commands."),
      h("li", null, "Sounds and alerts: Sounds and Events."),
      h("li", null, "Everything the stick can do: the agent README.")));
  body.appendChild(h("div", {class: "wiz-foot"}, h("span", {class: "spacer"}),
    h("button", {class: "btn primary", type: "button", onclick: () => { remember("setupDismissed", allDone); close(false); }}, "Go to the dashboard")));
}
