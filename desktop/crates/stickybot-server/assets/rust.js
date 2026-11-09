// The Rust app's own additions to the shared dashboard page (the Python app never loads this).
// Settings → "Import from StickyBot": takes over the Python app's settings (POST /api/import).
import {h, card, get, post, put, toast} from "/static/lib.js";

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
    if (!confirm("Take over the StickyBot (Python) app's settings, voice commands, sound packs and history?\n\n" +
                 "This app's current settings and commands are kept as *.before-import. The Python app's files are only read.")) return;
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
    help.textContent = r.detail || (r.on ? "StickyBot (Rust) starts when you log in." : "Opens the menu bar app when you log in to this Mac.");
  }});
  return card("Start at login",
    h("div", {class: "field"}, h("label", null, "Login item"),
      h("label", {class: "switch"}, box, h("span", null, "Start StickyBot (Rust) when I log in")), help));
}

function switchCard(s) {
  const note = h("div", {class: "help"}, s.switched
    ? `Switched over: this app takes the Python app's place (port 8660, its token)${s.restart_needed ? " from its next start" : ""}.`
    : s.python_running
      ? "Quit the StickyBot (Python) app first; then this app can take its place (port 8660, its token), so the Claude Code hooks and terminal stick tools keep working."
      : "Take the StickyBot (Python) app's place: port 8660 and its token, from the next start (see desktop/SWITCHOVER.md).");
  const btn = h("button", {class: "btn", type: "button", disabled: !s.switched && s.python_running, onclick: async () => {
    const body = s.switched ? {undo: true} : {confirm: true};
    if (!s.switched && !confirm("Take the Python app's place (port 8660 and its token) from the next start?")) return;
    const r = await post("/api/switch-over", body);
    if (r.error) { toast(r.error, "err"); return; }
    toast(s.switched ? "Back to port 8670 from the next start" : "Switched over: quit and reopen the app");
  }}, s.switched ? "Undo" : "Switch over");
  return card("Switch over from StickyBot (Python)", h("div", {class: "field"}, h("label", null, "Port 8660"), h("div", {class: "row"}, btn), note));
}

let busy = false;
async function maybeAdd() {
  const title = document.getElementById("pageTitle");
  const page = document.getElementById("page");
  if (busy || !title || !page || title.textContent !== "Settings") return;
  if (document.getElementById("rsImport") || !page.querySelector(".card")) return;
  busy = true;
  try {
    const [info, login, sw] = await Promise.all([get("/api/import"), get("/api/login-item"), get("/api/switch-over")]);
    if (!info.error && !document.getElementById("rsImport") && title.textContent === "Settings") {
      if (!login.error) {
        const l = loginCard(login);
        l.id = "rsLogin-card";
        page.appendChild(l);
      }
      const c = importCard(info);
      c.id = "rsImport";
      page.appendChild(c);
      if (!sw.error) page.appendChild(switchCard(sw));
    }
  } finally { busy = false; }
}

const page = document.getElementById("page");
if (page) new MutationObserver(maybeAdd).observe(page, {childList: true, subtree: false});
maybeAdd();
