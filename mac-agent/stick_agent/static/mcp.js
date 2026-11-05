// The stick tools for Claude sessions (mcp_server.py): the on/off switch, the tools, and
// whether terminal Claude Code sessions have them (`stick-agent mcp install` = `claude mcp add`).
// Installing shows the exact command first and needs a second, confirming click.
import {h, clear, get, post, toast, field, toggle, fmtAgo} from "./lib.js";

export function mcpPanel(c) {
  const box = h("div", {style: "display:flex;flex-direction:column;gap:10px"});
  const status = h("div", {class: "small", style: "display:flex;flex-direction:column;gap:8px"});
  let st = null, confirming = null, busy = false;

  const pill = (cls, text) => h("span", {class: `pill ${cls}`}, h("span", {class: "dot"}), text);

  function paint() {
    clear(status);
    if (!st) { status.appendChild(h("span", {class: "dim"}, "Checking…")); return; }
    if (st.error) { status.appendChild(h("span", {class: "dim"}, st.missing ? "This version of the agent doesn't have the stick tools." : st.error)); return; }
    const t = st.terminal || {};
    const state = t.installed ? pill("ok", "Terminal sessions: added") : t.installed === false ? pill("", "Terminal sessions: not added") : pill("warn", "Terminal sessions: unknown");
    status.appendChild(h("div", null, state, " ", h("span", {class: "dim"}, t.detail || "")));
    const row = h("div", {class: "row", style: "gap:8px;flex-wrap:wrap"});
    if (!t.installed && t.installed !== null && !busy) {
      if (confirming === "install") {
        row.append(h("button", {class: "btn sm primary", type: "button", onclick: () => run("install")}, "Run it"),
          h("button", {class: "btn sm", type: "button", onclick: () => { confirming = null; paint(); }}, "Cancel"));
      } else {
        row.append(h("button", {class: "btn sm primary", type: "button", onclick: () => { confirming = "install"; paint(); }}, "Install…"));
      }
    } else if (t.installed && !busy) {
      if (confirming === "uninstall") {
        row.append(h("button", {class: "btn sm danger", type: "button", onclick: () => run("uninstall")}, "Remove them"),
          h("button", {class: "btn sm", type: "button", onclick: () => { confirming = null; paint(); }}, "Cancel"));
      } else {
        row.append(h("button", {class: "btn sm", type: "button", onclick: () => { confirming = "uninstall"; paint(); }}, "Remove…"));
      }
    }
    if (busy) row.appendChild(h("span", {class: "dim"}, "Running…"));
    if (confirming === "install") status.appendChild(h("div", null, h("div", {class: "dim"}, "This runs Claude Code's own command (it adds the server to your user settings, for every project):"),
      h("pre", {class: "log", style: "white-space:pre-wrap"}, t.command || "claude mcp add …"),
      h("div", {class: "dim"}, "<token> is the agent's secret from ~/.stick-agent/token. In a terminal: stick-agent mcp install")));
    if (confirming === "uninstall") status.appendChild(h("pre", {class: "log"}, "claude mcp remove --scope user stick"));
    status.appendChild(row);
    if (st.last_call) status.appendChild(h("div", {class: "dim"}, `${st.calls} call${st.calls === 1 ? "" : "s"} since the agent started · last: ${st.last_call.tool} from ${st.last_call.from}, ${fmtAgo(st.last_call.t)}`));
    status.appendChild(h("div", {class: "dim"}, "Tools: ", (st.tools || []).map(x => x.name + (x.approval ? " (asks you first)" : "")).join(", ")));
  }

  async function refresh() { st = await get("/api/mcp"); paint(); }

  async function run(action) {
    busy = true; confirming = null; paint();
    const r = await post(`/api/mcp/terminal/${action}`, {confirm: true});
    busy = false;
    toast(r.message || r.error || "done", r.ok ? null : "err");
    refresh();
  }

  box.append(
    field("Stick tools", toggle("mcpServer", "Claude sessions can notify you, ask A/B questions and show progress on the stick", c.mcp_server !== false),
      "An MCP server inside this agent (http://127.0.0.1:8660/mcp, this Mac only, needs the agent's token). Voice requests always get it while it's on. Tools that change the Mac (open a URL / app, quick commands) ask for your approval first."),
    field("Terminal sessions", status,
      "Lets every Claude Code session in a terminal use the stick — e.g. “run the tests and notify me on the stick when done”. New sessions pick it up; tools show up as mcp__stick__…"),
  );
  refresh();
  paint();
  return box;
}
