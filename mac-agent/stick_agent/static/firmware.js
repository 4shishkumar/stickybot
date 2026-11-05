// Firmware panel: the firmware on this Mac (a downloaded release, or a local build)
// vs the stick's, sticks on USB, "Update via USB" with live progress, and any other
// registered updater (e.g. over Bluetooth). Used by the Stick page and the setup wizard.
import {h, clear, get, post, toast} from "./lib.js";
import {streamJob} from "./updates.js";

function sourceLabel(b) {
  if (b.source === "release") return `release ${b.release || b.version}`;
  return b.source === "local" ? "local build" : "bundled";
}

function builtDate(iso) {
  const d = new Date(iso);
  return isNaN(d) ? iso : d.toLocaleDateString([], {day: "numeric", month: "short", year: "numeric"});
}

// firmwarePanel({onDone(ok), backupDefault}) -> element (refreshes itself)
export function firmwarePanel(opts = {}) {
  const box = h("div", {style: "display:flex;flex-direction:column;gap:14px"});
  let fw = null, job = null, es = null, updaters = [];

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
    if (newer) box.appendChild(h("div", {class: "notice warn"}, h("div", {class: "grow"}, h("b", null, `Version ${bundled.version} is available`), h("span", {class: "dim"}, updaters.some(u => u.available) ? "Update it below — over Bluetooth, or with a USB-C cable." : "Plug the stick in with a USB-C cable to update it."))));
    else if (bundled && onStick) box.appendChild(h("div", {class: "notice ok"}, h("div", {class: "grow"}, h("b", null, "Your stick is up to date"))));

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
    const bar = h("div", {class: "progress"}, h("i"));
    const stage = h("div", {class: "small dim"}, "Starting…");
    const log = h("pre", {class: "log"});
    job = {id: r.job, el: h("div", {style: "display:flex;flex-direction:column;gap:8px"},
      h("div", {class: "notice warn"}, h("div", {class: "grow"}, h("b", null, "Updating — don't unplug the stick"), h("span", {class: "dim"}, "It restarts on its own when it's done (about a minute)."))),
      stage, bar, log), done: false};
    paint();
    es = new EventSource(`/api/firmware/jobs/${encodeURIComponent(r.job)}`);
    es.onmessage = e => {
      let m;
      try { m = JSON.parse(e.data); } catch (err) { return; }
      if (m.line !== undefined) { log.textContent += m.line + "\n"; log.scrollTop = log.scrollHeight; }
      if (m.progress !== undefined) { bar.firstChild.style.width = Math.max(0, Math.min(100, m.progress)) + "%"; }
      if (m.stage) stage.textContent = m.stage;
      if (m.done) {
        es.close();
        job.done = true;
        bar.className = "progress " + (m.ok ? "done" : "err");
        bar.firstChild.style.width = "100%";
        stage.textContent = m.ok ? "Done. The stick is restarting." : `Update failed: ${m.error || "see the log"}`;
        toast(m.ok ? "Firmware updated" : "Firmware update failed", m.ok ? null : "err");
        if (opts.onDone) opts.onDone(!!m.ok);
        setTimeout(() => { job = null; refresh(); }, m.ok ? 8000 : 15000);
      }
    };
    es.onerror = () => { stage.textContent = "Lost the connection to the agent. The update may still be running."; };
  }

  refresh();
  return box;
}
