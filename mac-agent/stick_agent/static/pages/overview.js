// Overview: what needs you, then one card per part of the system.
import {h, clear, card, store, get, post, proto, approvalCard, fmtTime, fmtAgo, STATE_LABEL, report, refreshStatus} from "../lib.js";

export const title = "Overview";
export const subtitle = "Everything at a glance. Hold B on the stick and speak, or type in Activity.";

let off = null, timer = null, stickState = null, firmware = null, last = null, onboarding = null, pauseBtn = null;

export function actions() {
  pauseBtn = h("button", {class: "btn", type: "button", title: "Stop listening to the stick for now", onclick: async () => {
    report(await post("/api/pause", {paused: store.status.link !== "paused"}));
    setTimeout(refreshStatus, 300);
  }}, store.status.link === "paused" ? "Resume" : "Pause");
  return [pauseBtn, h("a", {class: "btn primary", href: "#activity"}, "Ask something")];
}

export function render(root) {
  const approvals = h("div", {class: "list", style: "gap:10px"});
  const setup = h("div");
  const grid = h("div", {class: "cards"});
  root.append(setup, approvals, grid);

  const paint = () => {
    const s = store.status || {};
    if (pauseBtn) pauseBtn.textContent = s.link === "paused" ? "Resume" : "Pause";
    // Pending approvals first
    clear(approvals);
    if ((s.prompts || []).length) {
      approvals.appendChild(h("h2", null, "Waiting for your approval"));
      for (const p of s.prompts) approvals.appendChild(approvalCard(p));
    }
    // Setup banner
    clear(setup);
    if (onboarding && !onboarding.error && !onboarding.done) {
      setup.appendChild(h("div", {class: "notice warn"},
        h("div", {class: "grow"}, h("b", null, "Setup isn't finished"), h("span", {class: "dim"}, "A few steps left to get your stick fully working.")),
        h("a", {class: "btn ask", href: "#setup"}, "Continue setup")));
    }
    clear(grid);
    grid.append(stickCard(s), agentCard(s), claudeCard(s), githubCard(s), weatherCard(s), lastCard());
  };

  off = store.on((type, ev) => {
    if (type === "status" || (type === "event" && ["reply", "heard", "prompt", "prompt_done"].includes(ev.kind))) paint();
  });
  paint();
  const slow = async () => {
    const [st, fw, hist, ob] = await Promise.all([
      store.status.link === "connected" ? proto("state") : Promise.resolve(null),
      get("/api/firmware"), get("/api/history?n=1"), get("/api/onboarding")]);
    stickState = st && st.ok && st.reply ? st.reply : null;
    firmware = fw && !fw.error ? fw : null;
    last = Array.isArray(hist) && hist.length ? hist[0] : null;
    onboarding = ob;
    paint();
  };
  slow();
  timer = setInterval(slow, 30000);
}

export function leave() { if (off) off(); clearInterval(timer); }

function statCard(label, big, meta, foot, tagEl) {
  return h("section", {class: "card stat"},
    h("div", {class: "row"}, h("span", {class: "eyebrow"}, label), tagEl ? h("span", {style: "margin-left:auto"}, tagEl) : null),
    h("div", {class: "big"}, big),
    meta && meta.length ? h("div", {class: "meta"}, meta.map(m => h("span", null, m))) : null,
    foot ? h("div", {class: "foot"}, foot) : null);
}

function stickCard(s) {
  if (!s.stick) {
    return statCard("Stick", "No stick yet", ["Pair one with this Mac, then pick it here."], h("a", {class: "btn sm primary", href: "#setup"}, "Add a stick"));
  }
  const link = s.link || "searching";
  const tag = h("span", {class: "pill s-" + link}, h("span", {class: "dot"}), STATE_LABEL[link] || link);
  const meta = [];
  if (stickState) {
    if (stickState.battery >= 0) meta.push(`Battery ${stickState.battery}%${stickState.charging ? " · charging" : ""}`);
    meta.push(`Combo ${stickState.combo || "—"} · Mac slot ${stickState.slot || "—"}`);
  }
  const fw = (stickState && stickState.fw) || (firmware && firmware.connected && firmware.connected.version);
  if (fw) meta.push(`Firmware ${fw}` + (firmware && firmware.bundled && firmware.bundled.version && firmware.bundled.version !== fw ? ` · ${firmware.bundled.version} available` : ""));
  return statCard("Stick", s.stick.name, meta, h("a", {class: "btn sm", href: "#stick"}, "Stick settings"), tag);
}

function agentCard(s) {
  const st = s.state || "starting";
  const tag = h("span", {class: "pill s-" + (store.online ? st : "offline")}, h("span", {class: "dot"}), store.online ? (STATE_LABEL[st] || st) : "Offline");
  const stats = s.stats || {};
  return statCard("Agent", `${stats.requests || 0} request${stats.requests === 1 ? "" : "s"}`,
    [`${stats.quick || 0} quick commands · ${stats.claude || 0} to Claude`,
     `Speech model ${s.stt_ready ? "ready (on this Mac)" : "loading…"}`,
     stats.started ? `Running since ${fmtTime(stats.started)}` : null].filter(Boolean),
    null, tag);
}

function claudeCard(s) {
  const n = (s.prompts || []).length;
  return statCard("Claude Code", n ? `${n} waiting` : "Nothing waiting",
    [`Model: ${s.model || "sonnet"}`,
     s.stick_approvals ? "Approvals go to the stick and here first (30 s)" : "Approvals: terminal only",
     s.claude_sessions !== undefined ? `${s.claude_sessions} sessions open` : null].filter(Boolean),
    h("a", {class: "btn sm", href: "#settings"}, "Claude settings"),
    n ? h("span", {class: "tag ask"}, "Needs you") : null);
}

function githubCard(s) {
  const g = s.github || {};
  let big = "Not checked yet", tag = null;
  if (g.error) { big = "Can't reach GitHub"; tag = h("span", {class: "tag err"}, "Error"); }
  else if (g.ok) { big = `${g.tracked || 0} PRs watched`; tag = h("span", {class: "tag ok"}, "Watching"); }
  return statCard("GitHub", big,
    [g.error ? g.error : g.ok ? `Checked ${fmtAgo(g.ok)}` : "Uses your gh login, read-only"],
    h("a", {class: "btn sm", href: "#events"}, "Alerts"), tag);
}

function weatherCard(s) {
  const w = s.weather;
  if (!w) return statCard("Weather", "—", ["Turn it on in Settings → Home screen."]);
  return statCard("Weather", `${w.t}° ${w.c}`, [`${w.at || ""} · low ${w.lo}° / high ${w.hi}°`, "Shown on the stick's home screen"]);
}

function lastCard() {
  const reply = [...store.events].reverse().find(e => e.kind === "reply");
  const heard = [...store.events].reverse().find(e => e.kind === "heard");
  const text = reply ? reply.text : last ? last.reply : null;
  const asked = reply && heard ? heard.text : last ? last.heard : null;
  const when = reply ? reply.t : last ? last.t : null;
  if (!text) return statCard("Last answer", "Nothing yet", ["Hold B on the stick and ask something."]);
  return h("section", {class: "card stat"},
    h("div", {class: "row"}, h("span", {class: "eyebrow"}, "Last answer"), when ? h("span", {class: "faint small", style: "margin-left:auto"}, fmtAgo(when)) : null),
    asked ? h("div", {class: "dim small"}, `“${asked}”`) : null,
    h("div", null, text),
    h("div", {class: "foot"}, h("a", {class: "btn sm", href: "#activity"}, "Open activity")));
}
