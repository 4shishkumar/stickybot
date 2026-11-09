//! Settings and files, in `~/.stickybot-rs/` (never the Python agent's `~/.stick-agent/`).

use serde_json::{Map, Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 8670;

/// Where the Rust app keeps its files: `$STICKYBOT_RS_HOME`, else `~/.stickybot-rs`.
pub fn default_home() -> PathBuf {
    if let Some(h) = std::env::var_os("STICKYBOT_RS_HOME") {
        return PathBuf::from(h);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".stickybot-rs")
}

/// The Python agent's folder. The Rust app must never read or write it.
fn python_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".stick-agent"))
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    /// Refuses the Python agent's folder (or anything inside it).
    pub fn new(home: impl Into<PathBuf>) -> Result<Self, String> {
        let home: PathBuf = home.into();
        if let Some(py) = python_home() {
            let norm = |p: &Path| p.components().collect::<PathBuf>();
            if norm(&home).starts_with(norm(&py)) {
                return Err(format!(
                    "{} belongs to the Python agent; the Rust app uses its own folder",
                    home.display()
                ));
            }
        }
        Ok(Self { home })
    }
    pub fn config(&self) -> PathBuf {
        self.home.join("config.json")
    }
    pub fn token(&self) -> PathBuf {
        self.home.join("token")
    }
    pub fn log(&self) -> PathBuf {
        self.home.join("app.log")
    }
    pub fn history(&self) -> PathBuf {
        self.home.join("history.jsonl")
    }
    pub fn commands(&self) -> PathBuf {
        self.home.join("commands.json")
    }
    /// The speech model (and later others): `$STICKYBOT_RS_MODELS`, else `<home>/models`.
    pub fn models(&self) -> PathBuf {
        match std::env::var_os("STICKYBOT_RS_MODELS") {
            Some(m) => PathBuf::from(m),
            None => self.home.join("models"),
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.home)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.home, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// The dashboard API secret (made on first start, 0600): the page has it built in.
    pub fn token_value(&self) -> std::io::Result<String> {
        let p = self.token();
        if let Ok(t) = fs::read_to_string(&p) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                return Ok(t);
            }
        }
        let t = new_token();
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&p)?.write_all(t.as_bytes())?;
        Ok(t)
    }
}

/// Like Python's `secrets.token_urlsafe(24)`.
pub fn new_token() -> String {
    use base64::Engine;
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).expect("system randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// The Python agent's `DEFAULT_CONFIG` (config.py), so the Settings page finds every key it shows.
/// Most of them do nothing in the Rust app yet (Phase 2+).
pub fn defaults() -> Map<String, Value> {
    let v = json!({
        "stick": null,
        "model": "sonnet",
        "stt_model": "mlx-community/parakeet-tdt-0.6b-v2",
        "stt_hints": ["Tauri", "StickyBot", "Claude"],
        "claude_path": "",
        "speak_replies": false,
        "speak_output": "stick",
        "speak_max_words": 40,
        "speak_voice": "",
        "speak_quick": false,
        "followup_minutes": 10,
        "brain_mode": "session",
        "brain_prewarm": true,
        "brain_idle_min": 10,
        "brain": "claude",
        "openrouter_model": "",
        "openrouter_max_tokens": 400,
        "stream_replies": true,
        "stream_interval_ms": 300,
        "speak_stream": true,
        "safe_tools": ["WebSearch", "WebFetch", "Read", "Glob", "Grep"],
        "weather": true,
        "weather_location": "auto",
        "ticker": true,
        "idle_content": ["facts", "hn", "onthisday", "wikipedia"],
        "onboarding": {},
        "install_skipped": [],
        "stick_approvals": true,
        "mcp_server": true,
        "wireless_mic": false,
        "wireless_mic_carrier": "auto",
        "wispr_mic_name": "",
        "wispr_switch_ms": 250,
        "wispr_tail_ms": 800,
        "event_rules": {},
        "quiet_hours": {"on": false, "from": "19:00", "to": "09:00"},
        "claude_done_min_s": 120,
        "github": {"on": true, "interval_s": 120, "primary_repo": ""},
        "update_check": true,
        "update_channel": "stable"
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// The defaults with the saved file on top (config.py `load`).
pub fn load(paths: &Paths) -> Map<String, Value> {
    let mut cfg = defaults();
    if let Ok(text) = fs::read_to_string(paths.config())
        && let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&text)
    {
        cfg.extend(saved);
    }
    cfg
}

pub fn save(paths: &Paths, cfg: &Map<String, Value>) -> std::io::Result<()> {
    let tmp = paths.home.join("config.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(cfg).unwrap_or_default())?;
    fs::rename(tmp, paths.config())
}

/// What's wrong with a `PUT /api/config` body (brain_api.valid_config, plus the stick's shape), or "".
pub fn invalid(body: &Map<String, Value>) -> String {
    if let Some(b) = body.get("brain")
        && !matches!(b.as_str(), Some("claude" | "openrouter"))
    {
        return "brain must be one of: claude, openrouter".into();
    }
    if let Some(m) = body.get("openrouter_model") {
        let ok = m.as_str().is_some_and(|s| {
            s.is_empty() || (s.len() <= 120 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:/@+".contains(c)))
        });
        if !ok {
            return "openrouter_model: not a model id".into();
        }
    }
    if let Some(n) = body.get("openrouter_max_tokens")
        && !n.as_i64().is_some_and(|n| (50..=4000).contains(&n))
    {
        return "openrouter_max_tokens must be 50-4000".into();
    }
    if let Some(h) = body.get("stt_hints") {
        let ok = h.as_array().is_some_and(|a| {
            a.len() <= 50
                && a.iter().all(|w| w.as_str().is_some_and(|w| !w.trim().is_empty() && w.chars().count() <= 40))
        });
        if !ok {
            return "stt_hints must be a list of words (at most 50, each up to 40 characters)".into();
        }
    }
    if let Some(c) = body.get("wireless_mic_carrier")
        && !c.as_str().is_some_and(|c| macos_audio::carrier::PREFS.contains(&c))
    {
        return "wireless_mic_carrier must be one of: auto, tap, blackhole".into();
    }
    if let Some(b) = body.get("wireless_mic")
        && !b.is_boolean()
    {
        return "wireless_mic must be true or false".into();
    }
    if let Some(s) = body.get("stick")
        && !s.is_null()
        && serde_json::from_value::<stick_link::Target>(s.clone()).is_err()
    {
        return "stick must be null or {\"address\", \"name\"}".into();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> Paths {
        let d = std::env::temp_dir().join(format!("stickybot-rs-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let p = Paths::new(d).unwrap();
        p.ensure().unwrap();
        p
    }

    #[test]
    fn the_python_agents_folder_is_refused() {
        let home = std::env::var("HOME").unwrap();
        assert!(Paths::new(format!("{home}/.stick-agent")).is_err());
        assert!(Paths::new(format!("{home}/.stick-agent/sub")).is_err());
        assert!(Paths::new(format!("{home}/.stickybot-rs")).is_ok());
    }

    #[test]
    fn the_token_is_made_once_and_private() {
        let p = tmp("token");
        let a = p.token_value().unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(p.token_value().unwrap(), a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(p.token()).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn saved_settings_override_the_defaults() {
        let p = tmp("cfg");
        assert_eq!(load(&p)["model"], "sonnet");
        let mut c = load(&p);
        c.insert("model".into(), json!("opus"));
        save(&p, &c).unwrap();
        let back = load(&p);
        assert_eq!(back["model"], "opus");
        assert_eq!(back["brain"], "claude");
    }

    #[test]
    fn bad_values_are_refused() {
        let m = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(invalid(&m(json!({"brain": "claude"}))), "");
        assert!(!invalid(&m(json!({"brain": "gpt"}))).is_empty());
        assert!(!invalid(&m(json!({"openrouter_max_tokens": 10}))).is_empty());
        assert!(!invalid(&m(json!({"openrouter_model": "a b"}))).is_empty());
        assert_eq!(invalid(&m(json!({"openrouter_model": "openai/gpt-5-mini"}))), "");
        assert_eq!(invalid(&m(json!({"stick": {"address": "X", "name": "Y"}}))), "");
        assert_eq!(invalid(&m(json!({"stick": null}))), "");
        assert!(!invalid(&m(json!({"stick": "X"}))).is_empty());
    }
}
