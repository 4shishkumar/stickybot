// Logs: the agent's log file (~/.stick-agent/agent.log), newest at the bottom.
import {h, get, report, store} from "../lib.js";

export const title = "Logs";
export const subtitle = "The agent's log, from ~/.stick-agent/agent.log. Also: stickybot logs in a terminal.";

let pre = null, off = null, follow = true;

export function actions() {
  return [
    h("label", {class: "switch"}, h("input", {type: "checkbox", id: "logFollow", checked: follow, onchange: e => { follow = e.target.checked; }}), h("span", {class: "small dim"}, "Follow live")),
    h("button", {class: "btn", type: "button", onclick: load}, "Reload"),
  ];
}

export function render(root) {
  pre = h("pre", {class: "log tall", tabindex: "0", "aria-label": "Agent log"}, "Loading…");
  root.appendChild(pre);
  load();
  off = store.on((type, ev) => {
    if (type === "event" && ev.kind === "log" && follow && pre) {
      pre.textContent += "\n" + ev.text;
      pre.scrollTop = pre.scrollHeight;
    }
  });
}

export function leave() { if (off) off(); pre = null; }

async function load() {
  const lines = await get("/api/logs?n=500");
  if (!pre) return;
  if (!Array.isArray(lines)) { report(lines); pre.textContent = lines.error || ""; return; }
  pre.textContent = lines.join("\n") || "The log is empty.";
  pre.scrollTop = pre.scrollHeight;
}
