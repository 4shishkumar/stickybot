// The installer's components (see installer.py): what's installed, with Install /
// Remove for the ones that can be changed from here. Terminal-only ones show the
// command to run instead.
import {h, clear, get, post, toast} from "./lib.js";

const STATE = {installed: ["ok", "Installed"], partial: ["warn", "Needs attention"], missing: ["", "Not installed"], "n/a": ["", ""]};

export function componentsPanel() {
  const box = h("div", {style: "display:flex;flex-direction:column;gap:10px"});
  let list = null, running = null;

  function needsText(n) {
    const bits = [];
    if (n.download) bits.push(`downloads ${n.download}`);
    if (n.admin) bits.push("needs your admin password");
    if (n.usb) bits.push("stick on USB");
    if (n.terminal) bits.push(n.terminal);
    if (n.permission) bits.push(`macOS permission: ${n.permission}`);
    return bits.join(" · ");
  }

  function row(c) {
    const [cls, label] = STATE[c.status] || ["", c.status];
    const actions = h("div", {class: "row", style: "margin-left:auto;flex:none"});
    if (c.from_dashboard && !running) {
      if (c.status !== "installed") actions.appendChild(h("button", {class: "btn sm primary", type: "button", onclick: () => run(c, "install")}, c.status === "partial" ? "Fix" : "Install"));
      if (c.removable && c.status !== "missing") actions.appendChild(h("button", {class: "btn sm", type: "button", onclick: () => run(c, "uninstall")}, "Remove"));
    } else if (!c.from_dashboard && c.id === "firmware") {
      actions.appendChild(h("a", {class: "btn sm", href: "#stick"}, "Stick → Firmware"));
    } else if (!c.from_dashboard && c.id === "core") {
      actions.appendChild(h("span", {class: "small dim"}, "this is what's running"));
    }
    return h("div", {class: "row", style: "align-items:flex-start;gap:12px;padding:10px 0;border-top:1px solid var(--line)"},
      h("div", {class: "grow"},
        h("div", {class: "row", style: "gap:8px"}, h("b", null, c.title), label ? h("span", {class: "pill " + cls}, h("span", {class: "dot"}), label) : null),
        h("div", {class: "small dim"}, c.description),
        c.detail ? h("div", {class: "small"}, c.detail) : null,
        needsText(c.needs || {}) ? h("div", {class: "small dim"}, needsText(c.needs)) : null),
      actions);
  }

  function paint() {
    clear(box);
    if (!list) { box.appendChild(h("div", {class: "empty"}, "Checking…")); return; }
    if (list.error) {
      box.appendChild(h("div", {class: "notice"}, list.missing ? "This version of the agent doesn't list components yet." : list.error));
      return;
    }
    box.appendChild(h("p", {class: "lead"}, "The parts of the companion app. The same list as ", h("code", null, "stick-agent setup"), " in a terminal (which can also do the ones that need a password or a login)."));
    box.appendChild(h("div", {class: "row"}, h("a", {class: "btn sm", href: "#install"}, "Open the setup page"),
      h("span", {class: "small dim"}, "step by step, with the Claude / GitHub logins and the wireless mic — and any step you skipped")));
    for (const c of list) box.appendChild(row(c));
    if (running) box.appendChild(running.el);
  }

  async function refresh() { list = await get("/api/components"); paint(); }

  async function run(c, action) {
    const r = await post(`/api/components/${encodeURIComponent(c.id)}/${action}`);
    if (r.error) { toast(r.error, "err"); return; }
    const bar = h("div", {class: "progress"}, h("i"));
    const log = h("pre", {class: "log"});
    const stage = h("div", {class: "small dim"}, `${action === "install" ? "Installing" : "Removing"} ${c.title}…`);
    running = {el: h("div", {style: "display:flex;flex-direction:column;gap:8px"}, stage, bar, log)};
    paint();
    const es = new EventSource(`/api/components/jobs/${encodeURIComponent(r.job)}`);
    es.onmessage = e => {
      let m;
      try { m = JSON.parse(e.data); } catch (err) { return; }
      if (m.line !== undefined) { log.textContent += m.line + "\n"; log.scrollTop = log.scrollHeight; }
      if (m.progress !== undefined) bar.firstChild.style.width = Math.max(0, Math.min(100, m.progress)) + "%";
      if (m.done) {
        es.close();
        bar.className = "progress " + (m.ok ? "done" : "err");
        bar.firstChild.style.width = "100%";
        stage.textContent = m.ok ? "Done." : `Didn't work: ${m.error || "see above"}`;
        toast(m.ok ? `${c.title}: done` : `${c.title}: ${m.error || "failed"}`, m.ok ? null : "err");
        setTimeout(() => { running = null; refresh(); }, m.ok ? 2500 : 8000);
      }
    };
    es.onerror = () => { stage.textContent = "Lost the connection to the agent; the job may still be running."; };
  }

  refresh();
  paint();
  return box;
}
