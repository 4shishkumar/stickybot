// Updates (updates.py / selfupdate.py): the banner at the top of every page when a newer
// release or newer stick firmware is out, and the Settings → Updates card (channel,
// auto-check, Check now). "Update" updates the agent in one click for install.sh
// installs (a developer checkout gets the commands instead); stick firmware goes on
// from Stick → Firmware (USB, or Bluetooth once that updater is registered).
//
// The Rust app installs its own updates ("Install and restart", a progress bar, waiting while the
// stick is busy, errors with Try again): its /api/update has `state`, and then the banner and the
// card draw that flow (update_ui.js decides what to show). Without `state` (the Python app, an
// older Rust app) everything below the "install flow" section runs as it always did.
import {h, clear, get, post, toast, field, toggle, store, recall, remember, fmtAgo, refreshStatus} from "./lib.js";
import {view, appFlow, hasInstaller, updatedNotice} from "./update_ui.js";

// Runs an update job and streams it into `into`: {line} / {progress, stage} / {done, ok, error}.
// onEvent(m) also sees every event (e.g. the setup page's {code, url} for the GitHub login).
export function streamJob(jobsBase, job, into, onDone, onEvent) {
  const bar = h("div", {class: "progress"}, h("i"));
  const stage = h("div", {class: "small dim"}, "Starting…");
  const log = h("pre", {class: "log"});
  clear(into).append(stage, bar, log);
  const es = new EventSource(`${jobsBase}/${encodeURIComponent(job)}`);
  es.onmessage = e => {
    let m;
    try { m = JSON.parse(e.data); } catch (err) { return; }
    if (m.line !== undefined) { log.textContent += m.line + "\n"; log.scrollTop = log.scrollHeight; }
    if (m.progress !== undefined) bar.firstChild.style.width = Math.max(0, Math.min(100, m.progress)) + "%";
    if (m.stage) stage.textContent = m.stage;
    if (onEvent) onEvent(m);
    if (m.done) {
      es.close();
      bar.className = "progress " + (m.ok ? "done" : "err");
      bar.firstChild.style.width = "100%";
      stage.textContent = m.ok ? "Done." : `Didn't work: ${m.error || "see the log"}`;
      if (onDone) onDone(!!m.ok, m.error, stage);
    }
  };
  es.onerror = () => { if (es.readyState === EventSource.CLOSED) return; stage.textContent = "Lost the connection to the agent (it may be restarting)…"; };
  return es;
}

// After the agent restarts itself: wait until it answers again, then reload the page.
function reloadWhenBack(stage) {
  let tries = 0;
  const poll = async () => {
    tries++;
    const s = await get("/api/status");
    if (!s.error && tries > 2) { location.reload(); return; }
    if (tries > 60) { stage.textContent = "The agent hasn't come back yet — check `stickybot status`."; return; }
    setTimeout(poll, 1000);
  };
  setTimeout(poll, 2000);
}

function versionsLine(u) {
  const bits = [];
  const lat = u.latest || {};
  if (u.agent_update) bits.push(`Helper app v${u.current.agent} → v${lat.version}`);
  if (u.firmware_update) bits.push(`Stick firmware ${u.current.firmware} → ${lat.firmware}`);
  return bits.join(" · ");
}

function notesBlock(lat, open) {
  if (!lat || !lat.notes) return null;
  return h("details", {open},
    h("summary", {class: "small"}, `What's new in ${lat.tag}`),
    h("pre", {class: "log", style: "max-height:220px;margin-top:6px"}, lat.notes),
    lat.url ? h("a", {class: "small", href: lat.url, target: "_blank", rel: "noopener"}, "On GitHub") : null);
}

function instructions(install) {
  return h("div", {class: "small", style: "display:flex;flex-direction:column;gap:4px"},
    h("span", {class: "dim"}, `This agent runs from ${install.path} (${install.why}). To update it yourself:`),
    h("pre", {class: "log"}, (install.instructions || []).join("\n")));
}

// The Update button: the agent first (it restarts), then the firmware from Stick → Firmware.
async function runUpdate(u, out, btn) {
  if (!u.agent_update) { location.hash = "#stick"; return; }
  if (!u.install || !u.install.can_update) { clear(out).appendChild(instructions(u.install || {why: "unknown", path: "?"})); return; }
  btn.disabled = true;
  const r = await post("/api/update/agent", {tag: u.latest.tag});
  if (r.error) {
    btn.disabled = false;
    if (r.install) clear(out).appendChild(instructions(r.install)); else toast(r.error, "err");
    return;
  }
  streamJob("/api/update/jobs", r.job, out, (ok, err, stage) => {
    if (ok) { stage.textContent = "Updated — restarting the agent…"; reloadWhenBack(stage); }
    else { btn.disabled = false; toast(`Update failed: ${err || "see the log"}`, "err"); }
  });
}

// ---------- the install flow (the Rust app) ----------
// What the page keeps of /api/update between events: the full answer once, then the live fields.
const LIVE = ["state", "progress", "waiting", "install_error", "updated", "available", "title", "agent_update",
  "firmware_update", "error", "note", "auto"];
function merge(u, p) {
  const out = {...u};
  for (const k of LIVE) if (p[k] !== undefined) out[k] = p[k];
  if (p.install) out.install = {...(u.install || {}), ...p.install};
  return out;
}

function ensureBarStyle() {
  if (document.getElementById("updBar")) return;
  document.head.appendChild(h("style", {id: "updBar"},
    ".progress.indeterminate > i { width: 40% !important; animation: updBar 1.2s ease-in-out infinite alternate; }" +
    "@keyframes updBar { from { margin-left: 0 } to { margin-left: 60% } }"));
}

// The Rust app polls until the new version answers, then reloads (once, however often we repaint).
let reloading = false;

// One panel for the banner and the Settings card: what update_ui.view(u) says, drawn. `host`:
// {refresh() reloads /api/update, later() hides the offer (banner only), instructions}.
function installPanel(u, host) {
  const v = view(u);
  if (v.mode === "none" || v.mode === "legacy") return null;
  const lat = u.latest || {};
  const title = h("b", null, u.title || `Update available: ${lat.tag || ""}`);
  const call = async path => {
    const r = await post(path);
    if (r.error) toast(r.error, "err");
    host.refresh();
  };
  const row = (...kids) => h("div", {class: "row", style: "flex:none"}, ...kids);
  const pageLink = href => href ? h("a", {class: "btn", href, target: "_blank", rel: "noopener"}, "Open the download page") : null;
  if (v.mode === "offer" || v.mode === "fallback") {
    const notes = notesBlock(lat, false);
    const install = v.mode === "offer";
    return h("div", {class: "notice ok", role: "status"},
      h("div", {class: "grow", style: "gap:6px"}, title, h("span", {class: "dim"}, versionsLine(u)),
        install ? null : h("span", {class: "small dim"}, `This StickyBot can't install it itself: ${v.why}`),
        notes,
        install || !host.instructions || !(u.install && u.install.instructions) ? null : instructions(u.install)),
      row(install
        ? h("button", {class: "btn primary", type: "button", onclick: () => call("/api/update/install")}, "Install and restart")
        : pageLink(v.download),
        host.later ? h("button", {class: "btn ghost", type: "button", title: "Hide this until the next release", onclick: host.later}, "Later") : null));
  }
  if (v.mode === "error") {
    return h("div", {class: "notice err", role: "alert"},
      h("div", {class: "grow", style: "gap:6px"}, h("b", null, v.heading), h("span", {class: "dim"}, v.message)),
      row(v.canRetry ? h("button", {class: "btn primary", type: "button", onclick: () => call("/api/update/install")}, "Try again") : null,
        pageLink(v.download),
        h("button", {class: "btn ghost", type: "button", title: "Hide this message", onclick: () => call("/api/update/cancel")}, "Dismiss")));
  }
  // downloading / waiting / installing / restarting: a line of words, and a bar
  ensureBarStyle();
  const stage = h("div", {class: "small dim"}, v.text);
  const indeterminate = v.mode !== "downloading" || v.percent === null;
  const bar = h("div", {class: "progress" + (indeterminate ? " indeterminate" : ""), role: "progressbar",
    "aria-valuenow": indeterminate ? null : v.percent, "aria-valuemin": 0, "aria-valuemax": 100},
    h("i", {style: `width:${indeterminate ? 100 : v.percent}%`}));
  if (v.mode === "restarting" && !reloading) { reloading = true; reloadWhenBack(stage); }
  return h("div", {class: "notice ok", role: "status"},
    h("div", {class: "grow", style: "gap:6px"}, title, stage, v.mode === "waiting" ? null : bar),
    v.mode === "waiting" || v.mode === "downloading"
      ? row(h("button", {class: "btn", type: "button", onclick: () => call("/api/update/cancel")}, "Cancel")) : null);
}

// "Updated to vX", once (dismissed in this browser by its time stamp).
function updatedPanel(n, dismiss) {
  return h("div", {class: "notice ok", role: "status"},
    h("div", {class: "grow", style: "gap:6px"}, h("b", null, n.text), h("span", {class: "dim"}, "StickyBot restarted on the new version."),
      n.notes ? h("a", {class: "small", href: n.notes, target: "_blank", rel: "noopener"}, "What's new") : null),
    h("div", {class: "row", style: "flex:none"}, h("button", {class: "btn ghost", type: "button", onclick: dismiss}, "Dismiss")));
}

// ---------- the banner ----------
export function mountBanner(el) {
  let shownKey = null, busy = false;
  let app = null, fullKey = null;   // the Rust app's /api/update, and the title it was fetched for
  let drawn = null;   // what is on screen (the view and the notice): an unchanged one isn't rebuilt
  const drawNew = () => {
    const v = view(app), n = updatedNotice(app, recall("updatedSeen", null));
    const offer = ["offer", "fallback"].includes(v.mode);
    const hidden = offer && recall("updateLater", null) === app.title;
    const sig = JSON.stringify([v, hidden, n, app.title, app.firmware_update]);
    if (sig === drawn) return;
    drawn = sig;
    clear(el);
    const later = () => { remember("updateLater", app.title); drawNew(); };
    if (!hidden) {
      const p = installPanel(app, {refresh: refreshStatus, later: offer ? later : null});
      if (p) el.appendChild(p);
    }
    if (n) el.appendChild(updatedPanel(n, () => { remember("updatedSeen", n.at); drawNew(); }));
  };
  // -> true when the new flow drew (or has nothing to draw), false when the old banner should
  const paintNew = async s => {
    if (!app || s.title !== fullKey) {
      const f = await get("/api/update");
      if (f.error || !hasInstaller(f)) { app = null; fullKey = null; drawn = null; return false; }
      app = f; fullKey = s.title;
    } else {
      app = merge(app, s);
    }
    drawNew();
    return true;
  };
  // the hub's update events carry the live fields (progress comes at most 5 a second): no refetch for those
  store.on((type, ev) => {
    if (type !== "event" || ev.kind !== "update" || !hasInstaller(ev)) return;
    if (!app || ev.title !== fullKey) { refreshStatus(); return; }   // a new release, or nothing loaded yet
    const was = app.state;
    app = merge(app, ev);
    if (["idle", "available"].includes(app.state) && ["downloading", "installing", "error"].includes(was)) { refreshStatus(); return; }
    drawNew();
  });
  const paint = async () => {
    const s = (store.status || {}).update || {};
    if (hasInstaller(s) && appFlow(s)) { if (await paintNew(s)) return; }
    else if (app) { app = null; fullKey = null; drawn = null; clear(el); shownKey = null; }   // nothing about the app now
    if (!s.available) { if (!busy) { clear(el); shownKey = null; } return; }
    const key = s.title;
    if (key === shownKey || busy) return;
    if (recall("updateLater", null) === key) { clear(el); shownKey = key; return; }
    shownKey = key;
    const u = await get("/api/update");
    if (u.error || !u.available) { clear(el); return; }
    const out = h("div", {style: "display:flex;flex-direction:column;gap:8px"});
    const updateBtn = h("button", {class: "btn primary", type: "button", onclick: () => { busy = true; runUpdate(u, out, updateBtn); }},
      u.agent_update ? "Update" : "Update the stick");
    clear(el).appendChild(h("div", {class: "notice ok", role: "status"},
      h("div", {class: "grow", style: "gap:6px"},
        h("b", null, u.title),
        h("span", {class: "dim"}, versionsLine(u)),
        notesBlock(u.latest, false),
        out),
      h("div", {class: "row", style: "flex:none"}, updateBtn,
        h("button", {class: "btn ghost", type: "button", title: "Hide this until the next release", onclick: () => { remember("updateLater", key); clear(el); }}, "Later"))));
  };
  store.on(type => { if (type === "status") paint(); });
  paint();
}

// ---------- Settings → Updates ----------
export function updatesPanel(c) {
  const box = h("div", {style: "display:flex;flex-direction:column;gap:10px"});
  const info = h("div", {class: "small", style: "display:flex;flex-direction:column;gap:6px"});
  const out = h("div", {style: "display:flex;flex-direction:column;gap:8px"});
  const autoBox = h("div");
  let u = null;

  const paint = () => {
    clear(info);
    if (!u) { info.appendChild(h("span", {class: "dim"}, "Checking…")); return; }
    if (u.error && u.missing) { info.appendChild(h("span", {class: "dim"}, "This version of the agent can't check for updates.")); return; }
    const lat = u.latest;
    // Two numbers on purpose: every release has its own (StickyBot's); the stick firmware's
    // only changes when a release changes the stick, so most releases need no stick update.
    const state = (have, latest, newer) => !latest || !have ? "" : newer ? ` → ${latest} available` : " · latest";
    info.appendChild(h("div", {class: "kv"},
      h("span", null, "Helper app"), h("span", {class: "num"},
        `v${u.current.agent}${state(u.current.agent, lat && "v" + lat.version, u.agent_update)}${u.install ? " · " + installLabel(u.install) : ""}`),
      h("span", null, "Stick firmware"), h("span", {class: "num"}, u.current.firmware
        ? `${u.current.firmware}${state(u.current.firmware, lat && lat.firmware, u.firmware_update)}` : "stick not connected"),
      h("span", null, `Latest release (${u.channel})`), h("span", {class: "num"},
        lat ? `${lat.tag}${lat.firmware ? " · with stick firmware " + lat.firmware : ""}` : (u.note || "—")),
      h("span", null, "Last checked"), h("span", null, u.checking ? "checking now…" : u.checked_at ? fmtAgo(u.checked_at) : "not yet")));
    if (u.error) info.appendChild(h("div", {class: "notice err"}, h("div", {class: "grow"}, h("b", null, "Couldn't check"), h("span", {class: "dim"}, u.error))));
    const flow = appFlow(u);   // the Rust app installs this one itself (update_ui.js)
    if (flow) {
      const p = installPanel(u, {refresh, instructions: true});
      if (p) info.appendChild(p);
      if (u.firmware_update) info.appendChild(h("div", {class: "dim"}, `Stick firmware ${lat.firmware} is also available — update it from Stick → Firmware.`));
    } else if (u.available) {
      const btn = h("button", {class: "btn primary sm", type: "button", onclick: () => runUpdate(u, out, btn)}, u.agent_update ? `Update to ${lat.tag}` : "Update the stick");
      const canClick = !u.agent_update || (u.install && u.install.can_update);
      info.appendChild(h("div", {class: "notice ok"}, h("div", {class: "grow"}, h("b", null, u.title), h("span", {class: "dim"}, versionsLine(u))), canClick ? btn : null));
    } else if (lat && !u.error) {
      info.appendChild(h("div", {class: "dim"}, "You're up to date: StickyBot and the stick firmware are both the latest."));
    }
    if (!flow && u.install && !u.install.can_update && u.agent_update) info.appendChild(instructions(u.install));
    const notes = flow && ["offer", "fallback"].includes(view(u).mode) ? null : notesBlock(lat, false);
    if (notes) info.appendChild(notes);
    // "Install updates automatically": only where the app reports it (the Rust app)
    if (u.auto !== undefined && !autoBox.firstChild) {
      autoBox.appendChild(field("Install updates automatically", toggle("updAuto",
        "Install a new version by itself once nothing is in use and the stick has been idle for 10 minutes",
        c.update_auto !== undefined ? !!c.update_auto : !!u.auto),
        "Off by default. It never interrupts a talk, a permission prompt or a firmware update, and the person at the stick isn't cut off: it waits for 10 quiet minutes. The new version is checked against StickyBot's signing key first. Saved with the Save button."));
    }
  };

  async function refresh() {
    u = await get("/api/update");
    paint();
  }

  // the hub's update events keep the card current while an install runs
  let attached = false;
  const off = store.on((type, ev) => {
    if (box.isConnected) attached = true; else if (attached) { off(); return; }
    if (type !== "event" || ev.kind !== "update" || !hasInstaller(ev) || !hasInstaller(u)) return;
    const was = u.state;
    u = merge(u, ev);
    if (["idle", "available"].includes(u.state) && ["downloading", "installing", "error"].includes(was)) refresh(); else paint();
  });

  const checkBtn = h("button", {class: "btn", type: "button", onclick: async () => {
    checkBtn.disabled = true;
    u = {...(u || {}), checking: true}; paint();
    const r = await post("/api/update/check");
    checkBtn.disabled = false;
    if (r.error) { toast(r.error, "err"); refresh(); return; }
    u = r; paint();
    toast(r.available ? r.title : r.error ? "Couldn't check for updates" : "You're up to date");
  }}, "Check now");

  box.append(
    field("Channel", h("select", {id: "updChannel"}, [["stable", "Stable releases (recommended)"], ["beta", "Beta — try new things first"]]
      .map(([v, l]) => h("option", {value: v, selected: (c.update_channel || "stable") === v}, l))),
      "Beta also offers prereleases (vX.Y.Z-beta.N). Switching checks again right away."),
    field("Check automatically", toggle("updCheck", "At start-up and once a day (a small request to GitHub)", c.update_check !== false)),
    autoBox,
    h("div", {class: "row"}, checkBtn, h("span", {class: "small dim"}, "Releases come from GitHub; firmware is checked against its sha256 before it's used. The helper app and the stick firmware have separate version numbers — the firmware only changes when a release changes the stick.")),
    info, out);
  refresh();
  return box;
}

function installLabel(i) {
  if (i.kind === "release") return i.can_update ? "updates in one click" : `can't update here: ${i.why}`;
  if (i.kind === "dev") return "developer checkout";
  if (i.kind === "rust") return "Mac app";
  return "installed as a package";
}
