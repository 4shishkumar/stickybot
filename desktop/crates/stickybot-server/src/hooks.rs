//! Claude Code's hooks for this app (installer.py's `our_hooks` / `merge_hooks` / `remove_hooks` /
//! `hooks_state`, plus `hook.py` and `claude_hooks.py` themselves).
//!
//! The Python app registers `<venv python> …/stick_agent/hook.py` and `…/claude_hooks.py`. This app
//! has no Python: the hooks run **this program** with `--hook permission|prompt|stop` (the menu
//! bar app and `stickybot-headless` both take it before anything else starts), which does what the
//! two scripts do and posts to this app (its port and token, read when the hook runs).
//!
//! The Python app's hooks are left alone while it's in use (they keep reaching it on 8660; after
//! the switch-over they reach this app there, with the token it took over). Only after the
//! switch-over does adding the hooks here replace them with this app's.

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const EVENTS: [&str; 3] = ["PermissionRequest", "UserPromptSubmit", "Stop"];

/// `'…'` for /bin/sh (Claude Code runs hook commands through a shell; the bundle's path has spaces).
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:@".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn arg(event: &str) -> &'static str {
    match event {
        "PermissionRequest" => "permission",
        "UserPromptSubmit" => "prompt",
        _ => "stop",
    }
}

/// event -> this app's hook entry.
pub fn our_hooks(exe: &Path) -> Map<String, Value> {
    let cmd = |a: &str| format!("{} --hook {a}", sh_quote(&exe.to_string_lossy()));
    let mut m = Map::new();
    m.insert(
        "PermissionRequest".into(),
        json!({"type": "command", "command": cmd("permission"), "timeout": 45, "statusMessage": "Waiting for the stick…"}),
    );
    m.insert("UserPromptSubmit".into(), json!({"type": "command", "command": cmd("prompt"), "timeout": 5}));
    m.insert("Stop".into(), json!({"type": "command", "command": cmd("stop"), "timeout": 5}));
    m
}

fn command(entry: &Value) -> &str {
    entry.get("command").and_then(Value::as_str).unwrap_or("")
}

/// This app's hook, from this or another build (paths differ, `--hook <event>` doesn't).
pub fn is_ours(entry: &Value, event: &str) -> bool {
    let cmd = command(entry);
    cmd.ends_with(&format!(" --hook {}", arg(event))) && cmd.to_lowercase().contains("stickybot")
}

/// The Python app's hook (installer.py `is_ours`).
pub fn is_python(entry: &Value, event: &str) -> bool {
    let script = if event == "PermissionRequest" { "stick_agent/hook.py" } else { "stick_agent/claude_hooks.py" };
    command(entry).contains(script)
}

fn groups<'a>(settings: &'a Value, event: &str) -> impl Iterator<Item = &'a Value> {
    settings.get("hooks").and_then(|h| h.get(event)).and_then(Value::as_array).into_iter().flatten()
}

fn entries(group: &Value) -> impl Iterator<Item = &Value> {
    group.get("hooks").and_then(Value::as_array).into_iter().flatten()
}

/// What's there: (this app's hooks that are current, this app's at all, the Python app's).
pub fn state(settings: &Value, exe: &Path) -> (usize, usize, usize) {
    let ours = our_hooks(exe);
    let (mut current, mut present, mut python) = (0, 0, 0);
    for event in EVENTS {
        if let Some(h) = groups(settings, event).flat_map(entries).find(|h| is_ours(h, event)) {
            present += 1;
            current += usize::from(Some(h) == ours.get(event));
        }
        python += usize::from(groups(settings, event).flat_map(entries).any(|h| is_python(h, event)));
    }
    (current, present, python)
}

/// Adds / updates this app's hooks, keeping everything else (`replace_python`: the Python app's
/// hooks become this app's instead of being kept). -> (settings, changed).
pub fn merge(settings: &Value, exe: &Path, replace_python: bool) -> (Value, bool) {
    let mut s = if settings.is_object() { settings.clone() } else { json!({}) };
    let ours = our_hooks(exe);
    let mut changed = false;
    if !s["hooks"].is_object() {
        s["hooks"] = json!({});
    }
    for event in EVENTS {
        let want = ours[event].clone();
        let hooks = &mut s["hooks"];
        if !hooks[event].is_array() {
            hooks[event] = json!([]);
        }
        let groups = hooks[event].as_array_mut().unwrap();
        let mut found = false;
        for g in groups.iter_mut() {
            let Some(list) = g.get_mut("hooks").and_then(Value::as_array_mut) else { continue };
            let mut i = 0;
            while i < list.len() {
                let mine = is_ours(&list[i], event) || (replace_python && is_python(&list[i], event));
                if mine && found {
                    list.remove(i); // a second one (ours and the Python app's in one group)
                    changed = true;
                    continue;
                }
                if mine {
                    found = true;
                    if list[i] != want {
                        list[i] = want.clone();
                        changed = true;
                    }
                }
                i += 1;
            }
        }
        if !found {
            // a group for every tool (one with a matcher only fires for some)
            match groups.iter_mut().find(|g| g.get("matcher").is_none_or(|m| m.as_str() == Some(""))) {
                Some(g) => {
                    if !g["hooks"].is_array() {
                        g["hooks"] = json!([]);
                    }
                    g["hooks"].as_array_mut().unwrap().push(want);
                }
                None => groups.push(json!({"hooks": [want]})),
            }
            changed = true;
        }
    }
    (s, changed)
}

/// Removes only this app's hooks; drops groups / events that become empty. -> (settings, changed).
pub fn remove(settings: &Value) -> (Value, bool) {
    let mut s = settings.clone();
    let mut changed = false;
    let Some(hooks) = s.get_mut("hooks").and_then(Value::as_object_mut) else { return (s, false) };
    let events: Vec<String> = hooks.keys().cloned().collect();
    for event in events {
        let mut kept_groups = Vec::new();
        for g in hooks[&event].as_array().cloned().unwrap_or_default() {
            let list = g.get("hooks").and_then(Value::as_array).cloned().unwrap_or_default();
            let kept: Vec<Value> = list.iter().filter(|h| !is_ours(h, &event)).cloned().collect();
            if kept.len() != list.len() {
                changed = true;
            }
            if !kept.is_empty() {
                let mut g2 = g.clone();
                g2["hooks"] = Value::Array(kept);
                kept_groups.push(g2);
            }
        }
        if kept_groups.is_empty() {
            hooks.remove(&event);
            changed = true;
        } else {
            hooks.insert(event, Value::Array(kept_groups));
        }
    }
    if hooks.is_empty() {
        s.as_object_mut().map(|o| o.remove("hooks"));
    }
    (s, changed)
}

/// Claude Code's settings (an empty or missing file is `{}`); a broken file is an error: it's
/// never overwritten.
pub fn read_settings(path: &Path) -> Result<Value, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => Ok(v),
        _ => Err(format!("{} isn't valid JSON", path.display())),
    }
}

/// Writes the settings, keeping a dated backup of the old file. -> the backup's path.
pub fn write_settings(path: &Path, new: &Value) -> std::io::Result<Option<PathBuf>> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut backup = None;
    if path.exists() {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let b = path.with_file_name(format!("{name}.bak-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        std::fs::copy(path, &b)?;
        backup = Some(b);
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(new).unwrap_or_default() + "\n")?;
    std::fs::rename(tmp, path)?;
    Ok(backup)
}

// --- the hooks themselves (`<this program> --hook …`) ---------------------------------------------

/// hook.py `describe`.
pub fn describe(tool: &str, input: &Value) -> String {
    if tool == "Bash" {
        return input.get("command").and_then(Value::as_str).unwrap_or("").to_string();
    }
    for k in ["file_path", "notebook_path", "path", "url", "pattern", "query"] {
        if let Some(v) = input.get(k) {
            return v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
        }
    }
    input.to_string()
}

/// How long the permission hook waits for A / B (then Claude Code asks in the terminal).
pub const WAIT_S: u64 = 30;
const KEEP_S: f64 = 24.0 * 3600.0;

fn post(port: u16, token: &str, path: &str, body: &Value, timeout: std::time::Duration) -> Option<Value> {
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let r = agent
        .post(&format!("http://127.0.0.1:{port}{path}"))
        .set("Content-Type", "application/json")
        .set("X-Stick-Token", token)
        .send_string(&body.to_string())
        .ok()?;
    serde_json::from_str(&r.into_string().ok()?).ok()
}

/// The PermissionRequest hook: what to print (nothing: Claude Code asks as usual).
pub fn permission(req: &Value, home: &Path, port: u16, token: &str) -> Option<String> {
    let cwd = PathBuf::from(req.get("cwd").and_then(Value::as_str).unwrap_or(""));
    let origin = if cwd == home.join("workspace") {
        "Agent".to_string()
    } else {
        cwd.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or("terminal".into())
    };
    let tool = req.get("tool_name").and_then(Value::as_str).unwrap_or("?");
    let hint: String = describe(tool, req.get("tool_input").unwrap_or(&json!({}))).chars().take(300).collect();
    let body = json!({"tool": tool, "hint": hint, "from": origin, "wait": WAIT_S});
    let answer = post(port, token, "/api/permission", &body, std::time::Duration::from_secs(WAIT_S + 5))?;
    let d = answer.get("decision").and_then(Value::as_str)?;
    if d != "allow" && d != "deny" {
        return None;
    }
    let mut decision = json!({"behavior": d});
    if d == "deny" {
        decision["message"] = json!("Denied on the stick");
    }
    Some(json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}).to_string())
}

/// UserPromptSubmit / Stop (claude_hooks.py): remembers the prompt per session in
/// `<home>/claude_sessions.json`; at Stop tells the app how long it took. -> the body posted.
pub fn session_event(req: &Value, home: &Path, now: f64, send: &dyn Fn(&Value)) -> Option<Value> {
    let event = req.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
    let sid = req.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("?").to_string();
    let cwd = req.get("cwd").and_then(Value::as_str).unwrap_or("").to_string();
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon(Path::new(if cwd.is_empty() { "/" } else { &cwd })) == canon(&home.join("workspace")) {
        return None; // this app's own Claude
    }
    let file = home.join("claude_sessions.json");
    let mut sessions: Map<String, Value> =
        std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    sessions.retain(|_, v| now - v.get("t").and_then(Value::as_f64).unwrap_or(0.0) < KEEP_S);
    let save = |s: &Map<String, Value>| {
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, Value::Object(s.clone()).to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    };
    match event {
        "UserPromptSubmit" => {
            let prompt: String = req.get("prompt").and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
            sessions.insert(sid, json!({"t": now, "prompt": prompt, "cwd": cwd}));
            save(&sessions);
            None
        }
        "Stop" => {
            let start = sessions.remove(&sid);
            save(&sessions);
            let start = start?;
            let t = start.get("t").and_then(Value::as_f64).unwrap_or(now);
            let cwd =
                if cwd.is_empty() { start.get("cwd").and_then(Value::as_str).unwrap_or("").to_string() } else { cwd };
            let body = json!({"elapsed_s": (now - t).round() as i64, "cwd": cwd,
                              "prompt": start.get("prompt").cloned().unwrap_or(json!(""))});
            send(&body);
            Some(body)
        }
        _ => None,
    }
}

/// `<program> --hook permission|prompt|stop`: reads Claude Code's JSON on stdin, never fails
/// (exit 0, nothing printed unless there's a decision). -> the exit code.
pub fn main(kind: &str) -> i32 {
    use std::io::Read;
    let opts = crate::Options::from_env();
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(req) = serde_json::from_str::<Value>(&input) else { return 0 };
    let token = std::fs::read_to_string(opts.home.join("token")).map(|t| t.trim().to_string()).unwrap_or_default();
    if token.is_empty() {
        return 0;
    }
    match kind {
        "permission" => {
            if let Some(out) = permission(&req, &opts.home, opts.port, &token) {
                println!("{out}");
            }
        }
        "prompt" | "stop" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let (port, tok) = (opts.port, token.clone());
            session_event(&req, &opts.home, now, &move |body| {
                post(port, &tok, "/api/claude-event", body, std::time::Duration::from_secs(2));
            });
        }
        _ => {}
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "/Applications/StickyBot (Rust).app/Contents/MacOS/stickybot-app";

    fn exe() -> PathBuf {
        PathBuf::from(EXE)
    }

    fn python_entry(event: &str) -> Value {
        let s = if event == "PermissionRequest" { "hook.py" } else { "claude_hooks.py" };
        json!({"type": "command", "command": format!("/x/venv/bin/python /x/stick_agent/{s}"), "timeout": 5})
    }

    // --- test_installer.py's merge cases ---

    #[test]
    fn merge_into_empty_settings_adds_three_hooks() {
        let (s, changed) = merge(&json!({}), &exe(), false);
        assert!(changed);
        assert_eq!(state(&s, &exe()), (3, 3, 0));
        assert_eq!(s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 45);
        let cmd = s["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(cmd, format!("'{EXE}' --hook stop"));
    }

    #[test]
    fn merge_is_idempotent() {
        let (s, _) = merge(&json!({}), &exe(), false);
        let (s2, changed) = merge(&s, &exe(), false);
        assert!(!changed);
        assert_eq!(s, s2);
    }

    #[test]
    fn merge_keeps_other_hooks_and_settings() {
        let other = json!({"type": "command", "command": "echo hi"});
        let before = json!({"model": "opus", "hooks": {"Stop": [{"hooks": [other.clone()]}],
                                                      "PreToolUse": [{"matcher": "Bash", "hooks": [other.clone()]}]}});
        let (s, _) = merge(&before, &exe(), false);
        assert_eq!(s["model"], "opus");
        assert_eq!(s["hooks"]["PreToolUse"], before["hooks"]["PreToolUse"]);
        assert_eq!(s["hooks"]["Stop"][0]["hooks"][0], other);
        assert_eq!(s["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn merge_doesnt_join_a_group_with_a_matcher() {
        let before = json!({"hooks": {"PermissionRequest": [{"matcher": "Bash", "hooks": [{"command": "x"}]}]}});
        let (s, _) = merge(&before, &exe(), false);
        let groups = s["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["hooks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn merge_updates_a_hook_from_another_build() {
        let (old, _) = merge(&json!({}), Path::new("/old/StickyBot (Rust).app/Contents/MacOS/stickybot-app"), false);
        assert_eq!(state(&old, &exe()), (0, 3, 0)); // present, not current
        let (s, changed) = merge(&old, &exe(), false);
        assert!(changed);
        assert_eq!(state(&s, &exe()), (3, 3, 0));
        assert_eq!(s["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_python_apps_hooks_are_kept_or_replaced() {
        let py = json!({"hooks": {"PermissionRequest": [{"hooks": [python_entry("PermissionRequest")]}],
                                  "Stop": [{"hooks": [python_entry("Stop")]}],
                                  "UserPromptSubmit": [{"hooks": [python_entry("UserPromptSubmit")]}]}});
        assert_eq!(state(&py, &exe()), (0, 0, 3));
        let (kept, _) = merge(&py, &exe(), false);
        assert_eq!(state(&kept, &exe()), (3, 3, 3));
        let (replaced, changed) = merge(&py, &exe(), true);
        assert!(changed);
        assert_eq!(state(&replaced, &exe()), (3, 3, 0));
        // both in one group: replacing leaves just ours
        let (s, _) = merge(&kept, &exe(), true);
        assert_eq!(state(&s, &exe()), (3, 3, 0));
        assert_eq!(s["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn remove_takes_only_ours() {
        let other = json!({"type": "command", "command": "echo hi"});
        let (s, _) = merge(&json!({"hooks": {"Stop": [{"hooks": [other.clone()]}]}}), &exe(), false);
        let (r, changed) = remove(&s);
        assert!(changed);
        assert_eq!(r, json!({"hooks": {"Stop": [{"hooks": [other]}]}}));
        let (s, _) = merge(&json!({"x": 1}), &exe(), false);
        assert_eq!(remove(&s).0, json!({"x": 1})); // everything gone: no "hooks" key
        assert!(!remove(&json!({"x": 1})).1);
    }

    #[test]
    fn settings_files_backup_and_broken() {
        let d = std::env::temp_dir().join(format!("stickybot-rs-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("settings.json");
        assert_eq!(read_settings(&p).unwrap(), json!({}));
        assert_eq!(write_settings(&p, &json!({"a": 1})).unwrap(), None);
        let b = write_settings(&p, &json!({"a": 2})).unwrap().unwrap();
        assert_eq!(read_settings(&b).unwrap(), json!({"a": 1}));
        assert_eq!(read_settings(&p).unwrap(), json!({"a": 2}));
        std::fs::write(&p, "{broken").unwrap();
        assert!(read_settings(&p).unwrap_err().contains("isn't valid JSON"));
        std::fs::write(&p, "  ").unwrap();
        assert_eq!(read_settings(&p).unwrap(), json!({}));
    }

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("/usr/bin/x"), "/usr/bin/x");
        assert_eq!(sh_quote("/a b/it's"), "'/a b/it'\\''s'");
    }

    // --- test_hooks.py ---

    #[test]
    fn describe_like_hook_py() {
        assert_eq!(describe("Bash", &json!({"command": "ls -la"})), "ls -la");
        assert_eq!(describe("Edit", &json!({"file_path": "/x/y.rs", "old": 1})), "/x/y.rs");
        assert_eq!(describe("Other", &json!({"a": 1})), "{\"a\":1}");
    }

    #[test]
    fn permission_hook_with_nothing_listening_prints_nothing() {
        let req = json!({"tool_name": "Bash", "tool_input": {"command": "ls"}, "cwd": "/tmp/proj"});
        // a port nothing listens on
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert_eq!(permission(&req, Path::new("/tmp/h"), port, "t"), None);
    }

    #[test]
    fn session_hooks_record_and_report() {
        let home = std::env::temp_dir().join(format!("stickybot-rs-hooks-s-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("workspace")).unwrap();
        let sent = std::sync::Mutex::new(Vec::new());
        let send = |b: &Value| sent.lock().unwrap().push(b.clone());
        let p = json!({"hook_event_name": "UserPromptSubmit", "session_id": "s1", "cwd": "/w/web", "prompt": "fix it"});
        assert_eq!(session_event(&p, &home, 1000.0, &send), None);
        let s = json!({"hook_event_name": "Stop", "session_id": "s1", "cwd": "/w/web"});
        let body = session_event(&s, &home, 1300.4, &send).unwrap();
        assert_eq!(body, json!({"elapsed_s": 300, "cwd": "/w/web", "prompt": "fix it"}));
        assert_eq!(sent.lock().unwrap().len(), 1);
        // a Stop without its prompt: nothing
        assert_eq!(session_event(&s, &home, 1400.0, &send), None);
        // this app's own Claude is ignored
        let own = json!({"hook_event_name": "UserPromptSubmit", "session_id": "s2",
                         "cwd": home.join("workspace").to_string_lossy()});
        session_event(&own, &home, 1000.0, &send);
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("claude_sessions.json")).unwrap()).unwrap();
        assert!(saved.get("s2").is_none());
        // garbage: nothing
        assert_eq!(session_event(&json!({"x": 1}), &home, 1.0, &send), None);
    }
}
