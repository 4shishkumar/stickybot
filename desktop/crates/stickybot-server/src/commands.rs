//! Quick commands: spoken phrase -> action, matched without any AI (a port of `commands.py`).
//!
//! The list lives in `~/.stickybot-rs/commands.json` (made from the Python agent's defaults on
//! first start; the dashboard's Quick commands page edits it). Phrases are matched against the
//! normalized text: `{n}` a number, `{app}` / `{q}` any words. Actions: `open_url` (+ `browser`),
//! `open_app`, `shell`.

use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

pub const DEFAULTS: &str = include_str!("../../../../mac-agent/stick_agent/defaults/commands.json");

const FILLER: &str = r"(?:(?:hey|ok|okay|please|can you|could you|would you|go ahead and|just)\s+)*";

fn small(w: &str) -> Option<u32> {
    Some(match w {
        "zero" | "oh" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        _ => return None,
    })
}

fn tens(w: &str) -> Option<u32> {
    Some(match w {
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn is_number_word(w: &str) -> bool {
    small(w).is_some() || tens(w).is_some() || matches!(w, "hundred" | "thousand" | "double" | "triple")
}

/// Spoken numbers -> digits: "one two three four" -> 1234, "twelve thirty four" -> 1234,
/// "double five" -> 55, "four thousand five hundred twelve" -> 4512.
pub fn words_to_number(words: &[&str]) -> Option<String> {
    if words.iter().any(|w| matches!(*w, "hundred" | "thousand")) {
        let (mut total, mut cur) = (0u64, 0u64);
        for w in words {
            if let Some(v) = small(w).or_else(|| tens(w)) {
                cur += v as u64;
            } else if *w == "hundred" {
                cur = cur.max(1) * 100;
            } else if *w == "thousand" {
                total += cur.max(1) * 1000;
                cur = 0;
            } else {
                return None;
            }
        }
        return Some((total + cur).to_string());
    }
    let (mut out, mut i, mut rep) = (String::new(), 0, 1);
    while i < words.len() {
        let w = words[i];
        if w == "double" || w == "triple" {
            rep = if w == "double" { 2 } else { 3 };
        } else if let Some(mut v) = tens(w) {
            if let Some(n) = words.get(i + 1).and_then(|n| small(n)).filter(|n| (1..10).contains(n)) {
                v += n;
                i += 1;
            }
            out.push_str(&v.to_string());
        } else {
            out.push_str(&small(w)?.to_string().repeat(rep));
            rep = 1;
        }
        i += 1;
    }
    (!out.is_empty()).then_some(out)
}

/// Look-alike letters a multilingual speech model may use for English ("П Р 1234").
fn lookalike(c: char) -> char {
    const FROM: &str = "АВЕКМНОРСТХаеорсухПпРрИиЛлФф";
    const TO: &str = "ABEKMHOPCTXaeopcyxPpRrIiLlFf";
    // str.maketrans keeps the last mapping of a repeated letter (Р -> R, not P)
    FROM.chars().collect::<Vec<_>>().iter().rposition(|&f| f == c).and_then(|i| TO.chars().nth(i)).unwrap_or(c)
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("a valid pattern"));
    };
}
re!(THOUSANDS, r"(\d)[,\s](\d{3})\b");
re!(HASH, r"#\s*(\d)");
re!(PUNCT, r"[^\w\s-]");
re!(NUMBER, r"\bnumber\b");
re!(ARTICLES, r"\b(the|a|an|my)\b");
re!(PR, r"\b(p\s*r|pee are|p r)\b");

/// What a phrase is matched against: lower case, no punctuation or articles, numbers as digits.
pub fn normalize(text: &str) -> String {
    let mut t: String = text.chars().map(lookalike).collect::<String>().to_lowercase();
    loop {
        // 1,234 / 1 234 -> 1234 (Python's look-behind: repeat until nothing changes)
        let next = THOUSANDS.replace_all(&t, "${1}${2}").into_owned();
        if next == t {
            break;
        }
        t = next;
    }
    let t = HASH.replace_all(&t, "${1}");
    let t = PUNCT.replace_all(&t, " ").replace('-', " ");
    let t = NUMBER.replace_all(&t, " ");
    let t = ARTICLES.replace_all(&t, " ");
    let t = PR.replace_all(&t, "pr");
    let words: Vec<&str> = t.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    for w in words.iter().copied().chain(std::iter::once("")) {
        if is_number_word(w) {
            run.push(w);
            continue;
        }
        if !run.is_empty() {
            out.push(words_to_number(&run).unwrap_or_else(|| run.join(" ")));
            run.clear();
        }
        if !w.is_empty() {
            out.push(w.to_string());
        }
    }
    out.join(" ")
}

/// A phrase as a regex over normalized text, with the filler words allowed in front.
pub fn phrase_regex(phrase: &str) -> Result<Regex, regex::Error> {
    let p = phrase.replace("{n}", "NUMSLOT").replace("{app}", "APPSLOT").replace("{q}", "QSLOT");
    let p = regex::escape(&normalize(&p))
        .replace("numslot", r"(?P<n>\d+)")
        .replace("appslot", r"(?P<app>.+)")
        .replace("qslot", r"(?P<q>.+)");
    Regex::new(&format!("^{FILLER}{p}$"))
}

pub type Slots = BTreeMap<String, String>;

/// The first command whose phrase matches, with its slots.
pub fn match_in(cmds: &[Value], text: &str) -> Option<(Value, Slots)> {
    let norm = normalize(text);
    for cmd in cmds {
        for phrase in cmd.get("phrases").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            let Ok(re) = phrase_regex(phrase) else { continue };
            if let Some(m) = re.captures(&norm) {
                let slots = re
                    .capture_names()
                    .flatten()
                    .filter_map(|n| m.name(n).map(|v| (n.to_string(), v.as_str().to_string())))
                    .filter(|(_, v)| !v.is_empty())
                    .collect();
                return Some((cmd.clone(), slots));
            }
        }
    }
    None
}

/// The commands file: read, write, defaults.
#[derive(Debug, Clone)]
pub struct Store {
    pub path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Writes the defaults if there's no file yet.
    pub fn ensure(&self) -> std::io::Result<()> {
        if !self.path.exists() {
            std::fs::write(&self.path, DEFAULTS)?;
        }
        Ok(())
    }

    /// The whole file (`{"commands": [...]}`), the defaults if it's missing or broken.
    pub fn read(&self) -> Value {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::from_str(DEFAULTS).expect("valid defaults"))
    }

    /// The commands; none if the file is unreadable (commands.py `load`).
    pub fn load(&self) -> Vec<Value> {
        match std::fs::read_to_string(&self.path).map(|t| serde_json::from_str::<Value>(&t)) {
            Ok(Ok(v)) => v.get("commands").and_then(Value::as_array).cloned().unwrap_or_default(),
            Ok(Err(e)) => {
                log::error!("commands.json unreadable: {e}");
                Vec::new()
            }
            Err(e) => {
                log::error!("commands.json unreadable: {e}");
                Vec::new()
            }
        }
    }

    pub fn find(&self, text: &str) -> Option<(Value, Slots)> {
        match_in(&self.load(), text)
    }

    /// PUT /api/commands: `{"commands": [...]}` with every phrase compiling.
    pub fn save(&self, body: &Map<String, Value>) -> Result<(), String> {
        let Some(cmds) = body.get("commands").and_then(Value::as_array) else {
            return Err("expected {\"commands\": [...]}".into());
        };
        for c in cmds {
            for p in c.get("phrases").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                phrase_regex(p).map_err(|e| format!("phrase {p:?}: {e}"))?;
            }
        }
        let text = serde_json::to_string_pretty(&Value::Object(body.clone())).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text).map_err(|e| e.to_string())
    }
}

// --- running a command -----------------------------------------------------------------------------

/// What a command does to the Mac (so tests can see it without running anything).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `open [-a App] url` / `open -a App`
    Open(Vec<String>),
    /// `sh -c …`, its output
    Shell(String),
}

/// Runs an [`Action`]; returns the output of a shell command ("" for `open`).
pub type Runner = Arc<dyn Fn(&Action) -> String + Send + Sync>;

/// The real one: `open` and `sh -c` (30 s at most).
pub fn system_runner() -> Runner {
    Arc::new(|a: &Action| match a {
        Action::Open(args) => {
            let _ = std::process::Command::new("open").args(args).status();
            String::new()
        }
        Action::Shell(cmd) => shell(cmd),
    })
}

fn shell(cmd: &str) -> String {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        return String::new();
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "Timed out.".into();
            }
        }
    }
    let (mut out, mut err) = (String::new(), String::new());
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    if out.is_empty() { err } else { out }
}

fn fill(template: &str, slots: &Slots) -> String {
    let mut s = template.to_string();
    for (k, v) in slots {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

fn short_output(out: &str) -> String {
    let t: String = out.trim().chars().take(200).collect();
    if t.is_empty() { "Done.".into() } else { t }
}

/// Runs a matched command. Returns a short result line, or `None` if it doesn't apply (no such
/// app) so the request goes to the brain instead. `quote_shell`: slot values go into a `shell`
/// command shell-quoted (when the phrase came from a model).
pub fn run(cmd: &Value, slots: &Slots, quote_shell: bool, apps: &Apps, runner: &Runner) -> Option<String> {
    let get = |k: &str| cmd.get(k).and_then(Value::as_str);
    if quote_shell && let Some(sh) = get("shell") {
        let quoted: Slots = slots.iter().map(|(k, v)| (k.clone(), shell_quote(v))).collect();
        return Some(short_output(&runner(&Action::Shell(fill(sh, &quoted)))));
    }
    if let Some(url) = get("open_url") {
        let url = fill(url, slots);
        if url.contains("<org>") || url.contains("<repo>") {
            return Some("Set your repo in the dashboard (Commands).".into());
        }
        let args = match get("browser").filter(|b| !b.is_empty()) {
            Some(b) => vec!["-a".into(), b.into(), url],
            None => vec![url],
        };
        runner(&Action::Open(args));
        let n = slots.get("n").map(|n| format!(" #{n}")).unwrap_or_default();
        return Some(format!("Opened {}{n}", get("name").unwrap_or("link")));
    }
    if let Some(app) = get("open_app") {
        let app = apps.find(&fill(app, slots))?;
        runner(&Action::Open(vec!["-a".into(), app.clone()]));
        return Some(format!("Opened {app}."));
    }
    if let Some(sh) = get("shell") {
        return Some(short_output(&runner(&Action::Shell(fill(sh, slots)))));
    }
    None
}

// --- apps --------------------------------------------------------------------------------------------

/// The Mac's apps, by lower-case name.
#[derive(Debug, Clone, Default)]
pub struct Apps {
    /// `None`: look in the Applications folders each time
    pub fixed: Option<BTreeMap<String, String>>,
}

/// Spoken names that don't look like the app's file name.
const ALIASES: &[(&str, &str)] = &[
    ("vs code", "visual studio code"),
    ("vscode", "visual studio code"),
    ("code", "visual studio code"),
    ("chrome", "google chrome"),
    ("zoom", "zoom.us"),
    ("brave", "brave browser"),
    ("browser", "brave browser"),
    ("terminal", "terminal"),
    ("settings", "system settings"),
    ("mail", "mail"),
    ("calendar", "calendar"),
    ("github", "github desktop"),
    ("docs", "google docs"),
    ("sheets", "google sheets"),
    ("slides", "google slides"),
    ("one password", "1password"),
];

impl Apps {
    pub fn system() -> Self {
        Self { fixed: None }
    }

    pub fn list(&self) -> BTreeMap<String, String> {
        if let Some(f) = &self.fixed {
            return f.clone();
        }
        let mut apps = BTreeMap::new();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        for d in [
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Applications/Utilities"),
            home.join("Applications"),
        ] {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "app")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                {
                    apps.insert(stem.to_lowercase(), stem.to_string());
                }
            }
        }
        apps
    }

    /// The app a spoken name means (commands.py `find_app`).
    pub fn find(&self, spoken: &str) -> Option<String> {
        let apps = self.list();
        let s = spoken.to_lowercase();
        let s = s.trim();
        let s = s.strip_suffix(" app").unwrap_or(s);
        let s = ALIASES.iter().find(|(k, _)| *k == s).map(|(_, v)| *v).unwrap_or(s);
        if let Some(a) = apps.get(s) {
            return Some(a.clone());
        }
        if let Some(best) =
            apps.keys().filter(|n| n.starts_with(s) || s.starts_with(n.as_str())).min_by_key(|n| n.len())
        {
            return Some(apps[best].clone());
        }
        close_match(s, apps.keys().map(String::as_str), 0.75).map(|k| apps[k].clone())
    }
}

/// difflib.SequenceMatcher(None, a, b).ratio()
pub fn ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    2.0 * matches(&a, &b) as f64 / (a.len() + b.len()) as f64
}

/// The characters in matching blocks (Ratcliff/Obershelp, like difflib without junk).
fn matches(a: &[char], b: &[char]) -> usize {
    let (mut best_i, mut best_j, mut best) = (0, 0, 0);
    let mut prev = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![0usize; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            if ca == cb {
                cur[j + 1] = prev[j] + 1;
                let k = cur[j + 1];
                // difflib keeps the earliest longest block (smallest i, then smallest j)
                if k > best {
                    (best_i, best_j, best) = (i + 1 - k, j + 1 - k, k);
                }
            }
        }
        prev = cur;
    }
    if best == 0 {
        return 0;
    }
    best + matches(&a[..best_i], &b[..best_j]) + matches(&a[best_i + best..], &b[best_j + best..])
}

/// difflib.get_close_matches(word, possibilities, n=1, cutoff)
pub fn close_match<'a>(word: &str, options: impl Iterator<Item = &'a str>, cutoff: f64) -> Option<&'a str> {
    let mut best: Option<(f64, &str)> = None;
    for o in options {
        let r = ratio(word, o);
        // nlargest over (score, x): ties go to the larger string
        if r >= cutoff && best.is_none_or(|(s, x)| r > s || (r == s && o > x)) {
            best = Some((r, o));
        }
    }
    best.map(|(_, o)| o)
}

pub fn store_path(home: &Path) -> PathBuf {
    home.join("commands.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    // --- test_commands.py --------------------------------------------------------------------------

    #[test]
    fn normalize_like_python() {
        for (said, normal) in [
            ("Open PR 1234.", "open pr 1234"),
            ("PR number 1,234", "pr 1234"),
            ("pr twelve thirty four", "pr 1234"),
            ("pr one two three four", "pr 1234"),
            ("PR four thousand five hundred twelve", "pr 4512"),
            ("pr double five", "pr 55"),
            ("Can you open pull request #42?", "can you open pull request 42"),
            ("Please open the PR 1234", "please open pr 1234"),
            ("P R 1234", "pr 1234"),
            ("П Р 1234", "pr 1234"),
            ("what's the weather", "what s weather"),
            ("1,234,567 and 12 345", "1234567 and 12345"),
        ] {
            assert_eq!(normalize(said), normal, "{said}");
        }
    }

    #[test]
    fn words_to_number_rejects_non_numbers() {
        assert_eq!(words_to_number(&["four", "banana"]), None);
    }

    fn defaults() -> Vec<Value> {
        serde_json::from_str::<Value>(DEFAULTS).unwrap()["commands"].as_array().unwrap().clone()
    }

    fn first(said: &str) -> Option<(String, Vec<(String, String)>)> {
        match_in(&defaults(), said)
            .map(|(c, s)| (c["name"].as_str().unwrap().to_string(), s.into_iter().collect::<Vec<_>>()))
    }

    #[test]
    fn default_phrases_match() {
        let s = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        for (said, name, slots) in [
            ("Open PR 1234.", "GitHub PR", s("n", "1234")),
            ("pull request 99", "GitHub PR", s("n", "99")),
            ("hey can you open the pr 7", "GitHub PR", s("n", "7")),
            ("Devin review 5678", "Devin review", s("n", "5678")),
            ("Open Slack.", "Open app", s("app", "slack")),
            ("launch visual studio code", "Open app", s("app", "visual studio code")),
        ] {
            assert_eq!(first(said), Some((name.to_string(), slots)), "{said}");
        }
    }

    #[test]
    fn no_match_goes_to_the_brain() {
        for said in ["what's the weather", "how much disk space do I have", "pr", ""] {
            assert_eq!(first(said), None, "{said}");
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-cmd-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join("commands.json")
    }

    #[test]
    fn match_reads_the_users_commands() {
        let st = Store::new(tmp("users"));
        std::fs::write(
            &st.path,
            json!({"commands": [{"name": "Docs", "phrases": ["docs {q}"], "open_url": "https://example.com/search?q={q}"}]})
                .to_string(),
        )
        .unwrap();
        let (cmd, slots) = st.find("Docs bluetooth pairing").unwrap();
        assert_eq!(cmd["name"], "Docs");
        assert_eq!(slots.get("q").map(String::as_str), Some("bluetooth pairing"));
    }

    #[test]
    fn match_survives_a_broken_commands_file() {
        let st = Store::new(tmp("broken"));
        std::fs::write(&st.path, "{not json").unwrap();
        assert!(st.find("pr 1").is_none());
    }

    #[test]
    fn saving_checks_the_shape() {
        let st = Store::new(tmp("save"));
        assert!(st.save(json!({"x": 1}).as_object().unwrap()).is_err());
        st.save(json!({"commands": [{"name": "A", "phrases": ["a {n}"]}]}).as_object().unwrap()).unwrap();
        assert_eq!(st.read()["commands"][0]["name"], "A");
    }

    fn apps() -> Apps {
        let m = [
            ("slack", "Slack"),
            ("visual studio code", "Visual Studio Code"),
            ("zoom.us", "zoom.us"),
            ("system settings", "System Settings"),
            ("brave browser", "Brave Browser"),
            ("linear", "Linear"),
        ];
        Apps { fixed: Some(m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()) }
    }

    #[test]
    fn find_app() {
        for (spoken, app) in [
            ("slack", Some("Slack")),
            ("Slack app", Some("Slack")),
            ("vs code", Some("Visual Studio Code")),
            ("zoom", Some("zoom.us")),
            ("settings", Some("System Settings")),
            ("brave", Some("Brave Browser")),
            ("lineer", Some("Linear")),
            ("photoshop", None),
        ] {
            assert_eq!(apps().find(spoken).as_deref(), app, "{spoken}");
        }
    }

    #[test]
    fn difflib_ratio() {
        assert!((ratio("lineer", "linear") - 0.8333).abs() < 1e-3);
        assert_eq!(ratio("abc", "abc"), 1.0);
        assert_eq!(ratio("abc", "xyz"), 0.0);
        assert!((ratio("photoshop", "slack") - 0.0).abs() < 0.3);
    }

    fn recorder() -> (Runner, Arc<Mutex<Vec<Action>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c = calls.clone();
        (
            Arc::new(move |a: &Action| {
                c.lock().unwrap().push(a.clone());
                "out\n".to_string()
            }),
            calls,
        )
    }

    fn slots(k: &str, v: &str) -> Slots {
        [(k.to_string(), v.to_string())].into_iter().collect()
    }

    #[test]
    fn run_open_url_in_browser() {
        let (r, calls) = recorder();
        let cmd = json!({"name": "GitHub PR", "open_url": "https://github.com/acme/app/pull/{n}", "browser": "Brave Browser"});
        assert_eq!(run(&cmd, &slots("n", "1234"), false, &apps(), &r).as_deref(), Some("Opened GitHub PR #1234"));
        assert_eq!(
            calls.lock().unwrap()[0],
            Action::Open(vec!["-a".into(), "Brave Browser".into(), "https://github.com/acme/app/pull/1234".into()])
        );
    }

    #[test]
    fn run_open_url_default_browser() {
        let (r, calls) = recorder();
        run(&json!({"open_url": "https://x.test/{n}"}), &slots("n", "1"), false, &apps(), &r);
        assert_eq!(calls.lock().unwrap()[0], Action::Open(vec!["https://x.test/1".into()]));
    }

    #[test]
    fn run_placeholder_repo_is_not_opened() {
        let (r, calls) = recorder();
        let out =
            run(&json!({"open_url": "https://github.com/<org>/<repo>/pull/{n}"}), &slots("n", "1"), false, &apps(), &r);
        assert!(out.unwrap().contains("Set your repo"));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn run_open_app() {
        let (r, calls) = recorder();
        assert_eq!(
            run(&json!({"open_app": "{app}"}), &slots("app", "slack"), false, &apps(), &r).as_deref(),
            Some("Opened Slack.")
        );
        assert_eq!(calls.lock().unwrap()[0], Action::Open(vec!["-a".into(), "Slack".into()]));
    }

    #[test]
    fn run_unknown_app_falls_through_to_the_brain() {
        let (r, calls) = recorder();
        assert_eq!(run(&json!({"open_app": "{app}"}), &slots("app", "photoshop"), false, &apps(), &r), None);
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn run_shell() {
        let (r, calls) = recorder();
        assert_eq!(run(&json!({"shell": "echo {q}"}), &slots("q", "hi"), false, &apps(), &r).as_deref(), Some("out"));
        assert_eq!(calls.lock().unwrap()[0], Action::Shell("echo hi".into()));
    }

    #[test]
    fn run_shell_quoted_keeps_a_slot_one_word() {
        let (r, calls) = recorder();
        run(&json!({"shell": "echo {q}"}), &slots("q", "x; rm -rf ~"), true, &apps(), &r);
        assert_eq!(calls.lock().unwrap()[0], Action::Shell("echo 'x; rm -rf ~'".into()));
    }

    #[test]
    fn the_real_shell_runs() {
        assert_eq!(shell("echo hello"), "hello\n");
        assert_eq!(shell("echo oops >&2"), "oops\n");
    }
}
