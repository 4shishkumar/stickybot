// Feedback ("Report an issue"): a bug, an idea or a question for the StickyBot team. The Rust app
// posts it to the team's feedback web app (a Google Sheet behind Apps Script; feedback.rs) and keeps
// a copy here ("My reports"). Diagnostics are opt-in, redacted on this Mac, and shown here exactly
// as they'd be sent. Without the web app set up: copy the report or email it.
import {h, get, post, toast, card, field, clear, fmtAgo} from "/static/lib.js";

export const title = "Feedback";
export const subtitle = "Report a bug, suggest an idea or ask a question — it goes to the StickyBot team.";

const TYPES = [
  ["Bug", "What happened?", "What did StickyBot do? Any message on the stick or here?"],
  ["Idea", "What would you like?", "What should StickyBot do, and when would it help?"],
  ["Question", "What's your question?", "Ask away."],
];

let st = null;  // the page's state while it's shown

export function render(root) {
  st = {type: "Bug", info: null, diag: null, editing: false, sending: false};
  const s = st;
  const notice = h("div", {class: "notice", hidden: true, role: "status"});
  const seg = h("div", {class: "seg", role: "group", "aria-label": "Type"});
  const what = h("textarea", {id: "fbWhat", rows: 5, maxlength: 4000, required: true});
  const whatLabel = h("label", {for: "fbWhat"}, TYPES[0][1]);
  const expected = h("textarea", {id: "fbExpected", rows: 3, maxlength: 4000,
    placeholder: "What should have happened instead?"});
  const steps = h("textarea", {id: "fbSteps", rows: 3, maxlength: 4000,
    placeholder: "1. Hold B on the stick\n2. Say …\n3. …"});
  const contact = h("input", {type: "text", id: "fbContact", maxlength: 200, autocomplete: "email",
    placeholder: "Name or email (optional)"});
  const expectedField = field("What you expected", expected);
  const stepsField = field("Steps to reproduce (optional)", steps);
  const setType = t => {
    s.type = t;
    const [, label, hint] = TYPES.find(x => x[0] === t);
    whatLabel.textContent = label;
    what.placeholder = hint;
    for (const b of seg.children) b.classList.toggle("on", b.dataset.type === t);
    expectedField.hidden = stepsField.hidden = t !== "Bug";
  };
  for (const [t] of TYPES) {
    seg.appendChild(h("button", {type: "button", "data-type": t, onclick: () => setType(t)}, t));
  }

  // diagnostics: opt-in, redacted, shown as sent
  const attach = h("input", {type: "checkbox", id: "fbAttach", onchange: () => showDiag()});
  const diagText = h("textarea", {id: "fbDiag", rows: 14, readonly: true, spellcheck: "false",
    "aria-label": "Diagnostics that will be sent", style: "margin-top:8px"});
  const diagSize = h("span", {class: "small dim"});
  const editBtn = h("button", {class: "btn sm", type: "button", onclick: () => {
    s.editing = !s.editing;
    diagText.readOnly = !s.editing;
    editBtn.textContent = s.editing ? "Done editing" : "Edit";
    if (s.editing) diagText.focus();
  }}, "Edit");
  const reloadBtn = h("button", {class: "btn sm", type: "button", onclick: () => loadDiag(true)}, "Refresh");
  const diagBox = h("div", {hidden: true},
    h("div", {class: "small dim", style: "margin-top:6px"}, "Exactly this text is sent. Removed on this Mac first: keys and tokens, email addresses, your user and Mac name (/Users/you → ~), GitHub repos and PR titles, what you said to the stick and Claude's replies, Wi-Fi names, the stick's name, Bluetooth addresses and serial numbers (as a short hash). Read it; edit or untick it if anything else shouldn't go."),
    diagText,
    h("div", {class: "row"}, editBtn, reloadBtn, diagSize));
  diagText.addEventListener("input", () => { diagSize.textContent = sizeText(diagText.value); });

  async function loadDiag(force) {
    if (s.diag && !force) { diagText.value = s.diag.text; diagSize.textContent = sizeText(s.diag.text); return; }
    diagText.value = "Collecting…";
    const d = await get("/api/feedback/diagnostics");
    if (s !== st) return;
    if (d.error) { diagText.value = ""; toast(d.error, "err"); return; }
    s.diag = d;
    diagText.value = d.text;
    diagSize.textContent = sizeText(d.text);
  }
  function showDiag() {
    diagBox.hidden = !attach.checked;
    if (attach.checked) loadDiag(false);
  }

  const report = () => ({
    type: s.type, what: what.value.trim(),
    expected: s.type === "Bug" ? expected.value.trim() : "",
    steps: s.type === "Bug" ? steps.value.trim() : "",
    contact: contact.value.trim(),
    diagnostics: attach.checked ? diagText.value.trim() : "",
  });

  const sendBtn = h("button", {class: "btn primary", type: "button", id: "fbSend", onclick: send}, "Send report");
  const copyBtn = h("button", {class: "btn", type: "button", onclick: () => copyReport()}, "Copy report");
  const mailBtn = h("button", {class: "btn", type: "button", onclick: () => mail()}, "Email it instead");
  const left = h("span", {class: "small dim"});
  const mine = h("div", {class: "fb-mine"});

  async function send() {
    const r = report();
    if (!r.what) { toast("Say what happened first", "err"); what.focus(); return; }
    if (s.sending) return;
    s.sending = true;
    sendBtn.disabled = true;
    sendBtn.textContent = "Sending…";
    const res = await post("/api/feedback", r);
    if (s !== st) return;
    s.sending = false;
    sendBtn.textContent = "Send report";
    sendBtn.disabled = !(s.info && s.info.configured);
    if (res.error) {
      say(res.error, "warn", res.report ? smallBtn("Copy report", () => copyReport(res.report)) : null);
      toast(res.error, "err");
    } else {
      say(`Thanks! Sent as ${res.id} — mention it if you write to us about this.`, "ok");
      toast(`Sent as ${res.id}`);
      what.value = expected.value = steps.value = "";
    }
    refreshInfo();
    loadMine();
  }

  function say(text, kind, extra) {
    notice.hidden = false;
    notice.className = "notice" + (kind === "warn" ? " warn" : "");
    clear(notice).appendChild(h("div", {class: "row"}, h("span", null, text), extra || null));
  }

  async function copyReport(text) {
    const r = report();
    if (!text && !r.what) { toast("Say what happened first", "err"); what.focus(); return; }
    const ok = await copy(text || reportText(r, s.diag && s.diag.version));
    if (ok) toast("Report copied — paste it into an email or a GitHub issue");
    else toast("Couldn't copy: select the text and press ⌘C", "err");
  }

  async function mail() {
    const r = report();
    if (!r.what) { toast("Say what happened first", "err"); what.focus(); return; }
    const text = reportText(r, s.diag && s.diag.version);
    const subject = `StickyBot ${r.type.toLowerCase()}: ${r.what.split("\n")[0].slice(0, 60)}`;
    const max = 1800;  // mail apps take only so much from a link
    let body = text;
    if (body.length > max) {
      body = body.slice(0, max) + "\n\n[… cut: the full report is on the clipboard — paste it here]";
      await copy(text);
      toast("The full report is on the clipboard: paste it into the mail");
    }
    const a = h("a", {href: `mailto:?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`, target: "_blank", rel: "noopener"});
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  async function refreshInfo() {
    const info = await get("/api/feedback");
    if (s !== st) return;
    if (info.error) { say(info.error, "warn"); return; }
    s.info = info;
    sendBtn.disabled = !info.configured || s.sending;
    left.textContent = info.configured ? `${info.left} of ${info.per_hour} reports left this hour` : "";
    if (!info.configured) {
      say("Feedback isn't set up yet in this version — use Copy report or Email it instead.", "warn");
    }
  }

  async function loadMine() {
    const list = await get("/api/feedback/reports");
    if (s !== st || !Array.isArray(list)) return;
    clear(mine);
    if (!list.length) { mine.appendChild(h("div", {class: "small dim"}, "Reports you send are listed here, with their ID.")); return; }
    for (const r of list) {
      const when = Date.parse(r.created) / 1000;
      const state = r.sent
        ? h("span", {class: "tag ok", style: "align-self:center"}, "Sent")
        : h("span", {class: "tag err", style: "align-self:center", title: r.error || ""}, "Not sent");
      const retry = r.sent ? null : smallBtn("Retry", async () => {
        const res = await post(`/api/feedback/reports/${r.id}/retry`, {});
        if (res.error) toast(res.error, "err"); else toast(`Sent as ${res.id}`);
        refreshInfo();
        loadMine();
      });
      const copyOne = smallBtn("Copy", async () => {
        const one = await get(`/api/feedback/reports/${r.id}`);
        if (one.error) { toast(one.error, "err"); return; }
        if (await copy(one.report)) toast(`${r.id} copied`);
      });
      mine.appendChild(h("div", {class: "fb-report", style: "display:flex;gap:10px;align-items:center;padding:8px 0;border-top:1px solid var(--line)"},
        h("div", {style: "flex:1;min-width:0"},
          h("div", {class: "row"}, h("b", {class: "mono"}, r.id), state, h("span", {class: "small dim"}, `${r.type} · ${isNaN(when) ? "" : fmtAgo(when)}`)),
          h("div", {class: "small dim", style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap"}, r.what),
          !r.sent && r.error ? h("div", {class: "small dim"}, "Last try: " + r.error) : null),
        h("div", {class: "row"}, retry, copyOne)));
    }
  }

  root.appendChild(notice);
  root.appendChild(card("Your report",
    h("div", {class: "field"}, h("label", null, "Type"), seg),
    h("div", {class: "field"}, whatLabel, what),
    expectedField,
    stepsField,
    field("Contact (optional)", contact, "Only if you'd like an answer. Sent as you write it."),
  ));
  root.appendChild(card("Diagnostics",
    h("div", {class: "field"},
      h("label", {class: "switch"}, attach, h("span", null, "Attach diagnostics")),
      h("div", {class: "help"}, "Helps with bugs: app and firmware version, macOS version, the stick's model and link, which features are on, and the last 200 lines of the app's log — redacted first.")),
    diagBox));
  root.appendChild(h("div", {class: "row", style: "margin:4px 0 18px"}, sendBtn, copyBtn, mailBtn, left));
  root.appendChild(card("My reports", mine));
  setType("Bug");
  refreshInfo();
  loadMine();
  get("/api/feedback/diagnostics").then(d => { if (s === st && !d.error && !s.diag) s.diag = d; });  // the version line
}

export function leave() { st = null; }

function smallBtn(label, onclick) {
  return h("button", {class: "btn sm", type: "button", onclick}, label);
}

function sizeText(t) {
  const kb = new Blob([t]).size / 1024;
  return `${kb.toFixed(1)} KB of 30 KB`;
}

// close to feedback.rs `Report::text` (that one also has the ID)
export function reportText(r, version) {
  const parts = [`StickyBot ${r.type.toLowerCase()} report\n`];
  const add = (title, v) => { if (v) parts.push(`\n## ${title}\n${v}\n`); };
  add("What happened", r.what);
  add("What you expected", r.expected);
  add("Steps", r.steps);
  add("Contact", r.contact);
  add("Version", version || "");
  add("Diagnostics", r.diagnostics);
  return parts.join("");
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (e) {
    const ta = h("textarea", {style: "position:fixed;opacity:0"});
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    let ok = false;
    try { ok = document.execCommand("copy"); } catch (e2) { ok = false; }
    ta.remove();
    return ok;
  }
}
