//! The stick's IR remote (firmware 2.5.0; PROTOCOL.md "IR remote"), a port of `ir.py` and
//! `ir_api.py`: what's learned on the stick, learning / sending / renaming from the dashboard's
//! Remote page, voice quick commands (`{"ir": {"dev", "btn"}}`) and the stick tools `ir_list` /
//! `ir_send`.
//!
//! The stick keeps the codes (`/ir/<device>.json` in its flash) so its Remote screen works without
//! the Mac; this app asks for them with `@ir …` lines over Bluetooth. Built-in code sets
//! ([`BRANDS`]) for a few TV brands work without learning: a device can be mapped to a brand
//! (config `ir_brands`, `{"tv": "samsung"}`) or a brand's buttons put on the stick as a device.

use crate::AppState;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

pub const MIN_FIRMWARE: &str = "2.5.0";
pub const REPLY: Duration = Duration::from_secs(4);
pub const LEARN_STEP_S: u64 = 30;
pub const MAX_DEVICES: usize = 16;
pub const MAX_BUTTONS: usize = 48;

/// A built-in code set: `(id, title, [(button, code)])`. Only codes published in more than one
/// well-known place (the sources are in ir.py). The code strings are the stick's
/// (`logic/ir_codec.h`): `<protocol>:<address hex>:<command hex>`.
pub struct Brand {
    pub id: &'static str,
    pub title: &'static str,
    pub buttons: &'static [(&'static str, &'static str)],
}

pub const BRANDS: &[Brand] = &[
    // Samsung32, address 0x07 (LIRC samsung/BN59-00603A, Flipper-IRDB TVs/Samsung)
    Brand {
        id: "samsung",
        title: "Samsung TV",
        buttons: &[
            ("power", "samsung:7:2"),
            ("volume up", "samsung:7:7"),
            ("volume down", "samsung:7:b"),
            ("mute", "samsung:7:f"),
            ("input", "samsung:7:1"),
            ("channel up", "samsung:7:12"),
            ("channel down", "samsung:7:10"),
        ],
    },
    // NEC, address 0x04 (LIRC lg/AKB72915207, Flipper-IRDB TVs/LG)
    Brand {
        id: "lg",
        title: "LG TV",
        buttons: &[
            ("power", "nec:4:8"),
            ("volume up", "nec:4:2"),
            ("volume down", "nec:4:3"),
            ("mute", "nec:4:9"),
            ("input", "nec:4:b"),
            ("channel up", "nec:4:0"),
            ("channel down", "nec:4:1"),
        ],
    },
    // SIRC 12-bit, device 1 (SB-Projects, the JP1 Sony TV table)
    Brand {
        id: "sony",
        title: "Sony TV",
        buttons: &[
            ("power", "sony12:1:15"),
            ("volume up", "sony12:1:12"),
            ("volume down", "sony12:1:13"),
            ("mute", "sony12:1:14"),
            ("input", "sony12:1:25"),
            ("channel up", "sony12:1:10"),
            ("channel down", "sony12:1:11"),
        ],
    },
    // RC5 system 0 (SB-Projects' RC-5 table, Wikipedia RC-5)
    Brand {
        id: "philips",
        title: "Philips TV (RC5)",
        buttons: &[
            ("power", "rc5:0:c"),
            ("volume up", "rc5:0:10"),
            ("volume down", "rc5:0:11"),
            ("mute", "rc5:0:d"),
            ("channel up", "rc5:0:20"),
            ("channel down", "rc5:0:21"),
            ("input", "rc5:0:38"),
        ],
    },
];

pub fn brand(id: &str) -> Option<&'static Brand> {
    BRANDS.iter().find(|b| b.id == id)
}

impl Brand {
    pub fn code(&self, button: &str) -> Option<&'static str> {
        self.buttons.iter().find(|(n, _)| *n == button).map(|(_, c)| *c)
    }
}

/// The default voice phrases (commands.json); "tv" works with a learned "tv" remote or a brand set.
pub fn voice_commands() -> Vec<Value> {
    vec![
        json!({"name": "TV power", "phrases": ["turn off the tv", "turn on the tv", "tv off", "tv on", "tv power",
                                              "switch off the tv", "switch on the tv"], "ir": {"dev": "tv", "btn": "power"}}),
        json!({"name": "TV volume up", "phrases": ["tv volume up", "turn the tv up", "louder"],
               "ir": {"dev": "tv", "btn": "volume up", "repeat": 2}}),
        json!({"name": "TV volume down", "phrases": ["tv volume down", "turn the tv down", "quieter"],
               "ir": {"dev": "tv", "btn": "volume down", "repeat": 2}}),
        json!({"name": "TV mute", "phrases": ["mute the tv", "unmute the tv", "tv mute"], "ir": {"dev": "tv", "btn": "mute"}}),
    ]
}

static CODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:(?:nec|samsung|lg|sony|sony12|sony15|sony20|rc5|rc6|panasonic):[0-9a-fA-F]{1,8}:[0-9a-fA-F]{1,8}|kaseikyo:[0-9a-fA-F]{1,4}:[0-9a-fA-F]{1,3}:[0-9a-fA-F]{1,2}|raw:[0-9]{2}:[0-9,]{1,4000}(?::[0-9a-z]{1,800})?)$",
    )
    .expect("the code pattern")
});

/// Like the stick's `irnames::deviceId`: "Living Room TV" -> "living-room-tv" ("" if nothing's left).
pub fn device_id(name: &str) -> String {
    let mut s = String::new();
    let mut sep = false;
    for c in name.to_lowercase().chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            sep = true;
            continue;
        }
        if sep {
            s.push('-');
            sep = false;
        }
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            s.push(c);
        }
    }
    if sep {
        s.push('-');
    }
    // [^a-z0-9-] removed above; now strip the dashes at the ends (and runs the removal made)
    let s = s.trim_matches('-');
    let cut: String = s.chars().take(16).collect();
    cut.trim_end_matches('-').to_string()
}

/// Like `irnames::buttonName`: "  Volume  Up " -> "volume up".
pub fn button_name(name: &str) -> String {
    let kept: String = name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || " +-_.#\t".contains(*c))
        .collect();
    let joined = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(24).collect::<String>().trim().to_string()
}

pub fn valid_code(code: &str) -> bool {
    CODE_RE.is_match(code)
}

/// Python's `urllib.parse.quote(v, safe="")`.
fn quote(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `@ir op=send&dev=tv&btn=volume%20up` (the &-form: values may have spaces); empty values left out.
pub fn line(op: &str, args: &[(&str, String)]) -> String {
    let mut parts = vec![format!("op={op}")];
    parts.extend(args.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}={}", quote(v))));
    format!("@ir {}", parts.join("&"))
}

/// `@state` -> `(supported, why)`.
pub fn supported(state: Option<&Value>) -> (bool, Option<String>) {
    match state {
        None | Some(Value::Null) => (false, Some("the stick didn't answer".into())),
        Some(s) if s.get("ir").is_some() => (true, None),
        Some(s) => {
            let fw = s.get("fw").and_then(Value::as_str).filter(|f| !f.is_empty()).unwrap_or("?");
            (false, Some(format!("the stick's firmware ({fw}) has no IR remote: it needs {MIN_FIRMWARE}")))
        }
    }
}

/// Adds the IR voice phrases to an existing commands.json once (new installs get them from the
/// defaults). Returns whether it ran (config `ir_voice_added` is set then).
pub fn ensure_voice_commands(st: &AppState) -> bool {
    if st.cfg.lock().unwrap().get("ir_voice_added").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    let path = &st.commands.path;
    let Some(Value::Object(mut doc)) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())
    else {
        return false;
    };
    let cmds = doc.entry("commands").or_insert_with(|| json!([]));
    if let Some(list) = cmds.as_array_mut()
        && !list.iter().any(|c| c.get("ir").is_some())
    {
        list.extend(voice_commands());
        match serde_json::to_string_pretty(&doc) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    log::warn!("commands.json: {e}");
                }
            }
            Err(e) => log::warn!("commands.json: {e}"),
        }
    }
    let changes: Map<String, Value> = [("ir_voice_added".to_string(), json!(true))].into_iter().collect();
    if let Err(e) = st.update_config(changes) {
        log::warn!("ir: {e}");
    }
    true
}

/// The app's side: the last learn's state as the stick reported it, and its `ir` events.
pub struct Ir {
    pub learn: Mutex<Option<Value>>,
    events: broadcast::Sender<Value>,
}

impl Default for Ir {
    fn default() -> Self {
        Self { learn: Mutex::new(None), events: broadcast::channel(32).0 }
    }
}

/// The stick's `{"evt":"ir","state":…}` messages become `ir` events for the Remote page (and the
/// learn's state). Called once at start (tests call it themselves).
pub fn spawn(st: Arc<AppState>) {
    let mut msgs = st.link.messages();
    tokio::spawn(async move {
        loop {
            match msgs.recv().await {
                Ok(m) if m["src"] == "agent" && m["evt"] == "ir" => on_message(&st, &m),
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
    });
}

fn on_message(st: &AppState, m: &Value) {
    if m.get("state").is_some_and(|s| !s.is_null()) {
        let mut d = m.as_object().cloned().unwrap_or_default();
        d.remove("src");
        d.remove("evt");
        *st.ir.learn.lock().unwrap() = Some(Value::Object(d.clone()));
        st.hub.emit("ir", Value::Object(d));
    }
    let _ = st.ir.events.send(m.clone());
}

/// A protocol line over Bluetooth -> its reply (None: no answer / not connected).
pub async fn ask(st: &AppState, text: &str) -> Option<Value> {
    if !st.link.connected() {
        return None;
    }
    st.link.request_timeout(text, REPLY).await
}

/// `@ir <op>` -> the reply; `Err` with the stick's words.
pub async fn call(st: &AppState, op: &str, args: &[(&str, String)]) -> Result<Value, String> {
    if !st.link.connected() {
        return Err("the stick isn't connected".into());
    }
    let r = ask(st, &line(op, args)).await.ok_or("the stick didn't answer")?;
    if !r.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let error = r.get("error").and_then(Value::as_str).unwrap_or("");
        if r.get("code").and_then(Value::as_str) == Some("op") || error.contains("Unknown command") {
            return Err(format!("the stick's firmware has no IR remote: it needs {MIN_FIRMWARE}"));
        }
        return Err(if error.is_empty() { "the stick refused".into() } else { error.into() });
    }
    Ok(r)
}

fn s(v: &str) -> String {
    v.to_string()
}

/// `{"devices": [{"id", "n", "fav", "buttons"?}], "max", "free"}`
pub async fn devices(st: &AppState, with_buttons: bool) -> Result<Value, String> {
    let r = call(st, "list", &[]).await?;
    let mut out = Vec::new();
    for d in r.get("devices").and_then(Value::as_array).cloned().unwrap_or_default() {
        let id = d.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let mut e = json!({"id": id, "n": d.get("n").cloned().unwrap_or(json!(0)),
                           "fav": d.get("fav").cloned().unwrap_or(json!(0))});
        if with_buttons {
            e["buttons"] = Value::Array(buttons(st, &id).await?);
        }
        out.push(e);
    }
    Ok(json!({"devices": out, "max": r.get("max").cloned().unwrap_or(Value::Null),
              "free": r.get("free").cloned().unwrap_or(Value::Null)}))
}

/// A remote's buttons, every page: `[{"name", "proto", "fav"}]`.
pub async fn buttons(st: &AppState, dev: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for page in 0..10 {
        let r = call(st, "list", &[("dev", s(dev)), ("page", page.to_string())]).await?;
        for b in r.get("buttons").and_then(Value::as_array).into_iter().flatten() {
            let fav = b.get("f").is_some_and(|f| f.as_i64().unwrap_or(0) != 0 || f.as_bool() == Some(true));
            out.push(json!({"name": b.get("n").cloned().unwrap_or(Value::Null),
                            "proto": b.get("p").cloned().unwrap_or(Value::Null), "fav": fav}));
        }
        if !r.get("more").and_then(Value::as_bool).unwrap_or(false) {
            break;
        }
    }
    Ok(out)
}

pub fn brand_for(st: &AppState, dev: &str) -> Option<String> {
    let cfg = st.cfg.lock().unwrap();
    cfg.get("ir_brands").and_then(|b| b.get(device_id(dev))).and_then(Value::as_str).map(str::to_string)
}

fn clamp_repeat(r: i64) -> String {
    r.clamp(0, 20).to_string()
}

/// Sends a learned button, or the device's brand code. -> a short line for the stick / Claude.
pub async fn send(st: &AppState, dev: &str, btn: &str, repeat: i64) -> Result<String, String> {
    let (dev_id, name) = (device_id(dev), button_name(btn));
    if dev_id.is_empty() || name.is_empty() {
        return Err("say which device and which button".into());
    }
    let rep = clamp_repeat(repeat);
    match call(st, "send", &[("dev", dev_id.clone()), ("btn", name.clone()), ("repeat", rep.clone())]).await {
        Ok(_) => return Ok(format!("{dev_id}: {name} sent")),
        Err(e) if !e.contains("No such") => return Err(e),
        Err(_) => {}
    }
    let Some(b) = brand_for(st, &dev_id) else {
        return Err(format!(
            "no '{name}' button on the '{dev_id}' remote (learn it, or pick a brand for '{dev_id}' in the dashboard's Remote page)"
        ));
    };
    let Some(code) = brand(&b).and_then(|br| br.code(&name)) else {
        let title = brand(&b).map(|br| br.title.to_string()).unwrap_or(b);
        return Err(format!("the {title} code set has no '{name}' button"));
    };
    call(st, "send", &[("code", s(code)), ("repeat", rep)]).await?;
    Ok(format!("{dev_id}: {name} sent"))
}

pub async fn send_code(st: &AppState, code: &str, repeat: i64) -> Result<String, String> {
    if !valid_code(code) {
        return Err("not a code the stick can send".into());
    }
    call(st, "send", &[("code", s(code)), ("repeat", clamp_repeat(repeat))]).await?;
    Ok("sent".into())
}

pub async fn start_learn(st: &AppState, dev: &str, btn: &str, once: bool) -> Result<Value, String> {
    let dev_id = Some(device_id(dev)).filter(|d| !d.is_empty()).unwrap_or_else(|| "remote".into());
    let name = button_name(btn);
    if !btn.is_empty() && name.is_empty() {
        return Err("button names: letters, digits, spaces, + - _ . #".into());
    }
    let once = if once { s("1") } else { String::new() };
    let r = call(st, "learn", &[("dev", dev_id.clone()), ("btn", name.clone()), ("once", once)]).await?;
    let learn = json!({"state": "learning", "dev": r.get("dev").cloned().unwrap_or(json!(dev_id)),
                       "btn": r.get("btn").cloned().unwrap_or(json!(name)), "step": 1});
    *st.ir.learn.lock().unwrap() = Some(learn.clone());
    st.hub.emit("ir", learn.clone());
    Ok(learn)
}

/// Starts a learn and waits for its end -> the final event (saved / timeout / error / cancelled).
pub async fn learn_and_wait(
    st: &AppState,
    dev: &str,
    btn: &str,
    once: bool,
    timeout: Duration,
) -> Result<Value, String> {
    let mut rx = st.ir.events.subscribe();
    start_learn(st, dev, btn, once).await?;
    tokio::time::timeout(timeout, async {
        loop {
            match rx.recv().await {
                Ok(m) if matches!(m["state"].as_str(), Some("saved" | "timeout" | "error" | "cancelled")) => {
                    return Ok(m);
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return Err("stopped".to_string()),
            }
        }
    })
    .await
    .map_err(|_| "the stick didn't finish learning".to_string())?
}

pub async fn cancel(st: &AppState) -> Result<Value, String> {
    call(st, "cancel", &[]).await
}

pub async fn rename(st: &AppState, dev: &str, to: &str, btn: Option<&str>) -> Result<Value, String> {
    match btn.filter(|b| !b.is_empty()) {
        Some(b) => call(st, "rename", &[("dev", device_id(dev)), ("btn", button_name(b)), ("to", s(to))]).await,
        None => call(st, "rename", &[("dev", device_id(dev)), ("to", s(to))]).await,
    }
}

pub async fn remove(st: &AppState, dev: &str, btn: Option<&str>) -> Result<Value, String> {
    let btn = btn.filter(|b| !b.is_empty()).map(button_name).unwrap_or_default();
    call(st, "rm", &[("dev", device_id(dev)), ("btn", btn)]).await
}

pub async fn fav(st: &AppState, dev: &str, btn: &str, on: bool) -> Result<Value, String> {
    call(st, "fav", &[("dev", device_id(dev)), ("btn", button_name(btn)), ("on", s(if on { "1" } else { "0" }))]).await
}

pub async fn put(st: &AppState, dev: &str, btn: &str, code: &str, fav: bool) -> Result<Value, String> {
    if !valid_code(code) {
        return Err(format!("'{btn}': not a code the stick can send"));
    }
    let f = s(if fav { "1" } else { "0" });
    call(st, "put", &[("dev", device_id(dev)), ("btn", button_name(btn)), ("code", s(code)), ("fav", f)]).await
}

/// Puts a brand's buttons on the stick as remote `dev`.
pub async fn install_brand(st: &AppState, dev: &str, brand_id: &str) -> Result<Value, String> {
    let b = brand(brand_id).ok_or_else(|| format!("unknown code set: {brand_id}"))?;
    for (name, code) in b.buttons {
        put(st, dev, name, code, *name == "power").await?;
    }
    Ok(json!({"dev": device_id(dev), "n": b.buttons.len()}))
}

/// Everything learned, codes included: `{"stickybot_ir": 1, "devices": {id: [{n, c, f?}]}}`.
pub async fn export(st: &AppState) -> Result<Value, String> {
    let mut out = Map::new();
    let d = devices(st, true).await?;
    for dev in d["devices"].as_array().into_iter().flatten() {
        let id = dev["id"].as_str().unwrap_or("").to_string();
        let mut rows = Vec::new();
        for b in dev["buttons"].as_array().into_iter().flatten() {
            let name = b["name"].as_str().unwrap_or("").to_string();
            let r = call(st, "get", &[("dev", id.clone()), ("btn", name.clone())]).await?;
            let mut row = json!({"n": name, "c": r.get("c").cloned().unwrap_or(Value::Null)});
            if b["fav"] == json!(true) {
                row["f"] = json!(1);
            }
            rows.push(row);
        }
        out.insert(id, Value::Array(rows));
    }
    Ok(json!({"stickybot_ir": 1, "devices": out}))
}

/// The export's format back onto the stick (buttons with the same name are replaced).
/// -> `{"added": n, "skipped": [why…]}`
pub async fn import_doc(st: &AppState, doc: &Value) -> Result<Value, String> {
    let devs = doc.get("devices").and_then(Value::as_object).filter(|d| !d.is_empty());
    let Some(devs) = devs else {
        return Err(r#"expected {"devices": {"tv": [{"n": "power", "c": "nec:4:8"}]}}"#.into());
    };
    if devs.len() > MAX_DEVICES {
        return Err(format!("at most {MAX_DEVICES} remotes"));
    }
    let (mut added, mut skipped) = (0, Vec::<String>::new());
    for (dev, rows) in devs {
        let Some(rows) = rows.as_array().filter(|_| !device_id(dev).is_empty()) else {
            skipped.push(format!("{dev}: not a remote"));
            continue;
        };
        for row in rows.iter().take(MAX_BUTTONS) {
            let Some(row) = row.as_object() else { continue };
            let n = row.get("n").and_then(Value::as_str).unwrap_or("");
            let name = button_name(n);
            let code = row.get("c").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() || !valid_code(code) {
                let shown = row.get("n").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
                skipped.push(format!("{dev}/{}: bad name or code", shown.unwrap_or_else(|| "None".into())));
                continue;
            }
            let fav = row.get("f").is_some_and(|f| f.as_bool().unwrap_or(f.as_i64().unwrap_or(0) != 0));
            match put(st, dev, &name, code, fav).await {
                Ok(_) => added += 1,
                Err(e) => skipped.push(format!("{dev}/{name}: {e}")),
            }
        }
    }
    Ok(json!({"added": added, "skipped": skipped}))
}

pub async fn diag(st: &AppState) -> Result<Value, String> {
    call(st, "diag", &[]).await
}

fn capital(e: &str) -> String {
    let mut c = e.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// A quick command's `{"ir": {"dev", "btn", "repeat"}}` -> `(ok, line)`.
pub async fn voice(st: &AppState, spec: &Value) -> (bool, String) {
    let g = |k: &str| spec.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match send(st, &g("dev"), &g("btn"), int(spec.get("repeat"))).await {
        Ok(t) => (true, t),
        Err(e) => (false, capital(&e)),
    }
}

/// For `ir_list`: the remotes and their buttons (learned on the stick, plus brand sets mapped to
/// devices), in the stick's order.
pub async fn listing(st: &AppState) -> Result<Vec<(String, Vec<String>)>, String> {
    let have = devices(st, true).await?;
    let mut out: Vec<(String, Vec<String>)> = have["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let names =
                d["buttons"].as_array().into_iter().flatten().filter_map(|b| b["name"].as_str()).map(s).collect();
            (d["id"].as_str().unwrap_or("").to_string(), names)
        })
        .collect();
    let mapped = st.cfg.lock().unwrap().get("ir_brands").and_then(Value::as_object).cloned().unwrap_or_default();
    for (dev, b) in mapped {
        let Some(br) = b.as_str().and_then(brand) else { continue };
        let i = match out.iter().position(|(d, _)| *d == dev) {
            Some(i) => i,
            None => {
                out.push((dev.clone(), Vec::new()));
                out.len() - 1
            }
        };
        for (n, _) in br.buttons {
            if !out[i].1.iter().any(|x| x == n) {
                out[i].1.push(n.to_string());
            }
        }
    }
    Ok(out)
}

/// `int(v or 0)`: a number or a numeric string.
fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Some(Value::String(t)) => t.trim().parse().unwrap_or(0),
        Some(Value::Bool(b)) => *b as i64,
        _ => 0,
    }
}

// --- the dashboard API (ir_api.py) ------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/ir", get(api_all))
        .route("/api/ir/send", post(api_send))
        .route("/api/ir/learn", post(api_learn))
        .route("/api/ir/learn/cancel", post(api_learn_cancel))
        .route("/api/ir/rename", post(api_rename))
        .route("/api/ir/remove", post(api_remove))
        .route("/api/ir/fav", post(api_fav))
        .route("/api/ir/brand", post(api_brand))
        .route("/api/ir/export", get(api_export))
        .route("/api/ir/import", post(api_import))
        .route("/api/ir/diag", get(api_diag))
}

fn err(text: &str, status: StatusCode) -> Response {
    (status, axum::Json(json!({"error": capital(text)}))).into_response()
}

fn body(b: &Bytes) -> Map<String, Value> {
    match serde_json::from_slice::<Value>(b) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// `{"ok": true, ...result}` or 409 with the words.
fn done(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => {
            let mut m = Map::new();
            m.insert("ok".into(), json!(true));
            if let Value::Object(o) = v {
                m.extend(o);
            }
            axum::Json(Value::Object(m)).into_response()
        }
        Err(e) => err(&e, StatusCode::CONFLICT),
    }
}

fn result(r: Result<String, String>) -> Result<Value, String> {
    r.map(|t| json!({"result": t}))
}

fn text_of(b: &Map<String, Value>, k: &str) -> String {
    match b.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

async fn api_all(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let brands: Map<String, Value> = BRANDS
        .iter()
        .map(|b| {
            (
                b.id.to_string(),
                json!({"title": b.title, "buttons": b.buttons.iter().map(|(n, _)| n).collect::<Vec<_>>()}),
            )
        })
        .collect();
    let mapped = st.cfg.lock().unwrap().get("ir_brands").cloned().filter(Value::is_object).unwrap_or(json!({}));
    let learn = st.ir.learn.lock().unwrap().clone();
    let mut out = json!({"connected": st.link.connected(), "supported": false, "why": null, "fw": null, "devices": [],
                         "brands": brands, "mapped": mapped, "learn": learn});
    if !st.link.connected() {
        out["why"] = json!("the stick isn't connected by Bluetooth");
        return axum::Json(out);
    }
    let state = ask(&st, "@state").await;
    out["fw"] = state.as_ref().and_then(|s| s.get("fw")).cloned().unwrap_or(Value::Null);
    let (ok, why) = supported(state.as_ref());
    out["supported"] = json!(ok);
    out["why"] = json!(why);
    if ok {
        match devices(&st, true).await {
            Ok(Value::Object(d)) => {
                for (k, v) in d {
                    out[k] = v;
                }
            }
            Ok(_) => {}
            Err(e) => out["why"] = json!(e),
        }
    }
    axum::Json(out)
}

async fn api_send(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    let repeat = int(b.get("repeat"));
    if truthy(b.get("code")) {
        return done(result(send_code(&st, &text_of(&b, "code"), repeat).await));
    }
    if truthy(b.get("brand")) {
        let code = brand(&text_of(&b, "brand")).and_then(|br| br.code(&button_name(&text_of(&b, "btn"))));
        let Some(code) = code else { return err("no such button in that code set", StatusCode::NOT_FOUND) };
        return done(result(send_code(&st, code, repeat).await));
    }
    done(result(send(&st, &text_of(&b, "dev"), &text_of(&b, "btn"), repeat).await))
}

async fn api_learn(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    done(start_learn(&st, &text_of(&b, "dev"), &text_of(&b, "btn"), truthy(b.get("once"))).await)
}

async fn api_learn_cancel(State(st): State<Arc<AppState>>) -> Response {
    done(cancel(&st).await)
}

async fn api_rename(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    if !truthy(b.get("dev")) || !truthy(b.get("to")) {
        return err("say what to rename and to what", StatusCode::BAD_REQUEST);
    }
    let btn = text_of(&b, "btn");
    done(rename(&st, &text_of(&b, "dev"), &text_of(&b, "to"), Some(&btn)).await)
}

async fn api_remove(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    if !truthy(b.get("dev")) {
        return err("which remote?", StatusCode::BAD_REQUEST);
    }
    let btn = text_of(&b, "btn");
    done(remove(&st, &text_of(&b, "dev"), Some(&btn)).await)
}

async fn api_fav(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    done(fav(&st, &text_of(&b, "dev"), &text_of(&b, "btn"), truthy(b.get("on"))).await)
}

/// `{"dev","brand","install"}`: maps a remote to a code set (voice, ir_send); with `install` its
/// buttons go on the stick; brand "" unmaps.
async fn api_brand(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    let (dev, name) = (device_id(&text_of(&b, "dev")), text_of(&b, "brand"));
    if dev.is_empty() {
        return err("which remote?", StatusCode::BAD_REQUEST);
    }
    if !name.is_empty() && brand(&name).is_none() {
        return err("unknown code set", StatusCode::NOT_FOUND);
    }
    let mut mapped = st.cfg.lock().unwrap().get("ir_brands").and_then(Value::as_object).cloned().unwrap_or_default();
    if name.is_empty() {
        mapped.remove(&dev);
    } else {
        mapped.insert(dev.clone(), json!(name));
    }
    let changes: Map<String, Value> = [("ir_brands".to_string(), Value::Object(mapped.clone()))].into_iter().collect();
    if let Err(e) = st.update_config(changes) {
        return err(&e, StatusCode::INTERNAL_SERVER_ERROR);
    }
    if !name.is_empty() && truthy(b.get("install")) {
        return done(install_brand(&st, &dev, &name).await);
    }
    axum::Json(json!({"ok": true, "mapped": mapped})).into_response()
}

async fn api_export(State(st): State<Arc<AppState>>) -> Response {
    match export(&st).await {
        Ok(doc) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CONTENT_DISPOSITION, "attachment; filename=\"stickybot-remotes.json\""),
            ],
            serde_json::to_string_pretty(&doc).unwrap_or_default(),
        )
            .into_response(),
        Err(e) => err(&e, StatusCode::CONFLICT),
    }
}

async fn api_import(State(st): State<Arc<AppState>>, b: Bytes) -> Response {
    let b = body(&b);
    let doc = b.get("doc").cloned().unwrap_or_else(|| Value::Object(b.clone()));
    done(import_doc(&st, &doc).await)
}

async fn api_diag(State(st): State<Arc<AppState>>) -> Response {
    done(diag(&st).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_sticks() {
        assert_eq!(device_id("  Living Room_TV "), "living-room-tv");
        assert_eq!(device_id("AC!"), "ac");
        assert_eq!(device_id("a very long device name indeed"), "a-very-long-devi");
        assert_eq!(device_id(" -- "), "");
        assert_eq!(device_id("tv"), "tv");
        assert_eq!(button_name("  Volume   Up "), "volume up");
        assert_eq!(button_name("Cool 24°C"), "cool 24c");
        assert_eq!(
            line("send", &[("dev", s("tv")), ("btn", s("volume up")), ("repeat", s("0"))]),
            "@ir op=send&dev=tv&btn=volume%20up&repeat=0"
        );
        assert_eq!(line("learn", &[("dev", s("tv")), ("btn", s("")), ("once", s(""))]), "@ir op=learn&dev=tv");
    }

    #[test]
    fn codes() {
        for good in [
            "nec:4:8",
            "samsung:7:2",
            "sony12:1:15",
            "rc5:0:c",
            "kaseikyo:2002:8:3d",
            "panasonic:8:3d",
            "raw:38:9000,4500,560",
            "raw:38:3400,1700,420:0120",
        ] {
            assert!(valid_code(good), "{good}");
        }
        for bad in ["", "nec:4", "tv:1:2", "raw:38:", "nec:4:8; rm -rf ~", "raw:38:1,2:ABC"] {
            assert!(!valid_code(bad), "{bad}");
        }
        for b in BRANDS {
            // every built-in code is one the stick reads
            assert!(b.buttons.iter().all(|(_, c)| valid_code(c)));
            for need in ["power", "volume up", "volume down", "mute"] {
                assert!(b.code(need).is_some(), "{} {need}", b.id);
            }
        }
    }

    #[test]
    fn support_from_the_state() {
        assert_eq!(supported(Some(&json!({"fw": "2.5.0", "ir": {"v": 1}}))), (true, None));
        assert_eq!(
            supported(Some(&json!({"fw": "2.4.0"}))),
            (false, Some("the stick's firmware (2.4.0) has no IR remote: it needs 2.5.0".into()))
        );
        assert_eq!(supported(None).1.as_deref(), Some("the stick didn't answer"));
    }

    #[test]
    fn the_default_commands_have_the_tv_phrases() {
        let d: Value = serde_json::from_str(crate::commands::DEFAULTS).unwrap();
        let cmds = d["commands"].as_array().unwrap();
        assert!(
            cmds.iter()
                .filter(|c| c.get("ir").is_some())
                .any(|c| c["phrases"].as_array().unwrap().contains(&json!("turn off the tv")))
        );
        // the same as ours (Python's VOICE_COMMANDS)
        let ours: Vec<Value> = cmds.iter().filter(|c| c.get("ir").is_some()).cloned().collect();
        assert_eq!(ours, voice_commands());
    }
}
