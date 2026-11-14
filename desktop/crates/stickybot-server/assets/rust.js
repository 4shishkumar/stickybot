// The Rust app's own additions to the shared dashboard page (the Python app never loads this).
// Settings → "Import from StickyBot": takes over the Python app's settings (POST /api/import).
// Settings → Dock: "Always show in the Dock" (config dock_always; the app's dock.rs).
// The Feedback page ("Report an issue", feedback.js) in the nav, before Logs.
// The Connections page (the Mac's Calendar, connections.js) in the nav, before Feedback.
// Events: the meeting reminder's own name in the list (the shared page only knows the older kinds).
import {h, card, get, post, put, toast, ask} from "/static/lib.js";
import * as feedback from "/static-rs/feedback.js";
import * as connections from "/static-rs/connections.js";

if (window.stickyAddPage) {
  window.stickyAddPage("feedback", "Feedback", feedback,
    '<path d="M4 4h16v12H8l-4 4z"/><path d="M12 7.5v4M12 13.8v.2"/>', "logs");
  window.stickyAddPage("connections", "Connections", connections,
    '<path d="M9 3v4M15 3v4"/><path d="M6 7h12v4a6 6 0 0 1-12 0z"/><path d="M12 17v4"/>', "feedback");
}

function dockCard(cfg) {
  const box = h("input", {type: "checkbox", id: "rsDock", checked: !!cfg.dock_always, onchange: async () => {
    const r = await put("/api/config", {dock_always: box.checked});
    if (r.error) { toast(r.error, "err"); box.checked = !box.checked; return; }
    toast(box.checked ? "StickyBot stays in the Dock" : "In the Dock only while this window is open");
  }});
  return card("Dock",
    h("div", {class: "field"}, h("label", null, "Dock icon"),
      h("label", {class: "switch"}, box, h("span", null, "Always show in the Dock")),
      h("div", {class: "help"}, "Otherwise StickyBot is in the Dock and in ⌘-Tab only while this window is open; the menu bar icon is always there.")));
}

function importCard(info) {
  const out = h("div", {class: "help", id: "rsImportOut"});
  const show = (r) => {
    out.textContent = "";
    if (!r) return;
    const lines = [...(r.imported || []).map(s => "✓ " + s), ...(r.skipped || []).map(s => "– " + s)];
    for (const l of lines) out.appendChild(h("div", null, l));
  };
  if (info.last) {
    const when = new Date(info.last.t * 1000).toLocaleString();
    out.appendChild(h("div", null, `Last import: ${when}`));
  }
  const btn = h("button", {class: "btn", type: "button", disabled: !info.available, onclick: async () => {
    if (!await ask("Take over the StickyBot (Python) app's settings, voice commands, sound packs and history?\n\n" +
                   "This app's current settings and commands are kept as *.before-import. The Python app's files are only read.",
                   {ok: "Import"})) return;
    btn.disabled = true;
    const r = await post("/api/import", {});
    btn.disabled = false;
    if (r.error) { toast(r.error, "err"); return; }
    toast("Imported from StickyBot");
    show(r);
    setTimeout(() => location.reload(), 1500);
  }}, "Import from StickyBot");
  return card("Import from StickyBot (Python)",
    h("div", {class: "field"},
      h("label", null, "Settings"),
      h("div", {class: "row"}, btn),
      h("div", {class: "help"}, info.available
        ? `Copies the stick, GitHub repo, brain, wireless mic and other settings, your voice commands, sound packs and history from ${info.from}. Read-only there.`
        : "No StickyBot (Python) settings found on this Mac.")),
    out);
}

function loginCard(s) {
  const help = h("div", {class: "help"}, s.detail || "Opens the menu bar app when you log in to this Mac.");
  const box = h("input", {type: "checkbox", id: "rsLogin", checked: s.on, disabled: !s.available, onchange: async () => {
    const r = await put("/api/login-item", {on: box.checked});
    if (r.error) { toast(r.error, "err"); box.checked = !box.checked; return; }
    help.textContent = r.detail || (r.on ? "StickyBot starts when you log in." : "Opens the menu bar app when you log in to this Mac.");
  }});
  return card("Start at login",
    h("div", {class: "field"}, h("label", null, "Login item"),
      h("label", {class: "switch"}, box, h("span", null, "Start StickyBot when I log in")), help));
}

function switchCard(s) {
  const note = h("div", {class: "help"}, s.switched
    ? `Switched over: this app takes the Python app's place (port 8660, its token)${s.restart_needed ? " from its next start" : ""}.`
    : s.python_running
      ? "Quit the StickyBot (Python) app first; then this app can take its place (port 8660, its token), so the Claude Code hooks and terminal stick tools keep working."
      : "Take the StickyBot (Python) app's place: port 8660 and its token, from the next start (see desktop/SWITCHOVER.md).");
  const agent = s.python_login_item ? h("div", {class: "help"},
    `The Python app's login item (${s.python_login_item}) is still installed: unless it's disabled ` +
    "(launchctl disable gui/$UID/com.stickkey.agent, or System Settings → General → Login Items), it starts " +
    "again at the next login and takes port 8660 back. See desktop/SWITCHOVER.md.") : null;
  const btn = h("button", {class: "btn", type: "button", disabled: !s.switched && s.python_running, onclick: async () => {
    const body = s.switched ? {undo: true} : {confirm: true};
    if (!s.switched && !await ask("Take the Python app's place (port 8660 and its token) from the next start?", {ok: "Switch over"})) return;
    const r = await post("/api/switch-over", body);
    if (r.error) { toast(r.error, "err"); return; }
    toast(s.switched ? "Back to port 8670 from the next start" : "Switched over: quit and reopen the app");
  }}, s.switched ? "Undo" : "Switch over");
  return card("Switch over from StickyBot (Python)", h("div", {class: "field"}, h("label", null, "Port 8660"), h("div", {class: "row"}, btn), note, agent));
}

let busy = false;
async function maybeAdd() {
  const title = document.getElementById("pageTitle");
  const page = document.getElementById("page");
  if (busy || !title || !page || title.textContent !== "Settings") return;
  if (document.getElementById("rsImport") || !page.querySelector(".card")) return;
  busy = true;
  try {
    const [info, login, sw, cfg] = await Promise.all([get("/api/import"), get("/api/login-item"), get("/api/switch-over"), get("/api/config")]);
    if (!info.error && !document.getElementById("rsImport") && title.textContent === "Settings") {
      if (!login.error) {
        const l = loginCard(login);
        l.id = "rsLogin-card";
        page.appendChild(l);
      }
      if (!cfg.error) {
        const d = dockCard(cfg);
        d.id = "rsDock-card";
        page.appendChild(d);
      }
      const c = importCard(info);
      c.id = "rsImport";
      page.appendChild(c);
      if (!sw.error && !sw.fresh) page.appendChild(switchCard(sw));  // fresh: no Python app to switch from
    }
  } finally { busy = false; }
}

function relabelEvents() {
  const title = document.getElementById("pageTitle");
  if (!title || title.textContent !== "Events") return;
  for (const b of document.querySelectorAll("#page .ev-row .what b")) {
    if (b.textContent === "meeting_soon") b.textContent = "Meeting reminder";
  }
}

const page = document.getElementById("page");
if (page) new MutationObserver(() => { maybeAdd(); relabelEvents(); }).observe(page, {childList: true, subtree: false});
maybeAdd();
