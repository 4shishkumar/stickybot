// Settings: the agent's own options (saved in ~/.stick-agent/config.json).
import {h, clear, card, field, toggle, get, put, post, report, toast, refreshStatus, store} from "../lib.js";
import {componentsPanel} from "../components.js";

export const title = "Settings";
export const subtitle = "How the agent behaves. Changes apply right away once you save.";

const SOURCES = [["facts", "Fun facts"], ["hn", "Hacker News"], ["onthisday", "On this day"], ["wikipedia", "Wikipedia featured article"]];
let root = null, cfg = null, unsub = null;

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
    card("Claude",
      field("Model", h("select", {id: "model"}, [["sonnet", "Sonnet — fast (recommended)"], ["opus", "Opus — smartest, slower"], ["haiku", "Haiku — fastest"]]
        .map(([v, l]) => h("option", {value: v, selected: c.model === v}, l)))),
      field("Follow-ups", h("div", {class: "row"}, h("input", {type: "number", id: "followup", min: 0, max: 120, value: c.followup_minutes ?? 10, style: "width:90px"}), h("span", {class: "dim"}, "minutes")),
        "A request within this time continues the same conversation.")),
    card("Speech & replies",
      field("Read replies aloud", toggle("speak", "Short replies are spoken; long ones stay on the screen", c.speak_replies)),
      field("Play on", h("select", {id: "speakOut"}, [["stick", "the stick's speaker"], ["mac", "the Mac's speakers"]].map(([v, l]) => h("option", {value: v, selected: (c.speak_output || "stick") === v}, l)))),
      field("Short means", h("div", {class: "row"}, h("span", {class: "dim"}, "up to"), h("input", {type: "number", id: "speakMax", min: 5, max: 300, value: c.speak_max_words || 40, style: "width:90px"}), h("span", {class: "dim"}, "words"))),
      field("Voice", h("div", {class: "row"}, voice, h("button", {class: "btn", onclick: async () => {
        await put("/api/config", {speak_voice: voice.value, speak_output: document.getElementById("speakOut").value});
        report(await post("/api/voice/test", {}), "Playing a sample");
      }}, "Test")), "Better voices: System Settings → Accessibility → Spoken Content → System voice → Manage voices."),
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
        "The agent plays it into the BlackHole 2ch virtual mic. Set Wispr Flow's microphone to “BlackHole 2ch”. On USB the stick is a normal USB mic as before."),
      wirelessMicStatus()),
    card("Approvals",
      field("Stick approvals", toggle("stickApprovals", "Claude Code's permission prompts go to the stick and here first", c.stick_approvals !== false),
        "Off: Claude Code asks in the terminal straight away. On: it waits up to 30 s for A / B.")),
    card("Advanced",
      field("Run without asking", h("input", {id: "safeTools", class: "mono", value: (c.safe_tools || []).join(", ")}), "Claude Code tools that never need approval."),
      field("Claude path", h("input", {id: "claudePath", class: "mono", placeholder: "found automatically", value: c.claude_path || ""}))),
    card("Components", componentsPanel()),
  );
}

export function leave() { root = null; if (unsub) unsub(); unsub = null; }

// Device found / streaming, from /api/status (wireless_mic) and live "wispr_mic" events.
function wirelessMicStatus() {
  const line = h("div", {class: "small"});
  const paint = w => {
    clear(line);
    if (!w) { line.appendChild(h("span", {class: "dim"}, "This version of the agent doesn't report it.")); return; }
    if (w.error) {
      line.append(h("span", {class: "pill warn"}, h("span", {class: "dot"}), "Needs attention"), " ", w.error.includes("brew")
        ? h("span", null, "Install BlackHole 2ch in a terminal: ", h("code", null, "brew install --cask blackhole-2ch"), " (asks for your Mac password).")
        : w.error);
      return;
    }
    if (!w.enabled) { line.appendChild(h("span", {class: "dim"}, "Off.")); return; }
    const parts = [h("span", {class: "pill ok"}, h("span", {class: "dot"}), w.streaming ? "Streaming to Wispr" : "Ready"),
      " ", h("span", {class: "dim"}, `Playing into ${w.device || "BlackHole 2ch"}`)];
    if (w.last) parts.push(h("span", {class: "dim"}, ` · last: ${w.last.secs}s, ${w.last.lost} lost frame${w.last.lost === 1 ? "" : "s"}, ${w.last.underruns} gap${w.last.underruns === 1 ? "" : "s"}`));
    line.append(...parts);
  };
  paint((store.status || {}).wireless_mic);
  if (unsub) unsub();
  unsub = store.on((type, data) => {
    if (type === "status") paint(data.wireless_mic);
    else if (type === "event" && data && data.kind === "wispr_mic") paint(data);
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
  const r = await put("/api/config", {
    model: v("model").value,
    followup_minutes: +v("followup").value,
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
    wireless_mic: v("wirelessMic").checked,
    safe_tools: v("safeTools").value.split(",").map(s => s.trim()).filter(Boolean),
    claude_path: v("claudePath").value.trim(),
  });
  if (report(r, "Settings saved")) refreshStatus();
}
