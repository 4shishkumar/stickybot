// Updates (updates.py / selfupdate.py): the banner at the top of every page when a newer
// release or newer stick firmware is out, and the Settings → Updates card (channel,
// auto-check, Check now). "Update" updates the agent in one click for install.sh
// installs (a developer checkout gets the commands instead); stick firmware goes on
// from Stick → Firmware (USB, or Bluetooth once that updater is registered).
import {h, clear, get, post, toast, field, toggle, store, recall, remember, fmtAgo} from "./lib.js";

// Runs an update job and streams it into `into`: {line} / {progress, stage} / {done, ok, error}.
export function streamJob(jobsBase, job, into, onDone) {
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
    if (tries > 60) { stage.textContent = "The agent hasn't come back yet — check `stick-agent status`."; return; }
    setTimeout(poll, 1000);
  };
  setTimeout(poll, 2000);
}

function versionsLine(u) {
  const bits = [];
  const lat = u.latest || {};
  if (u.agent_update) bits.push(`stick-agent ${u.current.agent} → ${lat.version}`);
  if (u.firmware_update) bits.push(`stick firmware ${u.current.firmware} → ${lat.firmware}`);
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

// ---------- the banner ----------
export function mountBanner(el) {
  let shownKey = null, busy = false;
  const paint = async () => {
    const s = (store.status || {}).update || {};
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
  let u = null;

  const paint = () => {
    clear(info);
    if (!u) { info.appendChild(h("span", {class: "dim"}, "Checking…")); return; }
    if (u.error && u.missing) { info.appendChild(h("span", {class: "dim"}, "This version of the agent can't check for updates.")); return; }
    const lat = u.latest;
    info.appendChild(h("div", {class: "kv"},
      h("span", null, "This agent"), h("span", {class: "num"}, `${u.current.agent}${u.install ? " · " + installLabel(u.install) : ""}`),
      h("span", null, "Your stick"), h("span", {class: "num"}, u.current.firmware ? `firmware ${u.current.firmware}` : "not connected"),
      h("span", null, `Latest (${u.channel})`), h("span", {class: "num"}, lat ? `${lat.tag}${lat.firmware ? " · firmware " + lat.firmware : ""}` : (u.note || "—")),
      h("span", null, "Last checked"), h("span", null, u.checking ? "checking now…" : u.checked_at ? fmtAgo(u.checked_at) : "not yet")));
    if (u.error) info.appendChild(h("div", {class: "notice err"}, h("div", {class: "grow"}, h("b", null, "Couldn't check"), h("span", {class: "dim"}, u.error))));
    if (u.available) {
      const btn = h("button", {class: "btn primary sm", type: "button", onclick: () => runUpdate(u, out, btn)}, u.agent_update ? `Update to ${lat.tag}` : "Update the stick");
      const canClick = !u.agent_update || (u.install && u.install.can_update);
      info.appendChild(h("div", {class: "notice ok"}, h("div", {class: "grow"}, h("b", null, u.title), h("span", {class: "dim"}, versionsLine(u))), canClick ? btn : null));
    } else if (lat && !u.error) {
      info.appendChild(h("div", {class: "dim"}, "You're up to date."));
    }
    if (u.install && !u.install.can_update && u.agent_update) info.appendChild(instructions(u.install));
    const notes = notesBlock(lat, false);
    if (notes) info.appendChild(notes);
  };

  async function refresh() { u = await get("/api/update"); paint(); }

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
    h("div", {class: "row"}, checkBtn, h("span", {class: "small dim"}, "Releases come from GitHub; firmware is checked against its sha256 before it's used.")),
    info, out);
  refresh();
  return box;
}

function installLabel(i) {
  if (i.kind === "release") return i.can_update ? "updates in one click" : `can't update here: ${i.why}`;
  if (i.kind === "dev") return "developer checkout";
  return "installed as a package";
}
