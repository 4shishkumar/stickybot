//! Importing the Python app's settings (`~/.stick-agent/`) at switch-over: the stick, GitHub (the
//! primary repo), the wireless mic, the brain and model, sounds, approvals, events… (every
//! `config.json` key both apps share), the voice commands (`commands.json`, with their real repo
//! URLs), the sound packs (`sounds/<pack>/`), the request history (`history.jsonl`, same format)
//! and Claude's workspace notes (`workspace/CLAUDE.md`), the faces' pet (`pet.json`), the GitHub
//! watcher's state (`github_state.json`, so nothing replays) and the events log (`events.jsonl`).
//!
//! **Read-only** on the Python folder: files are read and copied, never written, moved or
//! deleted there. Runs on its own on the first start (no `config.json` and no earlier import in
//! this app's folder) and from Settings → "Import from StickyBot" (`POST /api/import`). The
//! source is `$STICKYBOT_RS_IMPORT_FROM`, else `~/.stick-agent` (only `Options::from_env` sets
//! it: tests never look at the real folder).

use crate::config::{self, Paths};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// The record of the last import, in this app's folder.
pub const MARKER: &str = "imported.json";

/// Settings that stay this app's own: the speech model is a different one (sherpa-onnx, not MLX).
const NOT_SHARED: &[&str] = &["stt_model"];

/// `$STICKYBOT_RS_IMPORT_FROM`, else `~/.stick-agent`.
pub fn default_source() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STICKYBOT_RS_IMPORT_FROM") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".stick-agent"))
}

/// What an import did, per part (`config`, `commands`, `sounds`, `history`, `workspace`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// what was taken over, e.g. `"config: 14 settings (stick, github, …)"`
    pub imported: Vec<String>,
    /// what was left as it was, and why
    pub skipped: Vec<String>,
    /// the settings that came over (to apply them to the running app)
    pub config: Map<String, Value>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        json!({"ok": true, "imported": self.imported, "skipped": self.skipped,
               "config_keys": self.config.keys().collect::<Vec<_>>()})
    }
}

/// How to import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// the first start: fill an empty app (commands only if still the defaults, history only if
    /// there's none yet)
    FirstRun,
    /// the user asked: the Python app's settings and commands win (this app's are kept as
    /// `*.before-import`), sound packs and history are merged
    Explicit,
}

/// Is this the first start (nothing saved, nothing imported before)?
pub fn first_run(paths: &Paths) -> bool {
    !paths.config().exists() && !paths.home.join(MARKER).exists()
}

/// Imports from `from` (the Python app's folder) into `paths`. `sounds` is this app's sound
/// folder (`clips::sounds_dir`). Errors only when there's nothing to import from.
pub fn import(from: &Path, paths: &Paths, sounds: &Path, mode: Mode) -> Result<Report, String> {
    if !from.join("config.json").is_file() && !from.join("commands.json").is_file() {
        return Err(format!("no StickyBot settings in {}", from.display()));
    }
    let mut r = Report::default();
    import_config(from, paths, mode, &mut r);
    import_commands(from, paths, mode, &mut r);
    import_sounds(&from.join("sounds"), sounds, &mut r);
    import_history(from, paths, &mut r);
    import_workspace(from, paths, mode, &mut r);
    import_state(from, paths, mode, &mut r);
    let marker = json!({"t": crate::hub::now(), "from": from, "mode": format!("{mode:?}"),
                        "imported": r.imported, "skipped": r.skipped});
    if let Err(e) = fs::write(paths.home.join(MARKER), serde_json::to_string_pretty(&marker).unwrap_or_default()) {
        log::warn!("{MARKER}: {e}");
    }
    Ok(r)
}

/// Keeps a copy of this app's file before replacing it.
fn backup(p: &Path) {
    if p.is_file() {
        let mut b = p.as_os_str().to_owned();
        b.push(".before-import");
        let _ = fs::copy(p, PathBuf::from(b));
    }
}

fn read_json(p: &Path) -> Option<Value> {
    fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// The Python `config.json`: every key both apps know, each checked like `PUT /api/config`.
pub fn shared_settings(py: &Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    let defaults = config::defaults();
    let (mut ok, mut bad) = (Map::new(), Vec::new());
    for (k, v) in py {
        if !defaults.contains_key(k) || NOT_SHARED.contains(&k.as_str()) {
            continue;
        }
        let one: Map<String, Value> = [(k.clone(), v.clone())].into_iter().collect();
        let why = config::invalid(&one);
        // the same type as the default (a string stays a string…), except `stick` (null or an object)
        let same_kind = k == "stick"
            || std::mem::discriminant(v) == std::mem::discriminant(&defaults[k])
            || (v.is_number() && defaults[k].is_number());
        if why.is_empty() && same_kind {
            ok.insert(k.clone(), v.clone());
        } else {
            bad.push(k.clone());
        }
    }
    // nested objects: the Python file may lack newer keys (github.primary_repo…): keep the defaults'
    for (k, v) in ok.iter_mut() {
        if let (Value::Object(have), Some(Value::Object(def))) = (v, defaults.get(k)) {
            for (dk, dv) in def {
                have.entry(dk.clone()).or_insert_with(|| dv.clone());
            }
        }
    }
    (ok, bad)
}

fn import_config(from: &Path, paths: &Paths, mode: Mode, r: &mut Report) {
    let Some(Value::Object(py)) = read_json(&from.join("config.json")) else {
        r.skipped.push("config: no readable config.json".into());
        return;
    };
    let (shared, bad) = shared_settings(&py);
    if !bad.is_empty() {
        r.skipped.push(format!("config: kept the defaults for {} (not valid here)", bad.join(", ")));
    }
    if shared.is_empty() {
        r.skipped.push("config: nothing to take over".into());
        return;
    }
    if mode == Mode::Explicit {
        backup(&paths.config());
    }
    let mut cfg = config::load(paths);
    cfg.extend(shared.clone());
    if let Err(e) = config::save(paths, &cfg) {
        r.skipped.push(format!("config: couldn't save: {e}"));
        return;
    }
    let mut keys: Vec<&String> = shared.keys().collect();
    keys.sort();
    let names: Vec<&str> = keys.iter().take(8).map(|k| k.as_str()).collect();
    let more = if keys.len() > 8 { ", …" } else { "" };
    r.imported.push(format!("config: {} settings ({}{more})", shared.len(), names.join(", ")));
    r.config = shared;
}

fn import_commands(from: &Path, paths: &Paths, mode: Mode, r: &mut Report) {
    let src = from.join("commands.json");
    let Some(v) = read_json(&src) else {
        r.skipped.push("commands: no readable commands.json".into());
        return;
    };
    let Some(list) = v.get("commands").and_then(Value::as_array) else {
        r.skipped.push("commands: commands.json has no \"commands\" list".into());
        return;
    };
    let dst = paths.commands();
    let mine = read_json(&dst);
    let defaults: Value = serde_json::from_str(crate::commands::DEFAULTS).unwrap_or(Value::Null);
    let untouched = mine.as_ref().is_none_or(|m| *m == defaults);
    if mine.as_ref() == Some(&v) {
        r.skipped.push("commands: already the same".into());
        return;
    }
    if mode == Mode::FirstRun && !untouched {
        r.skipped.push("commands: this app's own commands were kept".into());
        return;
    }
    backup(&dst);
    match fs::copy(&src, &dst) {
        Ok(_) => r.imported.push(format!("commands: {} voice commands", list.len())),
        Err(e) => r.skipped.push(format!("commands: {e}")),
    }
}

fn import_sounds(src: &Path, dst: &Path, r: &mut Report) {
    let Ok(packs) = fs::read_dir(src) else {
        return; // no sound packs: nothing to say
    };
    let (mut copied, mut had, mut pack_names) = (0, 0, Vec::new());
    for pack in packs.flatten() {
        let p = pack.path();
        // real folders only (no links out of the folder)
        if !pack.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = pack.file_name();
        let Ok(files) = fs::read_dir(&p) else { continue };
        let mut any = false;
        for f in files.flatten() {
            let fp = f.path();
            if !f.file_type().is_ok_and(|t| t.is_file()) || !crate::clips::is_sound(&fp) {
                continue;
            }
            let out = dst.join(&name).join(f.file_name());
            if out.exists() {
                had += 1;
                continue;
            }
            if fs::create_dir_all(dst.join(&name)).is_ok() && fs::copy(&fp, &out).is_ok() {
                copied += 1;
                any = true;
            }
        }
        if any {
            pack_names.push(name.to_string_lossy().into_owned());
        }
    }
    pack_names.sort();
    if copied > 0 {
        r.imported.push(format!("sounds: {copied} clips ({})", pack_names.join(", ")));
    }
    if had > 0 {
        r.skipped.push(format!("sounds: {had} clips were already here"));
    }
}

/// A history line both apps write: a JSON object with a numeric `t`.
fn history_entry(line: &str) -> Option<(f64, String)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let t = v.as_object()?.get("t")?.as_f64()?;
    Some((t, line.trim().to_string()))
}

fn import_history(from: &Path, paths: &Paths, r: &mut Report) {
    merge_jsonl(&from.join("history.jsonl"), &paths.history(), "history", "requests", r);
}

/// A pet nothing has happened to yet (made on this app's first start).
fn untouched_pet(v: &Value) -> bool {
    ["claude", "greens", "releases"].iter().all(|k| v.get(*k).and_then(Value::as_i64).unwrap_or(0) == 0)
        && v["streak"]["last"].as_i64().unwrap_or(-1) < 0
}

fn valid_state(file: &str, v: &Value) -> bool {
    match file {
        "pet.json" => v.get("xp").is_some_and(Value::is_i64) && v.get("seed").is_some() && v["streak"].is_object(),
        _ => v.get("prs").is_some_and(Value::is_object),
    }
}

/// The pet's progress (`pet.json`) and the GitHub watcher's state (`github_state.json`): taken
/// over on an explicit import, and on the first start unless this app already has its own; the
/// events log (`events.jsonl`) merged like the history.
fn import_state(from: &Path, paths: &Paths, mode: Mode, r: &mut Report) {
    for (file, what) in [("pet.json", "pet"), ("github_state.json", "github")] {
        let Some(v) = read_json(&from.join(file)) else { continue };
        if !v.is_object() || !valid_state(file, &v) {
            r.skipped.push(format!("{what}: {file} in an unknown format"));
            continue;
        }
        let dst = paths.home.join(file);
        let mine = read_json(&dst);
        if mine.as_ref() == Some(&v) {
            continue;
        }
        let replaceable = match &mine {
            None => true,
            Some(m) => file == "pet.json" && untouched_pet(m),
        };
        if mode == Mode::FirstRun && !replaceable {
            r.skipped.push(format!("{what}: this app's own {file} was kept"));
            continue;
        }
        backup(&dst);
        match fs::write(&dst, v.to_string()) {
            Ok(()) => r.imported.push(if what == "pet" {
                format!("pet: level {} ({} XP)", crate::pet::level_for(v["xp"].as_i64().unwrap_or(0)).0, v["xp"])
            } else {
                format!("github: {} PRs watched", v["prs"].as_object().map_or(0, Map::len))
            }),
            Err(e) => r.skipped.push(format!("{what}: {e}")),
        }
    }
    merge_jsonl(&from.join("events.jsonl"), &paths.home.join("events.jsonl"), "events", "events", r);
}

/// Merges a JSON-lines log (`{"t": …, …}` per line) into ours: in time order, no duplicates.
fn merge_jsonl(src: &Path, dst: &Path, what: &str, unit: &str, r: &mut Report) {
    let Ok(py) = fs::read_to_string(src) else { return };
    let mine = fs::read_to_string(dst).unwrap_or_default();
    let have: std::collections::HashSet<&str> = mine.lines().map(str::trim).collect();
    let (mut entries, mut unknown) = (Vec::new(), 0);
    for line in py.lines().filter(|l| !l.trim().is_empty()) {
        match history_entry(line) {
            Some(e) if !have.contains(e.1.as_str()) => entries.push(e),
            Some(_) => {}
            None => unknown += 1,
        }
    }
    if unknown > 0 {
        r.skipped.push(format!("{what}: {unknown} lines in an unknown format"));
    }
    if entries.is_empty() {
        return;
    }
    let n = entries.len();
    entries.extend(mine.lines().filter_map(history_entry));
    entries.sort_by(|a, b| a.0.total_cmp(&b.0));
    let text: String = entries.into_iter().map(|(_, l)| l + "\n").collect();
    let mut tmp = dst.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    match fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, dst)) {
        Ok(()) => r.imported.push(format!("{what}: {n} {unit}")),
        Err(e) => r.skipped.push(format!("{what}: {e}")),
    }
}

fn import_workspace(from: &Path, paths: &Paths, mode: Mode, r: &mut Report) {
    let Ok(py) = fs::read_to_string(from.join("workspace").join("CLAUDE.md")) else { return };
    if py == crate::brain::CLAUDE_MD {
        return; // the default one: nothing of the user's in it
    }
    let dst = paths.home.join("workspace").join("CLAUDE.md");
    let mine = fs::read_to_string(&dst).ok();
    if mine.as_deref() == Some(py.as_str()) {
        return;
    }
    if mode == Mode::FirstRun && mine.as_deref().is_some_and(|m| m != crate::brain::CLAUDE_MD) {
        r.skipped.push("workspace: this app's own CLAUDE.md was kept".into());
        return;
    }
    backup(&dst);
    match fs::create_dir_all(paths.home.join("workspace")).and_then(|_| fs::write(&dst, py)) {
        Ok(()) => r.imported.push("workspace: Claude's CLAUDE.md (your changes to it)".into()),
        Err(e) => r.skipped.push(format!("workspace: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake home with a Python app folder in it (never the real one).
    struct Fake {
        root: PathBuf,
        py: PathBuf,
        paths: Paths,
        sounds: PathBuf,
    }

    impl Fake {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("stickybot-migrate-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let py = root.join("home/.stick-agent");
            fs::create_dir_all(py.join("sounds/kenney/sub")).unwrap();
            fs::create_dir_all(py.join("workspace")).unwrap();
            let paths = Paths::new(root.join("home/.stickybot-rs")).unwrap();
            paths.ensure().unwrap();
            let sounds = paths.home.join("sounds");
            Fake { root, py, paths, sounds }
        }
        fn python_files(&self) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
            let mut out = Vec::new();
            let mut stack = vec![self.py.clone()];
            while let Some(d) = stack.pop() {
                for e in fs::read_dir(&d).unwrap().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        let m = fs::metadata(&p).unwrap().modified().unwrap();
                        out.push((p.clone(), fs::read(&p).unwrap(), m));
                    }
                }
            }
            out.sort();
            out
        }
        fn import(&self, mode: Mode) -> Report {
            import(&self.py, &self.paths, &self.sounds, mode).unwrap()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn python_config() -> Value {
        // the shape config.py saves: DEFAULT_CONFIG with the user's changes, plus a removed key
        let mut c = json!({
            "stick": {"address": "6A1C-STICK", "name": "Ashish's stick"},
            "model": "opus",
            "stt_model": "mlx-community/parakeet-tdt-0.6b-v3",
            "brain": "openrouter",
            "openrouter_model": "openai/gpt-5-mini",
            "openrouter_max_tokens": 600,
            "speak_replies": true,
            "wireless_mic": true,
            "wireless_mic_carrier": "tap",
            "wispr_switch_ms": 400,
            "github": {"on": true, "interval_s": 60, "primary_repo": "acme/widgets"},
            "quiet_hours": {"on": true, "from": "22:00", "to": "07:00"},
            "event_rules": {"pr_merged": "celebrate"},
            "onboarding": {"flash": true},
            "wispr_fallback_mic": "MacBook Pro Microphone",
            "update_channel": "beta"
        });
        c["safe_tools"] = json!(["Read"]);
        c
    }

    fn write_python(f: &Fake) {
        fs::write(f.py.join("config.json"), serde_json::to_string_pretty(&python_config()).unwrap()).unwrap();
        let cmds = json!({"commands": [
            {"name": "GitHub PR", "phrases": ["open pr {n}"], "open_url": "https://github.com/acme/widgets/pull/{n}",
             "browser": "Brave Browser"},
            {"name": "Slack", "phrases": ["open slack"], "open_app": "Slack"}
        ]});
        fs::write(f.py.join("commands.json"), serde_json::to_string_pretty(&cmds).unwrap()).unwrap();
        fs::write(f.py.join("sounds/kenney/glass_001.ogg"), b"OggS fake").unwrap();
        fs::write(f.py.join("sounds/kenney/bong_001.wav"), b"RIFF fake").unwrap();
        fs::write(f.py.join("sounds/kenney/readme.txt"), b"not a sound").unwrap();
        fs::write(f.py.join("sounds/kenney/sub/deep.ogg"), b"nested: not a pack file").unwrap();
        fs::write(f.py.join("sounds/loose.ogg"), b"not in a pack").unwrap();
        let hist = [
            json!({"t": 100.0, "heard": "open pr 12", "source": "stick", "via": "quick", "command": "GitHub PR",
                   "reply": "Opened GitHub PR #12"}),
            json!({"t": 200.5, "heard": "weather", "source": "stick", "via": "claude", "ok": true, "reply": "Sunny"}),
        ];
        let mut h: String = hist.iter().map(|v| format!("{v}\n")).collect();
        h.push_str("not json\n");
        fs::write(f.py.join("history.jsonl"), h).unwrap();
        fs::write(f.py.join("workspace/CLAUDE.md"), "# my notes\nAlways answer in one line.\n").unwrap();
        fs::write(f.py.join("token"), "python-secret").unwrap();
    }

    #[test]
    fn everything_both_apps_share_comes_over_and_the_python_folder_is_untouched() {
        let f = Fake::new("all");
        write_python(&f);
        // the app made its default commands before the import (AppState::new does)
        crate::commands::Store::new(f.paths.commands()).ensure().unwrap();
        assert!(first_run(&f.paths));
        let before = f.python_files();

        let r = f.import(Mode::FirstRun);

        let cfg = config::load(&f.paths);
        assert_eq!(cfg["stick"], json!({"address": "6A1C-STICK", "name": "Ashish's stick"}));
        assert_eq!(cfg["github"]["primary_repo"], "acme/widgets");
        assert_eq!(cfg["github"]["interval_s"], 60);
        assert_eq!(cfg["model"], "opus");
        assert_eq!(cfg["brain"], "openrouter");
        assert_eq!(cfg["openrouter_model"], "openai/gpt-5-mini");
        assert_eq!(cfg["wireless_mic"], true);
        assert_eq!(cfg["wireless_mic_carrier"], "tap");
        assert_eq!(cfg["wispr_switch_ms"], 400);
        assert_eq!(cfg["quiet_hours"]["from"], "22:00");
        assert_eq!(cfg["safe_tools"], json!(["Read"]));
        assert_eq!(cfg["update_channel"], "beta");
        // not shared / removed in Python / this app's own
        assert_eq!(cfg["stt_model"], config::defaults()["stt_model"]);
        assert!(!cfg.contains_key("wispr_fallback_mic"));
        assert_eq!(cfg["stt_hints"], config::defaults()["stt_hints"]);
        assert!(r.config.contains_key("stick") && !r.config.contains_key("stt_model"));

        let cmds = crate::commands::Store::new(f.paths.commands()).load();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0]["open_url"], "https://github.com/acme/widgets/pull/{n}");

        let lib = crate::clips::library(&f.sounds);
        assert_eq!(
            lib,
            vec![json!({"pack": "kenney", "name": "bong_001"}), json!({"pack": "kenney", "name": "glass_001"})]
        );
        assert_eq!(fs::read(f.sounds.join("kenney/glass_001.ogg")).unwrap(), b"OggS fake");

        let h = fs::read_to_string(f.paths.history()).unwrap();
        assert_eq!(h.lines().count(), 2);
        assert!(h.lines().next().unwrap().contains("open pr 12"));
        assert_eq!(
            fs::read_to_string(f.paths.home.join("workspace/CLAUDE.md")).unwrap(),
            "# my notes\nAlways answer in one line.\n"
        );
        // the token stays each app's own
        assert_ne!(f.paths.token_value().unwrap(), "python-secret");

        assert!(r.imported.iter().any(|s| s.starts_with("config: ")), "{r:?}");
        assert!(r.imported.contains(&"commands: 2 voice commands".to_string()), "{r:?}");
        assert!(r.imported.contains(&"sounds: 2 clips (kenney)".to_string()), "{r:?}");
        assert!(r.imported.contains(&"history: 2 requests".to_string()), "{r:?}");
        assert!(r.skipped.contains(&"history: 1 lines in an unknown format".to_string()), "{r:?}");

        // read-only: every Python file as it was (bytes and times), nothing added or removed
        assert_eq!(f.python_files(), before);
        // and it's done: the next start isn't a first run
        assert!(!first_run(&f.paths));
    }

    #[test]
    fn a_first_run_keeps_what_the_user_already_made_here() {
        let f = Fake::new("keep");
        write_python(&f);
        let mine = json!({"commands": [{"name": "Mine", "phrases": ["do my thing"], "shell": "true"}]});
        fs::write(f.paths.commands(), mine.to_string()).unwrap();
        fs::create_dir_all(f.sounds.join("kenney")).unwrap();
        fs::write(f.sounds.join("kenney/glass_001.ogg"), b"my own glass").unwrap();
        fs::write(f.paths.history(), format!("{}\n", json!({"t": 150.0, "heard": "rust one", "source": "typed"})))
            .unwrap();

        let r = f.import(Mode::FirstRun);
        assert_eq!(crate::commands::Store::new(f.paths.commands()).load()[0]["name"], "Mine");
        assert!(r.skipped.contains(&"commands: this app's own commands were kept".to_string()));
        assert_eq!(fs::read(f.sounds.join("kenney/glass_001.ogg")).unwrap(), b"my own glass");
        assert!(r.skipped.contains(&"sounds: 1 clips were already here".to_string()), "{r:?}");
        // the histories merge in time order
        let h: Vec<Value> =
            fs::read_to_string(f.paths.history()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(h.iter().map(|v| v["t"].as_f64().unwrap()).collect::<Vec<_>>(), [100.0, 150.0, 200.5]);
    }

    #[test]
    fn an_explicit_import_wins_keeps_a_copy_and_doesnt_repeat_history() {
        let f = Fake::new("explicit");
        write_python(&f);
        let mut cfg = config::defaults();
        cfg.insert("model".into(), json!("haiku"));
        cfg.insert("stt_hints".into(), json!(["Kubernetes"]));
        config::save(&f.paths, &cfg).unwrap();
        let mine = json!({"commands": [{"name": "Mine", "phrases": ["x"], "shell": "true"}]});
        fs::write(f.paths.commands(), mine.to_string()).unwrap();
        assert!(!first_run(&f.paths));

        f.import(Mode::Explicit);
        let cfg = config::load(&f.paths);
        assert_eq!(cfg["model"], "opus");
        assert_eq!(cfg["stt_hints"], json!(["Kubernetes"])); // this app's own setting stays
        assert_eq!(crate::commands::Store::new(f.paths.commands()).load().len(), 2);
        let kept: Value = read_json(&f.paths.home.join("commands.json.before-import")).unwrap();
        assert_eq!(kept, mine);
        assert!(f.paths.home.join("config.json.before-import").exists());

        // again: nothing doubles
        let r = f.import(Mode::Explicit);
        assert_eq!(fs::read_to_string(f.paths.history()).unwrap().lines().count(), 2);
        assert!(r.skipped.contains(&"commands: already the same".to_string()));
        assert!(!r.imported.iter().any(|s| s.starts_with("sounds") || s.starts_with("history")), "{r:?}");
    }

    #[test]
    fn bad_values_keep_the_defaults_and_no_folder_is_an_error() {
        let f = Fake::new("bad");
        fs::write(
            f.py.join("config.json"),
            json!({"brain": "gpt", "wireless_mic": "yes", "openrouter_max_tokens": 5, "stick": "X", "model": 3,
                   "github": {"primary_repo": "acme/x"}})
            .to_string(),
        )
        .unwrap();
        let r = f.import(Mode::FirstRun);
        let cfg = config::load(&f.paths);
        assert_eq!(cfg["brain"], "claude");
        assert_eq!(cfg["wireless_mic"], false);
        assert_eq!(cfg["stick"], Value::Null);
        assert_eq!(cfg["model"], "sonnet");
        // a partial github object gets the other defaults
        assert_eq!(cfg["github"], json!({"on": true, "interval_s": 120, "primary_repo": "acme/x"}));
        assert!(r.skipped.iter().any(|s| s.contains("brain") && s.contains("stick")), "{r:?}");

        let empty = f.root.join("nothing-here");
        assert!(import(&empty, &f.paths, &f.sounds, Mode::Explicit).is_err());
    }

    #[test]
    fn the_pet_github_state_and_events_come_over() {
        let f = Fake::new("pet");
        fs::write(f.py.join("config.json"), "{}").unwrap();
        let pet = json!({"xp": 125, "merged": 9, "greens": 4, "claude": 7, "releases": 1, "towers": [8], "seed": 4242,
                         "streak": {"days": 3, "best": 6, "last": 739000}, "today": {"day": 739000, "merged": 1}});
        fs::write(f.py.join("pet.json"), pet.to_string()).unwrap();
        let gh = json!({"prs": {"acme/web#7": {"repo": "acme/web", "number": 7, "merged": true}}, "asks": {},
                        "release": "v1", "primary": "acme/web", "seen_at": 100.0});
        fs::write(f.py.join("github_state.json"), gh.to_string()).unwrap();
        fs::write(
            f.py.join("events.jsonl"),
            format!("{}\n{}\n", json!({"t": 5.0, "kind": "pr_merged", "id": "e1"}), json!({"t": 9.0, "kind": "test", "id": "e2"})),
        )
        .unwrap();
        // this app's first start already made a fresh pet: it's replaced
        crate::pet::Pet::load(&f.paths.home.join("pet.json"), 0);
        let before = f.python_files();
        let r = f.import(Mode::FirstRun);
        assert_eq!(read_json(&f.paths.home.join("pet.json")).unwrap(), pet);
        assert_eq!(read_json(&f.paths.home.join("github_state.json")).unwrap(), gh);
        assert_eq!(fs::read_to_string(f.paths.home.join("events.jsonl")).unwrap().lines().count(), 2);
        assert!(r.imported.contains(&"pet: level 4 (125 XP)".to_string()), "{r:?}");
        assert!(r.imported.contains(&"github: 1 PRs watched".to_string()), "{r:?}");
        assert!(r.imported.contains(&"events: 2 events".to_string()), "{r:?}");
        assert_eq!(f.python_files(), before);

        // a pet with progress of its own is kept on a first run, replaced on an explicit import
        let mut mine = pet.clone();
        mine["claude"] = json!(99);
        fs::write(f.paths.home.join("pet.json"), mine.to_string()).unwrap();
        let r = f.import(Mode::FirstRun);
        assert!(r.skipped.contains(&"pet: this app's own pet.json was kept".to_string()), "{r:?}");
        f.import(Mode::Explicit);
        assert_eq!(read_json(&f.paths.home.join("pet.json")).unwrap(), pet);
        assert_eq!(read_json(&f.paths.home.join("pet.json.before-import")).unwrap(), mine);
        assert_eq!(fs::read_to_string(f.paths.home.join("events.jsonl")).unwrap().lines().count(), 2);

        // unknown formats are skipped
        fs::write(f.py.join("pet.json"), "[1, 2]").unwrap();
        let r = f.import(Mode::Explicit);
        assert!(r.skipped.contains(&"pet: pet.json in an unknown format".to_string()), "{r:?}");
    }
}
