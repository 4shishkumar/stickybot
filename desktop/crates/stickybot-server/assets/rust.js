// The Rust app's own additions to the shared dashboard page (the Python app never loads this).
// Settings → "Import from StickyBot": takes over the Python app's settings (POST /api/import).
import {h, card, get, post, toast} from "/static/lib.js";

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

let busy = false;
async function maybeAdd() {
  const title = document.getElementById("pageTitle");
  const page = document.getElementById("page");
  if (busy || !title || !page || title.textContent !== "Settings") return;
  if (document.getElementById("rsImport") || !page.querySelector(".card")) return;
  busy = true;
  try {
    const info = await get("/api/import");
    if (!info.error && !document.getElementById("rsImport") && title.textContent === "Settings") {
      const c = importCard(info);
      c.id = "rsImport";
      page.appendChild(c);
    }
  } finally { busy = false; }
}

const page = document.getElementById("page");
if (page) new MutationObserver(maybeAdd).observe(page, {childList: true, subtree: false});
maybeAdd();
