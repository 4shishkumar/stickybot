// Firmware panel: the firmware on this Mac (a downloaded release, or a local build)
// vs the stick's, "Update over Bluetooth" (stick 1.8.0+, connected; preferred), sticks on
// USB and "Update via USB" (always works, also for recovery), any other registered updater,
// with live progress. Used by the Stick page and the setup wizard.
import {h, clear, get, post, toast} from "./lib.js";
import {streamJob} from "./updates.js";

function sourceLabel(b) {
  if (b.source === "release") return `release ${b.release || b.version}`;
  return b.source === "local" ? "local build" : "bundled";
}

// "1.8.1-test" -> [1, 8, 1] for comparing
function vparts(v) { return (String(v || "").match(/\d+/g) || []).slice(0, 3).map(Number); }
function vcmp(a, b) {
  const x = vparts(a), y = vparts(b);
  for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) < (y[i] || 0) ? -1 : 1;
  return 0;
}

function builtDate(iso) {
  const d = new Date(iso);
  return isNaN(d) ? iso : d.toLocaleDateString([], {day: "numeric", month: "short", year: "numeric"});
}

// firmwarePanel({onDone(ok), backupDefault}) -> element (refreshes itself)
export function firmwarePanel(opts = {}) {
  const box = h("div", {style: "display:flex;flex-direction:column;gap:14px"});
  let fw = null, job = null, es = null, updaters = [], asking = false;

  const paint = () => {
    clear(box);
    if (!fw) { box.appendChild(h("div", {class: "empty"}, "Checking firmware…")); return; }
    if (fw.error) {
      box.appendChild(h("div", {class: "notice"}, h("div", {class: "grow"},
        h("b", null, fw.missing ? "Firmware updates aren't available yet" : "Can't check the firmware"),
        h("span", {class: "dim"}, fw.missing ? "This version of the agent can't flash the stick yet. You can still flash it from the repo: make -C m5-stick-s3 flash." : fw.error))));
      return;
    }
    const bundled = fw.bundled;
    const onStick = fw.connected && fw.connected.version;
    const newer = bundled && onStick && bundled.version !== onStick;
    box.appendChild(h("div", {class: "kv"},
      h("span", null, "Ready on this Mac"), h("span", {class: "num"}, bundled ? `${bundled.version} · ${sourceLabel(bundled)}${bundled.built ? " · built " + builtDate(bundled.built) : ""}` : "none yet"),
      h("span", null, "On your stick"), h("span", {class: "num"}, onStick ? `${onStick}${fw.connected.name ? " · " + fw.connected.name : ""}` : "unknown (stick not connected)")));
    if (!bundled) {
      const check = h("button", {class: "btn sm", type: "button", onclick: async () => {
        check.disabled = true;
        const r = await post("/api/update/check");
        if (r.error) toast(r.error, "err"); else if (r.note) toast(r.note);
        refresh();
      }}, "Check for updates");
      box.appendChild(h("div", {class: "notice"}, h("div", {class: "grow"},
        h("b", null, "No firmware on this Mac yet"),
        h("span", {class: "dim"}, "The agent downloads the latest release's firmware when it checks for updates. Before the first release, build it from the repo: make -C m5-stick-s3 release.")),
        check));
    }
    const bleOk = !!(fw.connected && fw.connected.ota);
    if (newer) box.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"}, h("b", null, `Version ${bundled.version} is available`),
      h("span", {class: "dim"}, bleOk ? "Update it over Bluetooth below, or plug it in with a USB-C cable." : "Plug the stick in with a USB-C cable to update it."))));
    else if (bundled && onStick) box.appendChild(h("div", {class: "notice ok"}, h("div", {class: "grow"}, h("b", null, "Your stick is up to date"))));

    // Over Bluetooth: firmware 1.8.0+ on a connected stick
    if (bundled && onStick) {
      const cmp = vcmp(bundled.version, onStick);
      if (bleOk) {
        if (asking) {
          const force = cmp <= 0;
          box.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"},
            h("b", null, `Update ${fw.connected.name || "the stick"} from ${onStick} to ${bundled.version} over Bluetooth?`),
            h("span", {class: "dim"}, (force ? (cmp < 0 ? "That's an older version than the stick's. " : "The stick already runs this version. ") : "") +
              "It takes about 1–2 minutes; keep the stick near this Mac. It restarts on its own and keeps its settings and pairings. If the new firmware doesn't start, it goes back to the old one."),
            h("div", {class: "row", style: "margin-top:8px;gap:8px"},
              h("button", {class: "btn primary", onclick: () => { asking = false; ota(force); }}, force ? "Install anyway" : "Update"),
              h("button", {class: "btn ghost", onclick: () => { asking = false; paint(); }}, "Not now")))));
        } else {
          box.appendChild(h("div", {class: "list"}, h("div", {class: "item"},
            h("div", {class: "what"}, h("b", null, "Over Bluetooth"), h("span", {class: "sub"}, `${fw.connected.name || "stick"} · firmware ${onStick} · no cable needed`)),
            h("button", {class: "btn " + (cmp > 0 ? "primary" : "ghost"), disabled: !!job, onclick: () => { asking = true; paint(); }},
              job ? "Updating…" : cmp > 0 ? "Update over Bluetooth" : "Reinstall over Bluetooth"))));
        }
      } else if (newer) {
        box.appendChild(h("div", {class: "small dim"}, `Firmware ${onStick} can only be updated over USB. From 1.8.0 on, updates can also go over Bluetooth.`));
      }
    }

    const usb = fw.usb || [];
    if (!usb.length) {
      box.appendChild(h("div", {class: "notice"}, h("div", {class: "grow"},
        h("b", null, "No stick on USB"), h("span", {class: "dim"}, "Plug the stick into this Mac with a USB-C data cable (some cables only charge)."))));
    }
    const backup = h("input", {type: "checkbox", id: "fwBackup", checked: !!opts.backupDefault});
    for (const u of usb) {
      box.appendChild(h("div", {class: "list"}, h("div", {class: "item"},
        h("div", {class: "what"}, h("b", null, u.board ? u.board : "M5StickS3"), h("span", {class: "sub mono"}, `${u.port}${u.version ? " · firmware " + u.version : ""}`)),
        h("button", {class: "btn primary", disabled: !bundled || !!job, onclick: () => flash(u.port, backup.checked)}, job ? "Updating…" : "Update via USB"))));
    }
    if (usb.length) box.appendChild(h("label", {class: "switch"}, backup, h("span", {class: "small"}, "Back up the stick's current firmware first (recommended for a brand-new stick)")));
    for (const u of updaters) {  // other ways (updates.register_firmware_updater), e.g. over Bluetooth
      box.appendChild(h("div", {class: "list"}, h("div", {class: "item"},
        h("div", {class: "what"}, h("b", null, u.title), u.why ? h("span", {class: "sub"}, u.why) : null),
        h("button", {class: "btn", disabled: !bundled || !u.available || !!job, onclick: () => runUpdater(u)}, u.title))));
    }
    if (job) box.appendChild(job.el);
    box.appendChild(h("div", {class: "row"}, h("button", {class: "btn sm ghost", onclick: refresh, disabled: !!(job && !job.done)}, "Check again")));
  };

  async function refresh() {
    const [f, up] = await Promise.all([get("/api/firmware"), get("/api/update")]);
    fw = f;
    updaters = up && !up.error && Array.isArray(up.firmware_updaters) ? up.firmware_updaters : [];
    paint();
  }

  async function runUpdater(u) {
    const r = await post(`/api/update/firmware/${encodeURIComponent(u.id)}`);
    if (r.error) { toast(r.error, "err"); return; }
    const el = h("div", {style: "display:flex;flex-direction:column;gap:8px"});
    job = {id: r.job, el, done: false};
    paint();
    streamJob("/api/update/jobs", r.job, el, ok => {
      job.done = true;
      toast(ok ? "Firmware updated" : "Firmware update failed", ok ? null : "err");
      if (opts.onDone) opts.onDone(ok);
      setTimeout(() => { job = null; refresh(); }, ok ? 8000 : 15000);
    });
  }

  async function flash(port, doBackup) {
    const r = await post("/api/firmware/flash", {port, backup: doBackup});
    if (r.error) { toast(r.error, "err"); return; }
    follow(r.job, "Updating — don't unplug the stick", "It restarts on its own when it's done (about a minute).", false);
  }

  async function ota(force) {
    const r = await post("/api/firmware/ota", {force});
    if (r.error) { toast(r.error, "err"); paint(); return; }
    follow(r.job, "Updating over Bluetooth — keep the stick near the Mac",
      "It shows the progress on its screen (B there cancels) and restarts on its own when it's done.", true);
  }

  const STAGES = {reset: "Resetting into download mode", backup: "Backing up", flash: "Writing", verify: "Verifying",
    reboot: "Restarting", send: "Sending over Bluetooth"};

  function follow(id, title, sub, cancellable) {
    const bar = h("div", {class: "progress"}, h("i"));
    const stage = h("div", {class: "small dim"}, "Starting…");
    const log = h("pre", {class: "log"});
    const stop = cancellable ? h("button", {class: "btn sm ghost", onclick: async () => {
      stop.disabled = true;
      const c = await post(`/api/firmware/jobs/${encodeURIComponent(id)}/cancel`, {});
      if (c.error) { toast(c.error, "err"); stop.disabled = false; }
    }}, "Cancel") : null;
    job = {id, el: h("div", {style: "display:flex;flex-direction:column;gap:8px"},
      h("div", {class: "notice warn"}, h("div", {class: "grow"}, h("b", null, title), h("span", {class: "dim"}, sub))),
      stage, bar, log, stop ? h("div", {class: "row"}, stop) : null), done: false};
    paint();
    es = new EventSource(`/api/firmware/jobs/${encodeURIComponent(id)}`);
    es.onmessage = e => {
      let m;
      try { m = JSON.parse(e.data); } catch (err) { return; }
      if (m.line !== undefined) { log.textContent += m.line + "\n"; log.scrollTop = log.scrollHeight; }
      if (m.progress !== undefined) { bar.firstChild.style.width = Math.max(0, Math.min(100, m.progress)) + "%"; }
      if (m.stage) stage.textContent = (STAGES[m.stage] || m.stage) + (m.progress !== undefined && m.progress < 100 ? ` · ${m.progress}%` : "");
      if (m.done) {
        es.close();
        job.done = true;
        if (stop) stop.remove();
        bar.className = "progress " + (m.ok ? "done" : "err");
        bar.firstChild.style.width = "100%";
        const cancelled = !m.ok && /^cancelled/.test(m.error || "");
        const now = m.result && m.result.version;
        stage.textContent = m.ok ? (now ? `Done. The stick runs ${now}.` : "Done. The stick is restarting.")
          : cancelled ? `Update ${m.error}. The stick keeps its firmware.` : `Update failed: ${m.error || "see the log"}`;
        toast(m.ok ? "Firmware updated" : cancelled ? "Update cancelled" : "Firmware update failed", m.ok || cancelled ? null : "err");
        if (opts.onDone) opts.onDone(!!m.ok);
        setTimeout(() => { job = null; refresh(); }, m.ok ? 8000 : 15000);
      }
    };
    es.onerror = () => { stage.textContent = "Lost the connection to the agent. The update may still be running."; };
  }

  refresh();
  return box;
}
