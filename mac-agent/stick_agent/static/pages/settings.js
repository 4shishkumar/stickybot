// Settings: the agent's own options (saved in ~/.stick-agent/config.json).
import {h, clear, card, field, toggle, get, put, post, report, toast, refreshStatus, store} from "../lib.js";
import {brainPanel, pickedModel} from "../brain.js";
import {componentsPanel} from "../components.js";
import {mcpPanel} from "../mcp.js";
import {updatesPanel} from "../updates.js";

export const title = "Settings";
export const subtitle = "How the agent behaves. Changes apply right away once you save.";

const SOURCES = [["facts", "Fun facts"], ["hn", "Hacker News"], ["onthisday", "On this day"], ["wikipedia", "Wikipedia featured article"]];
let root = null, cfg = null, unsub = null, unsubBrain = null;

export function actions() {
  return [
    h("button", {class: "btn", type: "button", title: "Restart the agent (a few seconds)", onclick: async () => {
      post("/api/restart");
      toast("Restarting the agent…");
      setTimeout(() => location.reload(), 4500);
    }}, "Restart agent"),
    h("button", {class: "btn primary", type: "button", onclick: save}, "Save"),
  ];
}

export async function render(el) {
  root = el;
  clear(root).appendChild(h("div", {class: "empty"}, "Loading…"));
  cfg = await get("/api/config");
  if (!root) return;
  clear(root);
  if (cfg.error) { root.appendChild(h("div", {class: "notice err"}, cfg.error)); return; }
  const c = cfg;
  const voice = h("select", {id: "voice"}, h("option", {value: ""}, "System voice"));
  get("/api/voices").then(vs => {
    if (!Array.isArray(vs)) return;
    for (const v of vs) voice.appendChild(h("option", {value: v.name, selected: c.speak_voice === v.name}, `${v.name} (${v.lang})`));
  });
  root.append(
    card("Your stick", stickPicker()),
    card("Brain", brainPanel(c), brainStatus()),
    card("Claude Code",
      field("Model", h("select", {id: "model"}, [["sonnet", "Sonnet — fast (recommended)"], ["opus", "Opus — smartest, slower"], ["haiku", "Haiku — fastest"]]
        .map(([v, l]) => h("option", {value: v, selected: c.model === v}, l)))),
      field("Follow-ups", h("div", {class: "row"}, h("input", {type: "number", id: "followup", min: 0, max: 120, value: c.followup_minutes ?? 10, style: "width:90px"}), h("span", {class: "dim"}, "minutes")),
        "A request within this time continues the same conversation; after it, a new chat. (Or say “new chat”.) Applies to OpenRouter too."),
      field("Claude Code runs", h("select", {id: "brainMode"}, [["session", "as one session, always ready (faster)"], ["oneshot", "once per request (the old way)"]]
        .map(([v, l]) => h("option", {value: v, selected: (c.brain_mode || "session") === v}, l)))),
      field("Live replies", toggle("streamReplies", "Show the reply on the stick while it's written (both brains)", c.stream_replies !== false))),
    card("Speech & replies",
      field("Read replies aloud", toggle("speak", "Short replies are spoken; long ones stay on the screen", c.speak_replies)),
      field("Play on", h("select", {id: "speakOut"}, [["stick", "the stick's speaker"], ["mac", "the Mac's speakers"]].map(([v, l]) => h("option", {value: v, selected: (c.speak_output || "stick") === v}, l)))),
      field("Short means", h("div", {class: "row"}, h("span", {class: "dim"}, "up to"), h("input", {type: "number", id: "speakMax", min: 5, max: 300, value: c.speak_max_words || 40, style: "width:90px"}), h("span", {class: "dim"}, "words"))),
      field("Voice", h("div", {class: "row"}, voice, h("button", {class: "btn", onclick: async () => {
        await put("/api/config", {speak_voice: voice.value, speak_output: document.getElementById("speakOut").value});
        report(await post("/api/voice/test", {}), "Playing a sample");
      }}, "Test")), "Better voices: System Settings → Accessibility → Spoken Content → System voice → Manage voices."),
      field("As it's written", toggle("speakStream", "Start speaking with the first sentence (don't wait for the whole reply)", c.speak_stream !== false),
        "Long replies: the first sentences up to the word limit are read, the rest stays on the screen."),
      field("Quick commands", toggle("speakQuick", "Also speak confirmations like “Opened Slack.”", c.speak_quick)),
      field("Speech model", h("select", {id: "sttModel"}, [["mlx-community/parakeet-tdt-0.6b-v2", "English only (recommended)"], ["mlx-community/parakeet-tdt-0.6b-v3", "25 languages"]]
        .map(([v, l]) => h("option", {value: v, selected: (c.stt_model || "mlx-community/parakeet-tdt-0.6b-v2") === v}, l))), "Runs on this Mac. Changing it takes effect after a restart (first use downloads ~2.5 GB).")),
    card("Home screen",
      field("Weather", toggle("wxOn", "Show the weather on the stick", c.weather !== false)),
      field("Location", h("input", {id: "wxWhere", placeholder: "automatic (from this network)", value: c.weather_location === "auto" ? "" : (c.weather_location || "")}),
        "Leave empty to guess the city from this Mac's internet connection. Weather comes from Open-Meteo."),
      field("Facts & news ticker", toggle("tickerOn", "Rotate facts and news when nothing needs you", c.ticker !== false)),
      field("Sources", h("div", {class: "row", style: "gap:16px"}, SOURCES.map(([v, l]) =>
        h("label", {class: "switch"}, h("input", {type: "checkbox", class: "idle", value: v, checked: (c.idle_content || []).includes(v)}), h("span", null, l)))))),
    card("Wireless Wispr mic",
      field("Wireless mic", toggle("wirelessMic", "Hold A away from USB: the stick's mic reaches Wispr over Bluetooth", !!c.wireless_mic),
        `While you hold A, a microphone called “${micName(c)}” appears on this Mac and Wispr switches to it; a moment after you let go it disappears and Wispr is back on your usual mic. Plugged in by USB, the stick is its USB mic (“${usbMicName(c)}”, firmware 1.8.1+) as before.`),
      h("div", {class: "notice", style: "margin-bottom:12px"}, h("div", {class: "grow"},
        h("b", null, "Once: "),
        h("span", null, `Hold A once (or use the button below); in Wispr Flow → Settings → Microphone, drag “${micName(c)}” to the top of the ranking. Wispr then uses the stick while you hold A and your usual mic otherwise.`),
        h("div", {class: "small dim", style: "margin-top:4px"}, `You can put “${usbMicName(c)}” wherever you like in that list — Wispr only sees it while the stick is plugged in. Renamed the mic (here or after an update)? Wispr sees a new device: rank it again.`))),
      field("Ranking step", showMicButton(c), "Makes the mic appear for 60 s so you can drag it to the top in Wispr's settings. Needs the stick connected by Bluetooth and not plugged in by USB."),
      wirelessMicStatus(),
      h("details", {style: "margin-top:8px"}, h("summary", {class: "small dim"}, "Advanced timings"),
        slider("wisprSwitch", "Switch time", c.wispr_switch_ms ?? 250, 0, 1000, 25,
          "After the mic appears, how long the stick waits before pressing Wispr's shortcut, so Wispr has switched to it. Longer = safer, the key goes down later. The stick presses anyway after 800 ms (creating the mic takes ~100–170 ms on top of this)."),
        slider("wisprTail", "Keep after release", c.wispr_tail_ms ?? 800, 0, 3000, 50,
          "How long the mic stays after the key-up, so Wispr finishes reading it (removing it mid-dictation stops Wispr with “MicDisconnected”). It also waits while Wispr is still reading it, up to 4 s more."),
        field("Mic name", h("input", {id: "wisprMicName", value: customMicName(c), placeholder: autoMicName(c), style: "max-width:260px"}),
          "The name Wispr shows (plus “(Aggregate)”). Empty = named after the stick. Change it only if that causes trouble — Wispr then needs the ranking again."))),
    card("Approvals",
      field("Stick approvals", toggle("stickApprovals", "Claude Code's permission prompts go to the stick and here first", c.stick_approvals !== false),
        "Off: Claude Code asks in the terminal straight away. On: it waits up to 30 s for A / B.")),
    card("Stick tools for Claude Code (MCP)", mcpPanel(c)),
    card("Updates", updatesPanel(c)),
    card("Advanced",
      field("Run without asking", h("input", {id: "safeTools", class: "mono", value: (c.safe_tools || []).join(", ")}), "Claude Code tools that never need approval."),
      field("Claude path", h("input", {id: "claudePath", class: "mono", placeholder: "found automatically", value: c.claude_path || ""})),
      field("Pre-warm", toggle("brainPrewarm", "Start the Claude Code session when the stick connects", c.brain_prewarm !== false))),
    card("Components", componentsPanel()),
  );
}

export function leave() {
  root = null;
  if (unsub) unsub();
  if (unsubBrain) unsubBrain();
  unsub = unsubBrain = null;
  wmListeners.clear();
}

// The picked brain and how fast the last requests were, from /api/status (brain).
function brainStatus() {
  const line = h("div", {class: "small dim"});
  const s = ms => ms == null ? "–" : `${(ms / 1000).toFixed(1)} s`;
  const paint = b => {
    clear(line);
    if (!b) { line.textContent = "This version of the agent doesn't report it."; return; }
    if (b.brain === "openrouter") {
      line.appendChild(h("div", null, `Now: OpenRouter, ${b.model || "no model picked"}${b.session ? " (a conversation is going)" : ""}`));
      const l = b.last;
      if (l) line.appendChild(h("div", null, l.ok
        ? `Last request (${l.model}): first words after ${s(l.first_token_ms)}, done after ${s(l.total_ms)}${l.cost != null ? `, cost $${l.cost.toFixed(5)}` : ""}${l.tools && l.tools.length ? `, used ${l.tools.join(", ")}` : ""}`
        : `Last request: ${l.cancelled ? "cancelled" : `failed — ${l.error}`}`));
    } else {
      const mode = b.mode === "oneshot" ? "once per request" : `session ${b.alive ? "running" : "not running"}`;
      line.appendChild(h("div", null, `Now: Claude Code, ${mode}`));
      if (b.last) line.appendChild(h("div", null, `Last request (${b.last.mode}${b.last.warm ? ", warm" : ""}): first words after ${s(b.last.first_token_ms)}, done after ${s(b.last.total_ms)}${b.last.cancelled ? " (cancelled)" : ""}`));
    }
    for (const [m, a] of Object.entries(b.avg || {}))
      line.appendChild(h("div", null, `Average ${m} (${a.n}): first words ${s(a.first_token_ms)}, done ${s(a.total_ms)}`));
  };
  paint((store.status || {}).brain);
  if (unsubBrain) unsubBrain();
  unsubBrain = store.on((type, data) => { if (type === "status") paint(data.brain); });
  return field("Speed", line);
}

// Named after the stick, so two sticks' mics differ (same rule as stick_mic_device.slug / the firmware)
const slug = s => String(s || "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
const stickSlug = c => slug(c.stick && c.stick.name);
const autoMicName = c => stickSlug(c) ? `${stickSlug(c)}-mic-bluetooth` : "stick-mic-bluetooth";
const customMicName = c => { const n = (c.wispr_mic_name || "").trim(); return n === "stick-mic-bluetooth" ? "" : n; };
const micName = c => customMicName(c) || autoMicName(c);
const usbMicName = c => `${stickSlug(c) || "stick"}-mic-usb`;

function slider(id, label, value, min, max, step, help) {
  const out = h("span", {class: "dim", style: "min-width:64px"}, `${value} ms`);
  const input = h("input", {type: "range", id, min, max, step, value, style: "flex:1;max-width:320px",
    oninput: () => { out.textContent = `${input.value} ms`; }});
  return field(label, h("div", {class: "row"}, input, out), help);
}

// The live wireless-mic state: /api/status (wireless_mic) and the "wispr_mic" / "stick_mic" events.
let wm = null;
const wmListeners = new Set();
function wmUpdate(w) { wm = w; for (const fn of wmListeners) fn(wm); }

// "Create the device now for 60 s": for the one-time ranking in Wispr. Off while it can't exist.
function showMicButton(c) {
  const why = h("span", {class: "small dim"});
  const btn = h("button", {class: "btn", type: "button", onclick: async () => {
    btn.disabled = true;
    const r = await post("/api/wireless-mic/show", {secs: 60});
    if (report(r, `“${r.name || micName(c)}” is there for ${r.secs || 60} s — rank it first in Wispr → Settings → Microphone`)) refreshStatus();
    else btn.disabled = false;
  }}, `Show “${micName(c)}” for 60 s`);
  const paint = w => {
    const unavailable = !w ? "This version of the agent can't do it." : w.unavailable;
    const showing = w && w.device_mic && w.device_mic.showing_s;
    btn.disabled = !!unavailable || !!showing;
    why.textContent = unavailable ? `Not now: ${unavailable}.` : showing ? `There now — ${showing} s left.` : "";
  };
  wmListeners.add(paint);
  paint(wm);
  return h("div", {class: "row"}, btn, why);
}

function wirelessMicStatus() {
  const line = h("div", {class: "small", style: "display:flex;flex-direction:column;gap:6px"});
  const pill = (cls, text) => h("span", {class: `pill ${cls}`}, h("span", {class: "dot"}), text);
  const paint = w => {
    clear(line);
    if (!w) { line.appendChild(h("span", {class: "dim"}, "This version of the agent doesn't report it.")); return; }
    if (w.error) {
      line.append(h("div", null, pill("warn", "Needs attention"), " ", w.error.includes("brew")
        ? h("span", null, "Install BlackHole 2ch in a terminal: ", h("code", null, "brew install --cask blackhole-2ch"), " (asks for your Mac password).")
        : w.error));
      return;
    }
    if (!w.enabled) { line.appendChild(h("span", {class: "dim"}, "Off.")); return; }
    const d = w.device_mic || {};
    const now = w.stick_usb ? pill("", "Stick on USB — Wispr uses the stick's USB mic")
      : d.present ? pill("ok", `“${d.name}” is there${w.streaming ? " — streaming" : ""}`)
      : pill("", `“${d.name || "the stick's mic"}” appears while you hold A`);
    line.appendChild(h("div", null, now, " ", h("span", {class: "dim"}, `carried by ${w.device || "BlackHole 2ch"}`)));
    if (d.error) line.appendChild(h("div", null, pill("warn", "Mic device"), " ", d.error));
    const l = d.last;
    if (l) {
      const bits = [l.made === "created" ? `created in ${l.create_ms} ms` : l.made === "present" ? "already there" : "couldn't be created",
        `key pressed ${l.ready_ms} ms after A`];
      if (l.released_early) bits.push("released before that");
      if (l.wispr_open_ms != null) bits.push(`Wispr opened it ${l.wispr_open_ms} ms later`);
      if (l.wispr_closed_ms != null) bits.push(`closed it ${l.wispr_closed_ms} ms after the stream`);
      if (l.kept_ms != null) bits.push(`removed after ${l.kept_ms} ms`);
      line.appendChild(h("div", {class: "dim"}, `Last hold: ${bits.join(" · ")}`));
    }
    if (w.last) line.appendChild(h("div", {class: "dim"}, `Last stream: ${w.last.secs}s, ${w.last.lost} lost frame${w.last.lost === 1 ? "" : "s"}, ${w.last.underruns} gap${w.last.underruns === 1 ? "" : "s"}`));
  };
  wmListeners.add(paint);
  wmUpdate((store.status || {}).wireless_mic || null);
  if (unsub) unsub();
  unsub = store.on((type, data) => {
    if (type === "status") wmUpdate(data.wireless_mic || null);
    else if (type === "event" && data && data.kind === "wispr_mic") wmUpdate({...(wm || {}), ...data});
    else if (type === "event" && data && data.kind === "stick_mic") wmUpdate({...(wm || {}), device_mic: data});
    else if (type === "event" && data && data.kind === "link") refreshStatus();
  });
  return field("Status", line);
}

function stickPicker() {
  const s = store.status || {};
  const now = h("div", {class: "dim"}, s.stick ? `Using ${s.stick.name}` : "No stick chosen yet.");
  const found = h("div", {class: "list"});
  const msg = h("span", {class: "small dim"});
  const scan = h("button", {class: "btn", onclick: async () => {
    scan.disabled = true; msg.textContent = "Looking for sticks (5 s)…"; clear(found);
    const list = await get("/api/scan");
    scan.disabled = false;
    if (!Array.isArray(list)) { msg.textContent = list.error; return; }
    msg.textContent = list.length ? `${list.length} found, closest first` : "None found. Is the stick on and paired with this Mac?";
    for (const st of list) {
      found.appendChild(h("div", {class: "item"}, h("div", {class: "what"}, h("b", null, st.name), h("span", {class: "sub"}, `signal ${st.rssi} dBm`)),
        h("button", {class: "btn primary", onclick: async () => {
          if (report(await put("/api/config", {stick: {address: st.address, name: st.name}}), `Using ${st.name}`)) { now.textContent = `Using ${st.name}`; refreshStatus(); }
        }}, "Use this one")));
    }
  }}, "Find sticks nearby");
  return h("div", {style: "display:flex;flex-direction:column;gap:10px"}, now,
    h("p", {class: "lead", style: "margin:0"}, "Pair the stick with this Mac first (System Settings → Bluetooth). The agent only ever talks to the stick you pick here."),
    h("div", {class: "row"}, scan, msg), found);
}

async function save() {
  const v = id => document.getElementById(id);
  if (!v("model")) return;
  const brain = (document.querySelector("input[name=brain]:checked") || {}).value || "claude";
  if (brain === "openrouter" && !pickedModel()) { toast("Pick an OpenRouter model first (Settings → Brain)", "err"); return; }
  const r = await put("/api/config", {
    brain,
    openrouter_model: pickedModel(),
    model: v("model").value,
    followup_minutes: +v("followup").value,
    brain_idle_min: +v("followup").value || 10,
    brain_mode: v("brainMode").value,
    stream_replies: v("streamReplies").checked,
    speak_stream: v("speakStream").checked,
    brain_prewarm: v("brainPrewarm").checked,
    speak_replies: v("speak").checked,
    speak_output: v("speakOut").value,
    speak_max_words: +v("speakMax").value || 40,
    speak_voice: v("voice").value,
    speak_quick: v("speakQuick").checked,
    stt_model: v("sttModel").value,
    weather: v("wxOn").checked,
    weather_location: v("wxWhere").value.trim() || "auto",
    ticker: v("tickerOn").checked,
    idle_content: [...document.querySelectorAll(".idle")].filter(c => c.checked).map(c => c.value),
    stick_approvals: v("stickApprovals").checked,
    mcp_server: v("mcpServer").checked,
    wireless_mic: v("wirelessMic").checked,
    wispr_switch_ms: +v("wisprSwitch").value,
    wispr_tail_ms: +v("wisprTail").value,
    wispr_mic_name: v("wisprMicName").value.trim(),  // "" = named after the stick
    safe_tools: v("safeTools").value.split(",").map(s => s.trim()).filter(Boolean),
    claude_path: v("claudePath").value.trim(),
    update_channel: v("updChannel").value,
    update_check: v("updCheck").checked,
  });
  if (report(r, "Settings saved")) refreshStatus();
}
