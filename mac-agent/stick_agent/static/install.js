// The setup page (#install/<step>): what install.sh opens. One step per screen, a progress
// bar of steps, Install buttons with live progress (the agent's /api/install jobs), Skip on
// the optional ones. The step lives in the URL, so a reload resumes. "Your stick" hands off
// to the setup wizard (wizard.js); the repo goes through the same call as the wizard's.
import {h, clear, get, post, toast, report, remember, store} from "./lib.js";
import {streamJob} from "./updates.js";
import * as wizard from "./wizard.js";

// el.append(), but leaving out null / undefined / false (append() would show them as text, e.g. "null")
const append = (el, ...kids) => el.append(...kids.filter(k => k !== null && k !== undefined && k !== false));

export const title = "Set up StickyBot";
export const subtitle = "";

const DOCS = "https://github.com/4shishkumar/stickybot/blob/main/mac-agent/README.md";
const TEAM_DOCS = "https://github.com/4shishkumar/stickybot/blob/main/docs/TEAMMATE-SETUP.md";
const KIND = {required: "", recommended: "Recommended", optional: "Optional"};
const PILL = {done: ["ok", "Done"], todo: ["", "To do"], skipped: ["", "Skipped"], unavailable: ["warn", "Not available"]};
const POLL_MS = 4000;

let root = null, list = null, cur = null, step = null, es = null, timer = null, shownKey = "", alive = false;

export function render(el) {
  root = el;
  alive = true;
  window.addEventListener("hashchange", onHash);
  clear(root).appendChild(shell(h("div", {class: "empty"}, "Checking this Mac…")));
  load();
}

export function leave() {
  alive = false;
  window.removeEventListener("hashchange", onHash);
  stopStream();
  clearTimeout(timer);
}

function hashStep() {
  const m = location.hash.match(/^#install\/(\w+)/);
  return m ? m[1] : null;
}
const ids = () => (list ? list.steps.map(s => s.id) : []);

async function load() {
  const r = await get("/api/install");
  if (!alive) return;
  if (r.error) {
    // first start: the agent may still be coming up (or loading the speech model)
    clear(root).appendChild(shell(h("div", {class: "notice"}, h("div", {class: "grow"},
      h("b", null, r.missing ? "This version of the agent has no setup page" : "Waiting for the agent…"),
      h("span", {class: "dim"}, r.missing ? "Update it, or use stickybot setup in Terminal." : "It's starting up. This page retries by itself.")))));
    if (!r.missing) timer = setTimeout(load, 2000);
    return;
  }
  list = r;
  const want = hashStep();
  cur = ids().includes(want) ? want : r.next;
  if (want !== cur) history.replaceState(null, "", "#install/" + cur);
  step = list.steps.find(s => s.id === cur);
  paint();
  schedule();
}

function onHash() {
  const id = hashStep();
  if (!location.hash.startsWith("#install") || !list) return;
  const want = ids().includes(id) ? id : list.next;
  if (want === cur) return;
  cur = want;
  step = list.steps.find(s => s.id === cur);
  stopStream();
  paint();
  refresh();
}

function go(id) { location.hash = "#install/" + id; }
function goNext() { const i = ids().indexOf(cur); go(ids()[Math.min(i + 1, ids().length - 1)]); }

// Re-reads the current step (with full: every step, for the progress bar); repaints only on a change.
// Polls read just the one step: the full listing runs every check (claude, gh, …).
async function refresh(full = true) {
  clearTimeout(timer);
  const [one, all] = await Promise.all([get(`/api/install/${cur}`), full ? get("/api/install") : null]);
  if (!alive) return;
  if (all && !all.error) list = all;
  if (!one.error && one.id === cur) {
    step = one;
    const i = list.steps.findIndex(s => s.id === cur);
    if (i >= 0) list.steps[i] = one;
  }
  if (key(step) !== shownKey && !es) paint();
  else paintBar();
  schedule();
}

// Steps that wait for something outside the page (a Terminal login, a browser, the stick) poll.
function schedule() {
  clearTimeout(timer);
  if (!alive || !step) return;
  const waits = step.state === "todo" && (["claude", "github", "stick", "model"].includes(step.id) ||
    (step.id === "wispr" && step.data && step.data.tap_mode && step.data.on));  // waiting for the permission
  if (waits) timer = setTimeout(() => refresh(false), POLL_MS);
}

function key(s) {
  if (!s) return "";
  const d = {...(s.data || {})};
  delete d.bytes;
  return `${s.id}|${s.state}|${s.detail}|${s.job}|${JSON.stringify(d)}`;
}

function stopStream() { if (es) { es.close(); es = null; } }

// ---------- layout ----------
function shell(body) {
  return h("div", {class: "inst"},
    h("header", {class: "inst-top"},
      h("div", {class: "brand-mark", "aria-hidden": "true"}, h("div", {class: "brand-eyes"}, h("i"), h("i"))),
      h("div", {class: "grow"}, h("div", {class: "brand-name"}, "Set up StickyBot"),
        h("div", {class: "small faint"}, "One step at a time. Optional steps can be skipped and done later.")),
      h("a", {class: "btn ghost sm", href: "#overview", onclick: () => remember("installDismissed", true)}, "Skip to dashboard")),
    h("nav", {class: "inst-steps", id: "instSteps", "aria-label": "Setup steps"}),
    body);
}

function paintBar() {
  const nav = document.getElementById("instSteps");
  if (!nav || !list) return;
  clear(nav);
  const steps = list.steps.filter(s => s.id !== "done");
  const settled = steps.filter(s => s.state !== "todo").length;
  nav.appendChild(h("div", {class: "inst-bar", role: "progressbar", "aria-valuemin": 0, "aria-valuemax": steps.length, "aria-valuenow": settled},
    h("i", {style: `width:${Math.round(settled * 100 / steps.length)}%`})));
  nav.appendChild(h("ol", {class: "inst-dots"}, list.steps.map((s, i) => h("li", null,
    h("button", {type: "button", class: `inst-dot ${s.state}${s.id === cur ? " on" : ""}`, title: `${s.title}: ${PILL[s.state] ? PILL[s.state][1] : s.state}`,
      "aria-current": s.id === cur ? "step" : null, onclick: () => go(s.id)},
      h("span", {class: "n"}, s.state === "done" ? "✓" : s.state === "skipped" ? "–" : s.state === "unavailable" ? "!" : String(i + 1)),
      h("span", {class: "t"}, s.title))))));
}

function paint() {
  if (!root || !list || !step) return;
  shownKey = key(step);
  const i = ids().indexOf(cur);
  const body = h("section", {class: "card inst-card"});
  const [pc, pl] = PILL[step.state] || ["", step.state];
  append(body,
    h("div", {class: "eyebrow"}, `Step ${i + 1} of ${ids().length}` + (KIND[step.kind] ? ` · ${KIND[step.kind]}` : "")),
    h("div", {class: "row"}, h("h2", {class: "step-title"}, step.title),
      step.id !== "done" ? h("span", {class: "pill " + pc}, h("span", {class: "dot"}), pl) : null),
    h("p", {class: "lead"}, step.why),
    needs(step.needs || {}));
  const content = h("div", {class: "inst-content"});
  (RENDER[step.id] || (() => null))(content, step);
  body.appendChild(content);
  body.appendChild(foot(i));
  clear(root).appendChild(shell(body));
  paintBar();
  const f = body.querySelector("input, button.primary");
  if (f && document.activeElement === document.body) setTimeout(() => f.focus({preventScroll: true}), 30);
}

function needs(n) {
  const bits = [];
  if (n.download) bits.push(["Download", n.download]);
  if (n.admin) bits.push(["Mac password", "macOS asks in its own window"]);
  if (n.terminal) bits.push(["Terminal", n.terminal]);
  if (n.browser) bits.push(["Browser", n.browser]);
  if (n.permission) bits.push(["Permission", n.permission]);
  if (!bits.length) return null;
  return h("div", {class: "row inst-needs"}, bits.map(([k, v]) => h("span", {class: "tag"}, h("b", null, k), " " + v)));
}

function foot(i) {
  const s = step;
  const skip = s.skippable && s.state === "todo"
    ? h("button", {class: "btn", type: "button", onclick: () => doSkip("skip")}, "Skip for now") : null;
  const unskip = s.skippable && s.state === "skipped"
    ? h("button", {class: "btn", type: "button", onclick: () => doSkip("unskip")}, "Do this step") : null;
  const last = i >= ids().length - 1;
  const nextLabel = s.id === "welcome" ? "Let's start" : s.state === "done" || s.state === "unavailable" || s.state === "skipped" ? "Next" : "Next without it";
  return h("div", {class: "wiz-foot"},
    i > 0 ? h("button", {class: "btn ghost", type: "button", onclick: () => go(ids()[i - 1])}, "Back") : null,
    h("span", {class: "spacer"}), skip, unskip,
    last ? null : h("button", {class: "btn" + (s.state !== "todo" || s.id === "welcome" ? " primary" : ""), type: "button",
      disabled: s.id === "welcome" && s.state === "unavailable" ? true : null, onclick: goNext}, nextLabel));
}

async function doSkip(action) {
  const r = await post(`/api/install/${cur}/${action}`);
  if (!report(r)) return;
  list = r;
  step = list.steps.find(s => s.id === cur);
  if (action === "skip") goNext(); else paint();
}

// ---------- shared bits ----------
function copyable(text) {
  const btn = h("button", {class: "btn sm", type: "button", onclick: async () => {
    try { await navigator.clipboard.writeText(text); btn.textContent = "Copied"; setTimeout(() => { btn.textContent = "Copy"; }, 1500); }
    catch (e) { toast("Couldn't copy — select the text instead", "err"); }
  }}, "Copy");
  return h("div", {class: "inst-cmd"}, h("code", null, text), btn);
}

function note(kind, title, ...text) {
  return h("div", {class: "notice" + (kind ? " " + kind : "")}, h("div", {class: "grow"}, h("b", null, title), ...text.map(t => typeof t === "string" ? h("span", {class: "dim"}, t) : t)));
}

// Starts the step's job (or re-attaches to a running one) and streams it into `out`.
async function runJob(out, {body, onEvent, after} = {}) {
  let job = step.job;
  if (!job) {
    const r = await post(`/api/install/${cur}/run`, body || {});
    if (r.error && !r.job) { toast(r.error, "err"); return; }
    job = r.job;
  }
  const id = cur;
  out.hidden = false;
  stopStream();
  es = streamJob("/api/install/jobs", job, out, (ok, err) => {
    es = null;
    if (id !== cur) return;
    toast(ok ? `${step.title}: done` : `${step.title}: ${err || "didn't work"}`, ok ? null : "err");
    if (after) after(ok);
    setTimeout(refresh, ok ? 600 : 1500);
  }, onEvent);
}

function jobBox() { return h("div", {class: "inst-job", hidden: true}); }

function actionRow(...kids) { return h("div", {class: "row inst-actions"}, ...kids); }

// ---------- the wireless mic ----------
// Once on: rank it in Wispr ("Show the mic for 60 s" makes it appear without a hold).
function rankSteps(d, blackhole) {
  const showBtn = h("button", {class: "btn", type: "button", onclick: async () => {
    const r = await post("/api/wireless-mic/show", {secs: 60});
    if (report(r, `“${r.name || d.mic}” is there for ${r.secs || 60} s — rank it now`)) showBtn.disabled = true;
    setTimeout(() => { showBtn.disabled = false; }, 60000);
  }}, "Show the mic for 60 s");
  return h("div", null,
    h("p", {class: "small"}, h("b", null, "Once: rank it in Wispr. ")),
    h("ol", {class: "small dim inst-ul"},
      h("li", null, "Press the button below (or hold A on the stick, away from USB) so the mic appears."),
      h("li", null, "In Wispr Flow → Settings → Microphone, drag ", h("b", null, `“${d.mic}”`), " to the top."),
      h("li", null, blackhole ? "Don't pick BlackHole 2ch itself. The rest of the time Wispr uses your usual mic."
        : "The rest of the time Wispr uses your usual mic.")),
    actionRow(showBtn));
}

async function checkTap(btn) {
  if (btn) btn.disabled = true;
  const r = await post("/api/wireless-mic/check", {});
  if (r.ok) toast("The Apple audio tap works");
  else toast(r.detail || r.error || "The Apple audio tap isn't allowed yet", "err");
  refresh();
}

function useTapInstead(out) {
  return h("details", null, h("summary", {class: "small"}, "No driver needed: use the Apple audio tap instead"),
    h("p", {class: "small dim"}, "This Mac can carry the stick's audio with an Apple audio tap of StickyBot's own sound (macOS 14.2+), so BlackHole isn't needed."),
    actionRow(h("button", {class: "btn sm", type: "button", onclick: e => { e.target.disabled = true; runJob(out, {body: {carrier: "tap"}}); }}, "Use the Apple audio tap")));
}

// What macOS calls the agent in its prompts: "StickyBot" (StickyBot.app) or "Python" (before it's set up)
const appName = () => ((store.status || {}).app || {}).name || "StickyBot";

function wisprTap(el, s, d, out) {
  const tap = d.tap || {};
  const prompt = h("div", {class: "notice", style: "margin:8px 0"}, h("div", {class: "grow"},
    h("b", null, "macOS will ask once: "),
    h("span", null, "allow “System Audio Recording” for ", h("b", null, appName()), appName() === "Python" ? " — that's StickyBot (StickyBot.app isn't set up yet)." : ".", " It only taps its own sound (the stick's audio it plays), muted, never other apps.")));
  const blackholeInstead = h("details", null, h("summary", {class: "small"}, "Use BlackHole instead"),
    h("p", {class: "small dim"}, `BlackHole 2ch is a free audio driver by Existential Audio. StickyBot downloads the tested version ${d.pinned}, checks its checksum and signature, then macOS asks for your Mac password.`),
    actionRow(h("button", {class: "btn sm", type: "button", onclick: e => { e.target.disabled = true; runJob(out, {body: {carrier: "blackhole"}}); }},
      d.blackhole ? "Use BlackHole 2ch" : "Install BlackHole 2ch")));
  if (!d.on) {
    append(el, h("p", {class: "small dim"}, "This Mac needs no extra driver: an Apple audio tap of StickyBot's own sound carries the stick's mic to Wispr Flow."),
      prompt,
      actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out, {body: {carrier: "tap"}}); }}, "Turn on")), out,
      blackholeInstead);
    if (s.job) runJob(out);
    return;
  }
  if (s.state === "done" && tap.carrier === "tap") {
    append(el, note("ok", "The wireless mic is on", "Carried by the Apple audio tap — no driver."), rankSteps(d, false), blackholeInstead);
    return;
  }
  if (s.state === "done") {  // auto: BlackHole carries it while the tap isn't allowed
    append(el, note("ok", "The wireless mic is on", s.detail), rankSteps(d, true),
      h("p", {class: "small dim"}, "To use the Apple audio tap instead (no driver): ", tap.hint, ", then press Check."),
      actionRow(h("button", {class: "btn sm", type: "button", onclick: e => checkTap(e.target)}, "Check")));
    return;
  }
  const checking = tap.state === "checking" || tap.state == null;
  append(el, checking ? note("", "Checking the Apple audio tap…", "An inaudible check. If macOS asks about System Audio Recording, press Allow.")
    : note("warn", "Allow System Audio Recording", tap.state === "failed" ? tap.hint + "." : tap.detail),
    prompt,
    h("ol", {class: "small dim inst-ul"},
      h("li", null, "If macOS showed a prompt, press Allow."),
      h("li", null, `Otherwise open System Settings → Privacy & Security → Screen & System Audio Recording, and under “System Audio Recording Only” turn on ${appName()} (add it with + if it isn't listed).`),
      h("li", null, "Then press Check.")),
    actionRow(h("button", {class: "btn primary", type: "button", disabled: tap.state === "checking", onclick: e => checkTap(e.target)}, "Check"),
      h("a", {class: "btn ghost sm", href: "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture", target: "_blank", rel: "noopener"}, "Open System Settings")),
    out, blackholeInstead);
  if (s.job) runJob(out);
}

// ---------- the steps ----------
const RENDER = {
  welcome(el, s) {
    const lvl = {ok: ["ok", "OK"], warn: ["warn", "Check"], err: ["err", "Problem"]};
    el.appendChild(h("div", {class: "list"}, (s.data.checks || []).map(c => h("div", {class: "item"},
      h("div", {class: "what"}, h("b", null, c.label), h("span", {class: "sub"}, c.detail)),
      h("span", {class: "pill " + lvl[c.level][0]}, h("span", {class: "dot"}), lvl[c.level][1])))));
    el.appendChild(h("p", {class: "small dim"}, "The app is installed and running (look for its icon in the menu bar). The next steps add the parts you want. ",
      "Each one says what it needs before it does anything."));
  },

  model(el, s) {
    const d = s.data || {};
    if (s.state === "done") {
      el.appendChild(note("ok", "The speech model is ready", d.stt_ready ? "Hold B on the stick and speak." : "The agent is loading it (a few seconds)."));
      return;
    }
    const out = jobBox();
    if (d.agent_downloading && !s.job) {
      el.appendChild(note("", "Already downloading in the background",
        `${(d.bytes / 1e9).toFixed(1)} GB so far. Press Download to see the progress.`));
    }
    append(el, h("p", {class: "small dim"}, `${d.repo} — about 2.5 GB, downloaded once from Hugging Face. A few minutes on a good connection.`),
      actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out); }}, "Download the speech model")), out);
    if (s.job) runJob(out);
  },

  claude(el, s) {
    const d = s.data || {};
    if (!d.installed) {
      const out = jobBox();
      append(el, h("p", {class: "small dim"}, "Not installed yet. The button runs Anthropic's official installer (it installs into ~/.local/bin, no password needed):"),
        copyable(d.install_cmd),
        actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out); }}, "Install Claude Code")), out,
        h("p", {class: "small faint"}, "Already use Claude Code somewhere else? Set its path in Settings → Claude later."));
      if (s.job) runJob(out);
      return;
    }
    if (!d.logged_in) {
      append(el, note("warn", "One thing to do in Terminal: log in",
        "Open Terminal, run the command below, and log in with your Claude account (a browser window opens). No API key is needed.",
        "This page notices by itself when you're logged in."), copyable("claude"),
        h("div", {class: "row small dim"}, h("span", {class: "spinner", "aria-hidden": "true"}), "Waiting for the login…"));
      return;
    }
    el.appendChild(note("ok", "Claude Code is ready", `${d.version || "installed"} · logged in`));
  },

  hooks(el, s) {
    const d = s.data || {};
    if (s.state === "unavailable") { el.appendChild(note("err", "Can't add the hooks", s.detail)); return; }
    if (s.state === "done") {
      el.appendChild(note("ok", "The hooks are in place", `In ${d.path}. Remove them any time in Settings → Components → Claude Code integration.`));
      return;
    }
    if (!d.claude_installed) el.appendChild(note("", "Claude Code isn't installed yet", "You can still add them: they start working once it is."));
    const out = jobBox();
    append(el, h("p", {class: "small dim"}, "This adds three hooks to ", h("code", null, d.path),
      ". A dated backup of the file is saved first; your other settings and hooks stay as they are. Nothing changes until you press the button."),
      h("details", {open: true}, h("summary", {class: "small"}, "Exactly what gets added"), h("pre", {class: "log"}, JSON.stringify(d.preview, null, 2))),
      h("ul", {class: "small dim inst-ul"},
        h("li", null, h("b", null, "PermissionRequest"), " — Claude Code's approval prompts go to the stick (A approve, B deny) and the dashboard."),
        h("li", null, h("b", null, "UserPromptSubmit + Stop"), " — a \"Claude finished\" alert on the stick after long tasks.")),
      actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out); }}, "Add the hooks")), out);
    if (s.job) runJob(out);
  },

  mcp(el, s) {
    const d = s.data || {};
    if (s.state === "unavailable") {
      append(el, note("warn", "Not available yet", s.detail), d.conflict ? null :
        actionRow(h("button", {class: "btn", type: "button", onclick: () => go("claude")}, "Go to Claude Code")));
      return;
    }
    if (s.state === "done") { el.appendChild(note("ok", "Added to Claude Code", `${s.detail}. New sessions have the mcp__stick__* tools.`)); return; }
    const out = jobBox();
    append(el, h("p", {class: "small dim"}, "Then you can ask any Claude Code session things like “run the tests and notify me on the stick when done”. ",
      "The button runs Claude Code's own command (your token is filled in; it stays on this Mac):"),
      d.command ? h("pre", {class: "log"}, d.command) : null,
      actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out); }}, "Add the stick tools")), out);
    if (s.job) runJob(out);
  },

  github(el, s) {
    const d = s.data || {};
    if (!d.gh) {
      append(el, note("", "First, the GitHub CLI (gh)", "Install it in Terminal, then press Check again."),
        d.brew ? null : h("p", {class: "small dim"}, "No Homebrew yet? Install it first (it asks for your password):"),
        d.brew ? null : copyable(d.brew_install),
        copyable(d.brew_cmd),
        actionRow(h("button", {class: "btn", type: "button", onclick: refresh}, "Check again")));
      return;
    }
    if (!d.logged_in) {
      const out = jobBox();
      const codeBox = h("div", {class: "inst-code-box"});
      const showCode = (code, url) => {
        append(clear(codeBox), 
          h("div", {class: "small dim"}, "Your one-time code"),
          h("div", {class: "inst-code"}, code),
          actionRow(
            h("button", {class: "btn sm", type: "button", onclick: () => navigator.clipboard.writeText(code).then(() => toast("Code copied"), () => {})}, "Copy code"),
            h("a", {class: "btn primary", href: url, target: "_blank", rel: "noopener"}, "Open github.com/login/device")),
          h("div", {class: "small dim"}, "Paste the code there and approve. This page continues by itself."),
          h("button", {class: "btn ghost sm", type: "button", onclick: async () => { const job = (step.data.login || {}).job || step.job; if (job) await post(`/api/install/jobs/${job}/cancel`); }}, "Cancel"));
      };
      const start = h("button", {class: "btn primary", type: "button", onclick: e => {
        e.target.disabled = true;
        runJob(out, {onEvent: m => { if (m.code) showCode(m.code, m.url); }});
      }}, "Log in to GitHub");
      append(el, h("p", {class: "small dim"}, "gh is installed but not logged in. The button starts GitHub's own login: you get a one-time code to enter on github.com."),
        actionRow(start), codeBox, out,
        h("p", {class: "small faint"}, "Prefer Terminal? Run ", h("code", null, "gh auth login"), " there — this page notices by itself."));
      if (d.login && d.login.code) showCode(d.login.code, d.login.url);
      if (s.job) { start.disabled = true; runJob(out, {onEvent: m => { if (m.code) showCode(m.code, m.url); }}); }
      return;
    }
    const repo = h("input", {class: "mono", placeholder: "owner/repo", value: d.repo || "", "aria-label": "Main repo", spellcheck: "false"});
    const save = async () => { if (await wizard.saveRepo(repo.value.trim(), true)) refresh(); };
    repo.addEventListener("keydown", e => { if (e.key === "Enter") save(); });
    if (s.state === "done") el.appendChild(note("ok", "GitHub alerts are on", `Watching ${d.repo} and your own PRs.`));
    append(el, h("p", {class: "small dim"}, s.state === "done" ? "Change the main repo:" : "Logged in. Which repo do you work on most? Alerts watch it first, and “PR 1234” on the stick opens its pull requests."),
      actionRow(repo, h("button", {class: "btn primary", type: "button", onclick: save}, s.state === "done" ? "Save" : "Turn on alerts")));
  },

  wispr(el, s) {
    const d = s.data || {};
    const out = jobBox();
    if (d.tap_mode) { wisprTap(el, s, d, out); return; }
    if (!d.blackhole) {
      append(el, h("p", {class: "small dim"}, "The stick's mic reaches Wispr Flow through ", h("b", null, "BlackHole 2ch"),
        ", a free audio driver by Existential Audio. It stays hidden: Wispr is never set to it."),
        h("ol", {class: "small dim inst-ul"},
          h("li", null, `StickyBot downloads the tested version ${d.pinned} from existential.audio,`),
          h("li", null, "checks its checksum and that it's signed by Existential Audio and notarized by Apple,"),
          h("li", null, "then macOS asks for your Mac password in its own window (StickyBot never sees it).")),
        actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out); }}, "Install BlackHole 2ch")), out,
        d.brew ? h("details", null, h("summary", {class: "small"}, "Prefer Homebrew?"), copyable(d.brew_cmd),
          actionRow(h("button", {class: "btn sm", type: "button", onclick: refresh}, "Check again"))) : null,
        d.tap_supported ? useTapInstead(out) : null);
      if (s.job) runJob(out);
      return;
    }
    if (!d.on) {
      append(el, h("p", {class: "small dim"}, s.detail + "."),
        actionRow(h("button", {class: "btn primary", type: "button", onclick: e => { e.target.disabled = true; runJob(out, {body: {carrier: "blackhole"}}); }}, "Turn on the wireless mic")), out,
        d.tap_supported ? useTapInstead(out) : null);
      if (s.job) runJob(out);
      return;
    }
    append(el, note("ok", "The wireless mic is on", s.detail), rankSteps(d, true), d.tap_supported ? useTapInstead(out) : null);
  },

  stick(el, s) {
    const d = s.data || {};
    const labels = {firmware: "Firmware up to date", paired: "Paired with this Mac", picked: "Picked in the agent", named: "Named", repo: "Your repo", tested: "Tried it (hold B)"};
    el.appendChild(h("div", {class: "list"}, Object.entries(labels).filter(([k]) => k in (d.steps || {})).map(([k, label]) => h("div", {class: "item"},
      h("div", {class: "what"}, h("b", null, label)),
      h("span", {class: "pill " + (d.steps[k] ? "ok" : "")}, h("span", {class: "dot"}), d.steps[k] ? "Done" : "Not yet")))));
    append(el, 
      h("p", {class: "small dim"}, s.state === "done" ? `Using ${d.stick ? d.stick.name : "your stick"}${d.connected ? " — connected" : ""}.` :
        "The stick setup walks you through Bluetooth pairing, picking it, naming it, the firmware (USB or Bluetooth) and a first try."),
      actionRow(h("button", {class: "btn" + (s.state === "done" ? "" : " primary"), type: "button", onclick: () => wizard.open(() => refresh())},
        s.state === "done" ? "Open the stick setup" : "Set up your stick")),
      h("p", {class: "small faint"}, "No stick to hand yet? Skip for now — the menu bar's Finish setup… brings you back."));
  },

  done(el) {
    const steps = list.steps.filter(x => x.id !== "done");
    el.appendChild(h("div", {class: "list"}, steps.map(x => {
      const [pc, pl] = PILL[x.state] || ["", x.state];
      return h("div", {class: "item"},
        h("div", {class: "what"}, h("b", null, x.title), h("span", {class: "sub"}, x.detail)),
        h("span", {class: "pill " + pc}, h("span", {class: "dot"}), pl),
        x.state === "todo" || x.state === "skipped" ? h("button", {class: "btn sm", type: "button", onclick: () => go(x.id)}, x.state === "skipped" ? "Do it now" : "Open") : null);
    })));
    append(el, 
      h("p", {class: "small dim"}, "Change any of this later in ", h("a", {href: "#settings"}, "Settings → Components"),
        ", or come back here with ", h("code", null, "stickybot install-page"), " or the menu bar's Finish setup…."),
      h("p", {class: "small dim"}, "More: ", h("a", {href: DOCS, target: "_blank", rel: "noopener"}, "what the stick can do"), " · ",
        h("a", {href: TEAM_DOCS, target: "_blank", rel: "noopener"}, "setting up a teammate")),
      actionRow(h("a", {class: "btn primary", href: "#overview", onclick: () => remember("installDismissed", true)}, "Go to the dashboard")));
  },
};
