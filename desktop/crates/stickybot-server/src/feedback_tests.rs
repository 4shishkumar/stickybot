use super::*;
use crate::Options;
use axum::body::Body;
use axum::extract::Request;
use http_body_util::BodyExt;
use stick_link::{Backend, fake::FakeStick};
use tower::ServiceExt;

fn known() -> Known {
    Known {
        token: "Zq3xV9kLm2Pq7RsT1uVw8XyZ0aBcDeFg".into(),
        user: "jdoe".into(),
        host: "Janes-MacBook".into(),
        sticks: vec!["Arbaaz".into()],
        addresses: vec!["5A1C2B3D-0000-4000-8000-1234567890AB".into()],
        repos: vec!["acme/rocket-web".into()],
        titles: vec!["Fix the login redirect loop".into()],
        wifi: vec!["HomeNet 5G".into()],
        meetings: vec![
            "Quarterly numbers".into(),
            "Teammate A".into(),
            "https://meet.google.com/abc-defg-hij".into(),
            "1:1".into(),
        ],
    }
}

// --- redaction, one category at a time -----------------------------------------------------------

#[test]
fn keys_and_tokens_go() {
    let k = known();
    let sk = ["sk", "or", "v1", "abcdef0123456789abcdef"].join("-");
    let ghp = format!("{}_{}", "ghp", "A".repeat(36));
    let pat = format!("{}_{}", "github_pat", "11ABCDEFG0123456789_abcdefghijklmnop");
    let text = format!(
        "key {sk}\ngh {ghp}\npat {pat}\nAuthorization: Bearer abc.def-ghi\ncurl -H Bearer zzz.yyy\nX-Stick-Token: {}\napi_key=hunter22 password: swordfish\n",
        k.token
    );
    let out = redact(&text, &k);
    for gone in [sk.as_str(), &ghp, &pat, "abc.def-ghi", "zzz.yyy", &k.token, "hunter22", "swordfish"] {
        assert!(!out.contains(gone), "{gone} left in:\n{out}");
    }
    assert!(out.contains("key [key]"), "{out}");
    assert!(out.contains("gh [github token]"), "{out}");
    assert!(out.contains("Authorization: [redacted]\n"), "{out}");
    assert!(out.contains("curl -H Bearer [redacted]"), "{out}");
    assert!(out.contains("X-Stick-Token: [token]\n"), "{out}");
    // the user's own text: only secrets
    assert_eq!(redact_secrets(&format!("my key is {sk} ok"), ""), "my key is [key] ok");
    assert_eq!(redact_secrets(&format!("token {}", k.token), &k.token), "token [token]");
}

#[test]
fn emails_paths_user_and_host_go() {
    let k = known();
    let out = redact(
        "mail jane.doe+x@example.co.uk\nopen /Users/jdoe/Library/Logs/x.log\nuser jdoe on Janes-MacBook\njdoes stays\n",
        &k,
    );
    assert!(out.contains("mail [email]"), "{out}");
    assert!(out.contains("open ~/Library/Logs/x.log"), "{out}");
    assert!(out.contains("user [user] on [mac name]"), "{out}");
    assert!(out.contains("jdoes stays"), "whole words only: {out}");
    // someone else's home folder too
    assert_eq!(redact("/Users/other/x", &Known::default()), "~/x");
}

#[test]
fn github_repos_and_pr_titles_go() {
    let k = known();
    let out = redact(
        "github: Fix the login redirect loop in acme/rocket-web\nsee https://github.com/acme/rocket-web/pull/12 and github.com/other/thing\nrocket-web #12; acme org\n",
        &k,
    );
    for gone in ["Fix the login", "acme", "rocket-web", "other/thing"] {
        assert!(!out.contains(gone), "{gone} left in:\n{out}");
    }
    assert!(out.contains("[PR title]"), "{out}");
    assert!(out.contains("github.com/[repo]"), "{out}");
    // event lines carry PR titles of repos the watcher no longer tracks
    let ev =
        redact("12:00:01 events: event pr_merged -> nice: PR #3 merged! / web #3 - Secret plan", &Known::default());
    assert_eq!(ev, "12:00:01 events: event pr_merged -> nice: [event text]");
    let off = redact("events: event ci_failed (off): CI failed", &Known::default());
    assert_eq!(off, "events: event ci_failed (off): [event text]");
}

#[test]
fn what_was_said_goes() {
    let k = Known::default();
    assert_eq!(
        redact("10:00:00 talk: heard (2.1s audio, 0.30s, 0 lost): open my bank statement", &k),
        "10:00:00 talk: heard (2.1s audio, 0.30s, 0 lost): [voice transcript]"
    );
    assert_eq!(redact("brain: claude -p \"what's on my calendar\" --model sonnet", &k), "brain: claude [arguments]");
    assert_eq!(
        redact("mcp: claude code: notify(text='Deploy done for client X') -> ok (3 ms)", &k),
        "mcp: claude code: notify(…)"
    );
    assert_eq!(redact("openrouter: reply: Sure, your meeting is at 3", &k), "openrouter: reply: [text]");
    assert_eq!(redact("question: should I merge?", &k), "question: [text]");
    // the lines around them stay readable
    assert_eq!(redact("speech model ready in 2.3s (parakeet)", &k), "speech model ready in 2.3s (parakeet)");
    assert_eq!(redact("reading aloud: the last reply (12 words)", &k), "reading aloud: the last reply (12 words)");
}

#[test]
fn wifi_names_go() {
    let k = known();
    let out = redact("stick web: joined HomeNet 5G\nssid=\"Cafe Free\" ok\nwifi: Office\n", &k);
    for gone in ["HomeNet", "Cafe Free", "Office"] {
        assert!(!out.contains(gone), "{gone} left in:\n{out}");
    }
    assert!(out.contains("joined [wifi]"), "{out}");
    assert!(out.contains("ssid=[wifi] ok"), "{out}");
}

#[test]
fn bluetooth_addresses_serials_and_the_stick_name_get_a_short_hash() {
    let k = known();
    let out = redact(
        "connected to Arbaaz (MTU 247)\nstick 5A1C2B3D-0000-4000-8000-1234567890AB\nmac AA:BB:CC:11:22:33\nserial: ABC12345XYZ\nthe stick is now called \"Bob\" (was \"Arbaaz\")\n",
        &k,
    );
    for gone in ["Arbaaz", "5A1C2B3D", "AA:BB:CC", "ABC12345XYZ", "Bob"] {
        assert!(!out.contains(gone), "{gone} left in:\n{out}");
    }
    let h = short_hash("5A1C2B3D-0000-4000-8000-1234567890AB");
    assert_eq!(h.len(), 6);
    assert!(out.contains(&format!("stick [bt:{h}]")), "{out}");
    assert!(out.contains(&format!("mac [bt:{}]", short_hash("AA:BB:CC:11:22:33"))), "{out}");
    assert!(out.contains(&format!("serial: [serial:{}]", short_hash("ABC12345XYZ"))), "{out}");
    assert!(out.contains("connected to [stick name] (MTU 247)"), "{out}");
    // the same id hashes the same way (two reports about one stick can be matched), case aside
    assert_eq!(short_hash("aa:bb:cc:11:22:33"), short_hash("AA:BB:CC:11:22:33"));
    // an unknown UUID still goes
    let u = redact("peripheral 11111111-2222-3333-4444-555555555555 found", &Known::default());
    assert!(u.starts_with("peripheral [id:") && !u.contains("1111"), "{u}");
    // times stay
    assert_eq!(redact("12:34:56 link: ok", &Known::default()), "12:34:56 link: ok");
}

#[test]
fn diagnostics_are_redacted_and_fit_the_form() {
    let k = known();
    let facts = Facts {
        app: "StickyBot (Rust)".into(),
        version: "2.4.0".into(),
        firmware: Some("2.5.0".into()),
        macos: "15.1".into(),
        arch: "aarch64".into(),
        model: Some("m5stick-s3".into()),
        link: "connected".into(),
        features: vec!["brain=claude".into(), "mcp_server=on".into()],
    };
    let log =
        vec!["10:00:00 talk: heard (1s audio, 0.1s, 0 lost): call mom".to_string(), "10:00:01 a /Users/jdoe/x".into()];
    let d = diagnostics(&facts, &log, &k);
    assert!(
        d.starts_with(
            "App: StickyBot (Rust) 2.4.0\nFirmware: 2.5.0\nmacOS: 15.1 (aarch64)\nStick: m5stick-s3 · link connected"
        ),
        "{d}"
    );
    assert!(d.contains("Features: brain=claude, mcp_server=on"));
    assert!(d.contains("--- last 2 lines of app.log ---"));
    assert!(!d.contains("call mom") && !d.contains("jdoe"), "{d}");
    assert_eq!(facts.version_line(), "StickyBot (Rust) 2.4.0 · firmware 2.5.0 · macOS 15.1 aarch64");
    // a long log: the oldest lines go, the header stays
    let long: Vec<String> = (0..LOG_LINES).map(|i| format!("{i:03} {}", "x".repeat(400))).collect();
    let d = diagnostics(&facts, &long, &k);
    assert!(d.len() <= MAX_DIAGNOSTICS, "{}", d.len());
    assert!(d.starts_with("App: "));
    assert!(d.ends_with(&format!("199 {}", "x".repeat(400))));
    assert!(!d.contains("\n000 "));
}

#[test]
fn meeting_titles_attendees_and_links_go() {
    let k = known();
    let out = redact(
        "refresh: Quarterly numbers with Teammate A\nopened https://meet.google.com/abc-defg-hij for you\nthe quarterly NUMBERS again\nit is a 1:1 today\n",
        &k,
    );
    for gone in ["Quarterly numbers", "quarterly NUMBERS", "Teammate", "meet.google.com", "abc-defg-hij"] {
        assert!(!out.contains(gone), "{gone} left in:\n{out}");
    }
    assert!(out.contains("refresh: [meeting] with [meeting]"), "{out}");
    // a very short title is not worth blanking a word of the logs for
    assert!(out.contains("1:1 today"), "{out}");
    // the reminder's own log line carries no text at all; a debug line that did would still go
    assert_eq!(redact("event meeting_soon -> headsup", &k), "event meeting_soon -> headsup");
    assert_eq!(
        redact("event meeting_soon -> headsup: Standup in 5 min / 10:30 · Google Meet", &Known::default()),
        "event meeting_soon -> headsup: [event text]"
    );
    // a tool call's result is cut like the others
    assert_eq!(
        redact("mcp: Agent: calendar_next() -> (from your calendar) (4 ms)", &Known::default()),
        "mcp: Agent: calendar_next(…)"
    );
}

#[test]
fn features_say_on_and_off_but_no_free_text() {
    let mut cfg = crate::config::defaults();
    assert!(features(&cfg, true).contains(&"calendar=off".to_string()));
    cfg.insert("connections".into(), json!({"calendar": {"on": true, "calendars": ["work-id-1"]}}));
    let f = features(&cfg, true).join(", ");
    assert!(f.contains("calendar=on") && !f.contains("work-id"), "{f}");
    cfg.insert("github".into(), json!({"on": false, "primary_repo": "acme/secret"}));
    cfg.insert("model".into(), json!("a model with spaces / and a path"));
    let f = features(&cfg, true).join(", ");
    assert!(f.contains("brain=claude") && f.contains("mcp_server=on") && f.contains("dock_always=off"), "{f}");
    assert!(f.contains("github_watcher=off") && f.contains("speech_model=ready"), "{f}");
    assert!(!f.contains("acme") && !f.contains("spaces"), "{f}");
}

// --- config, the report, the payload --------------------------------------------------------------

const URL: &str = "https://script.google.com/macros/s/AKfycbx-example/exec";

#[test]
fn unset_config_means_not_set_up() {
    let mut cfg = crate::config::defaults();
    assert_eq!(feedback_url(&cfg), "", "the compiled-in address is empty for now");
    cfg.insert("feedback_url".into(), json!(URL));
    assert_eq!(feedback_url(&cfg), URL);
    // a foreign address is never used, even if it got into config.json by hand
    cfg.insert("feedback_url".into(), json!("https://evil.example/macros/s/x/exec"));
    assert_eq!(feedback_url(&cfg), "");
    // the compiled-in one is used when the setting is empty
    cfg.insert("feedback_url".into(), json!(""));
    assert_eq!(url_from(&cfg, URL), URL);
}

#[test]
fn the_config_is_checked() {
    let ok = |v: Value| valid_config(v.as_object().unwrap());
    assert_eq!(ok(json!({"dock_always": true})), "");
    assert_ne!(ok(json!({"dock_always": "yes"})), "");
    assert_eq!(ok(json!({"feedback_url": ""})), "");
    assert_eq!(ok(json!({"feedback_url": URL})), "");
    assert_eq!(ok(json!({"feedback_url": "http://127.0.0.1:9/macros/s/x/exec"})), "", "a stand-in on this Mac");
    assert_ne!(ok(json!({"feedback_url": "http://script.google.com/macros/s/x/exec"})), "");
    assert_ne!(ok(json!({"feedback_url": "https://script.google.com/macros/s/x/dev"})), "");
    assert_ne!(ok(json!({"feedback_url": "https://docs.google.com/forms/d/e/x/formResponse"})), "");
    assert_ne!(ok(json!({"feedback_url": 3})), "");
    // and through PUT /api/config's own check
    assert_ne!(crate::config::invalid(json!({"dock_always": 1}).as_object().unwrap()), "");
}

fn facts() -> Facts {
    Facts {
        app: "StickyBot (Rust)".into(),
        version: "2.4.0".into(),
        firmware: Some("2.5.0".into()),
        macos: "15.1".into(),
        arch: "aarch64".into(),
        model: Some("m5stick-s3".into()),
        link: "connected".into(),
        features: vec![],
    }
}

fn report() -> Report {
    let mut r = Report {
        id: "SB-ABC234".into(),
        kind: "Bug".into(),
        what: "It froze & beeped".into(),
        expected: "No beep".into(),
        steps: "1. hold B\n2. wait".into(),
        contact: "jane@example.com".into(),
        diagnostics: "App: x".into(),
        created: "2026-11-11T10:00:00+00:00".into(),
        ..Default::default()
    };
    r.stamp(&facts());
    r
}

#[test]
fn the_payload_is_what_the_script_takes() {
    let p = report().payload("0123456789abcdef");
    assert_eq!(
        p,
        json!({"v": 1, "id": "SB-ABC234", "install": "0123456789abcdef", "type": "Bug", "what": "It froze & beeped",
               "expected": "No beep", "steps": "1. hold B\n2. wait", "contact": "jane@example.com",
               "app": "StickyBot (Rust) 2.4.0", "fw": "2.5.0", "macos": "15.1 aarch64",
               "stick": "m5stick-s3 · link connected", "diagnostics": "App: x",
               "created": "2026-11-11T10:00:00+00:00", "hp": ""})
    );
    // the local bookkeeping isn't sent
    let mut r = report();
    r.sent = true;
    r.error = "x".into();
    assert!(r.payload("i").get("sent").is_none() && r.payload("i").get("error").is_none());
    // the answer: ok with our id
    assert!(accepted(&json!({"ok": true, "id": "SB-ABC234"}), "SB-ABC234").is_ok());
    assert!(accepted(&json!({"ok": true}), "SB-ABC234").is_ok());
    assert_eq!(accepted(&json!({"ok": false, "error": "rate limited"}), "SB-ABC234").unwrap_err(), "rate limited");
    assert!(accepted(&json!({"ok": true, "id": "SB-ZZZZZZ"}), "SB-ABC234").is_err());
}

#[test]
fn ids_are_short_and_readable() {
    for _ in 0..50 {
        let id = new_id();
        assert!(id_ok(&id), "{id}");
        assert!(!id.contains('0') && !id.contains('O') && !id.contains('1') && !id.contains('I'));
    }
    assert!(!id_ok("SB-abc234") && !id_ok("SB-ABC23") && !id_ok("../etc/x") && !id_ok("SB-ABC2340"));
}

#[test]
fn the_report_text_and_the_body_checks() {
    let t = report().text();
    assert!(t.starts_with("StickyBot bug report SB-ABC234\n\n## What happened\nIt froze & beeped\n"), "{t}");
    assert!(t.contains("## Version\nStickyBot (Rust) 2.4.0 · firmware 2.5.0 · macOS 15.1 aarch64\n"), "{t}");
    assert!(t.contains("## Diagnostics\nApp: x"));
    let b = |v: Value| Report::from_body(v.as_object().unwrap());
    assert!(b(json!({"type": "Bug", "what": "x"})).is_ok());
    assert!(b(json!({"type": "Rant", "what": "x"})).is_err());
    assert!(b(json!({"type": "Idea", "what": "  "})).is_err());
    assert!(b(json!({"type": "Idea", "what": "x".repeat(MAX_TEXT + 1)})).is_err());
    assert!(b(json!({"type": "Idea", "what": "x", "contact": 3})).is_err());
    let big = b(json!({"type": "Question", "what": "x", "diagnostics": "line\n".repeat(20_000)})).unwrap();
    assert!(big.diagnostics.len() <= MAX_DIAGNOSTICS && big.diagnostics.starts_with("[… the start was cut"));
}

#[test]
fn five_reports_an_hour() {
    let l = RateLimit::new(PER_HOUR, HOUR);
    let t0 = Instant::now();
    for i in 0..PER_HOUR {
        assert!(l.take(t0 + Duration::from_secs(i as u64 * 60)).is_ok());
    }
    assert_eq!(l.left(t0 + Duration::from_secs(300)), 0);
    let wait = l.take(t0 + Duration::from_secs(600)).unwrap_err();
    assert_eq!(wait, Duration::from_secs(3000), "until the first one is an hour old");
    assert!(l.take(t0 + HOUR).is_ok(), "an hour after the first, one more");
    assert!(l.take(t0 + HOUR + Duration::from_secs(1)).is_err());
    assert_eq!(l.left(t0 + HOUR * 3), PER_HOUR);
}

#[test]
fn local_copies_and_the_install_id() {
    let d = std::env::temp_dir().join(format!("stickybot-feedback-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let s = Store::new(&d);
    let id = s.install_id();
    assert_eq!(id.len(), 16);
    assert_eq!(s.install_id(), id, "kept");
    let mut a = report();
    a.id = "SB-AAAAAA".into();
    a.created = "2026-11-10T10:00:00+00:00".into();
    let mut b = report();
    b.id = "SB-BBBBBB".into();
    b.sent = true;
    s.save(&a).unwrap();
    s.save(&b).unwrap();
    assert_eq!(s.load("SB-AAAAAA"), Some(a.clone()));
    assert_eq!(s.load("../config"), None);
    let ids: Vec<String> = s.list().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, ["SB-BBBBBB", "SB-AAAAAA"], "newest first");
    assert!(report().summary().get("diagnostics").unwrap().as_bool().unwrap());
    let _ = std::fs::remove_dir_all(&d);
}

// --- a stand-in for the Apps Script web app (on this Mac: no request reaches Google) ---------------

/// Answers like Apps Script: a POST to `/macros/s/x/exec` gets a 302 to `/macros/echo?…`, and that
/// (a GET) gets the JSON. `answer`: what the echo says (`None`: our id back, ok). Records
/// `(method, path, body)`.
struct Script {
    url: String,
    seen: Arc<Mutex<Vec<(String, String, String)>>>,
}

fn script(answer: Option<&'static str>) -> Script {
    use std::io::{BufRead, BufReader, Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<(String, String, String)>::new()));
    let rec = seen.clone();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut r = BufReader::new(conn.try_clone().unwrap());
            let mut line = String::new();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
            let mut len = 0;
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = r.read_exact(&mut body);
            let body = String::from_utf8_lossy(&body).to_string();
            rec.lock().unwrap().push((method.clone(), path.clone(), body.clone()));
            let mut out = conn;
            let resp = if method == "POST" && path == "/macros/s/x/exec" {
                format!(
                    "HTTP/1.1 302 Moved Temporarily\r\nLocation: http://127.0.0.1:{port}/macros/echo?user_content_key=k\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            } else if method == "GET" && path.starts_with("/macros/echo") {
                let posted = rec
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|(m, _, _)| m == "POST")
                    .map(|p| p.2.clone())
                    .unwrap_or_default();
                let id = serde_json::from_str::<Value>(&posted).ok().map(|v| v["id"].clone()).unwrap_or_default();
                let text = answer.map(str::to_string).unwrap_or_else(|| json!({"ok": true, "id": id}).to_string());
                let ctype = if text.starts_with('{') { "application/json" } else { "text/html" };
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                )
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
            };
            let _ = out.write_all(resp.as_bytes());
        }
    });
    Script { url: format!("http://127.0.0.1:{port}/macros/s/x/exec"), seen }
}

#[test]
fn the_post_follows_apps_scripts_redirect_with_a_get() {
    let s = script(None);
    let body = report().payload("i");
    let a = UreqPoster.post_json(&s.url, &body).unwrap();
    assert_eq!(a, json!({"ok": true, "id": "SB-ABC234"}));
    let seen = s.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!((seen[0].0.as_str(), seen[0].1.as_str()), ("POST", "/macros/s/x/exec"));
    assert_eq!(serde_json::from_str::<Value>(&seen[0].2).unwrap(), body, "the JSON as sent");
    assert_eq!((seen[1].0.as_str(), seen[1].2.as_str()), ("GET", ""), "the answer is fetched, the body not sent again");
    assert!(seen[1].1.starts_with("/macros/echo?user_content_key="));
}

#[test]
fn a_sign_in_page_or_a_refusal_is_an_error() {
    let s = script(Some("<html>Sign in</html>"));
    let e = UreqPoster.post_json(&s.url, &json!({})).unwrap_err();
    assert!(e.contains("Anyone"), "{e}");
    let s = script(Some(r#"{"ok": false, "error": "too many reports"}"#));
    let a = UreqPoster.post_json(&s.url, &json!({"id": "SB-ABC234"})).unwrap();
    assert_eq!(accepted(&a, "SB-ABC234").unwrap_err(), "too many reports");
    // nothing listening
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    assert!(UreqPoster.post_json(&format!("http://127.0.0.1:{dead}/macros/s/x/exec"), &json!({})).is_err());
}

// --- the API (the fake stick's app; its poster is a FakePoster or the stand-in above) -------------

const PORT: u16 = 8670;

async fn app(name: &str) -> (Router, Arc<AppState>, Arc<FakePoster>) {
    let home = std::env::temp_dir().join(format!("stickybot-rs-feedback-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let opts = Options { port: PORT, home, fake_stick: true, import_from: None };
    let st = AppState::new(&opts, Backend::Fake(FakeStick::new())).unwrap();
    let post = Arc::new(FakePoster::default());
    st.feedback.set_poster(post.clone());
    (crate::api::router(st.clone()), st, post)
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>, token: &str) -> (StatusCode, Value) {
    let r = Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("localhost:{PORT}"))
        .header("x-stick-token", token)
        .header("content-type", "application/json")
        .body(body.map(|v| Body::from(v.to_string())).unwrap_or_default())
        .unwrap();
    let resp = app.clone().oneshot(r).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn not_set_up_offers_the_report_to_copy() {
    let (app, st, post) = app("unset").await;
    let (s, v) = call(&app, "GET", "/api/feedback", None, "").await;
    assert_eq!((s, v["configured"].clone(), v["left"].clone()), (StatusCode::OK, json!(false), json!(PER_HOUR)));
    let (s, v) = call(&app, "POST", "/api/feedback", Some(json!({"type": "Bug", "what": "it broke"})), &st.token).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["not_configured"], true);
    assert!(v["report"].as_str().unwrap().contains("## What happened\nit broke"), "{v}");
    assert!(post.sent.lock().unwrap().is_empty());
    let (_, list) = call(&app, "GET", "/api/feedback/reports", None, "").await;
    assert_eq!(list, json!([]), "nothing kept: there's nowhere to send it");
}

#[tokio::test]
async fn a_report_is_posted_redacted_kept_and_rate_limited() {
    let (app, st, post) = app("send").await;
    let (s, _) = call(&app, "PUT", "/api/config", Some(json!({"feedback_url": URL})), &st.token).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, "GET", "/api/feedback", None, "").await;
    assert_eq!(v["configured"], true);
    // the diagnostics the page shows are already redacted
    let (s, d) = call(&app, "GET", "/api/feedback/diagnostics", None, "").await;
    assert_eq!(s, StatusCode::OK);
    let text = d["text"].as_str().unwrap();
    assert!(text.starts_with("App: StickyBot (Rust) "), "{text}");
    assert!(!text.contains(&st.token));
    // the token is needed to send
    let (s, _) = call(&app, "POST", "/api/feedback", Some(json!({"type": "Bug", "what": "x"})), "nope").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let edited = format!("{text}\nmy edit: {} at /Users/jdoe/x", st.token);
    let body = json!({"type": "Bug", "what": format!("broke, token {}", st.token), "contact": "Jane <jane@example.com>",
                      "diagnostics": edited});
    let (s, v) = call(&app, "POST", "/api/feedback", Some(body.clone()), &st.token).await;
    assert_eq!((s, v["ok"].clone(), v["sent"].clone()), (StatusCode::OK, json!(true), json!(true)), "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    assert!(id_ok(&id), "{id}");
    let sent = post.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let (url, p) = &sent[0];
    assert_eq!(url, URL);
    assert_eq!((p["v"].clone(), p["id"].clone(), p["type"].clone()), (json!(1), json!(id), json!("Bug")));
    assert_eq!(p["what"], "broke, token [token]");
    assert_eq!(p["contact"], "Jane <jane@example.com>", "the contact is the user's to give");
    assert_eq!(p["install"].as_str().unwrap().len(), 16);
    assert!(p["app"].as_str().unwrap().starts_with("StickyBot (Rust) "));
    let diag = p["diagnostics"].as_str().unwrap();
    assert!(!diag.contains(&st.token) && !diag.contains("jdoe") && diag.contains("my edit: [token] at ~/x"), "{diag}");
    // kept, redacted, as sent
    let (_, list) = call(&app, "GET", "/api/feedback/reports", None, "").await;
    assert_eq!(list[0]["id"], json!(id));
    assert_eq!(list[0]["sent"], true);
    let kept = std::fs::read_to_string(st.paths.home.join("feedback").join(format!("{id}.json"))).unwrap();
    assert!(!kept.contains(&st.token), "{kept}");
    let (s, _) = call(&app, "POST", &format!("/api/feedback/reports/{id}/retry"), None, &st.token).await;
    assert_eq!(s, StatusCode::CONFLICT, "already sent");
    // 4 more, then the hour is full: the 6th is kept, unsent
    for _ in 1..PER_HOUR {
        assert_eq!(call(&app, "POST", "/api/feedback", Some(body.clone()), &st.token).await.0, StatusCode::OK);
    }
    let (s, v) = call(&app, "POST", "/api/feedback", Some(body.clone()), &st.token).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(v["error"].as_str().unwrap().contains("retry in 60 min"), "{v}");
    let held = v["id"].as_str().unwrap().to_string();
    assert!(v["report"].as_str().unwrap().contains(&held));
    assert_eq!(post.sent.lock().unwrap().len(), PER_HOUR);
    let (_, list) = call(&app, "GET", "/api/feedback/reports", None, "").await;
    let unsent: Vec<&Value> = list.as_array().unwrap().iter().filter(|r| r["sent"] == false).collect();
    assert_eq!(unsent.len(), 1);
    assert_eq!(unsent[0]["id"], json!(held));
}

#[tokio::test]
async fn a_failed_post_is_kept_and_can_be_retried() {
    let (app, st, post) = app("fail").await;
    call(&app, "PUT", "/api/config", Some(json!({"feedback_url": URL})), &st.token).await;
    *post.fail.lock().unwrap() = Some("timed out".into());
    let (s, v) =
        call(&app, "POST", "/api/feedback", Some(json!({"type": "Idea", "what": "dark mode"})), &st.token).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert!(v["error"].as_str().unwrap().starts_with("Couldn't reach the feedback service"));
    assert_eq!(v["detail"], "timed out");
    let id = v["id"].as_str().unwrap().to_string();
    assert!(v["report"].as_str().unwrap().starts_with(&format!("StickyBot idea report {id}")));
    let (_, list) = call(&app, "GET", "/api/feedback/reports", None, "").await;
    assert_eq!((list[0]["sent"].clone(), list[0]["error"].clone()), (json!(false), json!("timed out")));
    let (s, one) = call(&app, "GET", &format!("/api/feedback/reports/{id}"), None, "").await;
    assert_eq!(s, StatusCode::OK);
    assert!(one["report"].as_str().unwrap().contains("dark mode"));
    // online again: the retry sends the same report, same id
    *post.fail.lock().unwrap() = None;
    let (s, v) = call(&app, "POST", &format!("/api/feedback/reports/{id}/retry"), None, &st.token).await;
    assert_eq!((s, v["id"].clone()), (StatusCode::OK, json!(id)), "{v}");
    assert_eq!(post.sent.lock().unwrap()[0].1["id"], json!(id));
    let (_, list) = call(&app, "GET", "/api/feedback/reports", None, "").await;
    assert_eq!((list[0]["sent"].clone(), list[0]["error"].clone()), (json!(true), json!("")));
    // bad requests
    let (s, _) = call(&app, "POST", "/api/feedback", Some(json!({"type": "Idea"})), &st.token).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&app, "POST", "/api/feedback/reports/SB-ZZZZZZ/retry", None, &st.token).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) =
        call(&app, "PUT", "/api/config", Some(json!({"feedback_url": "https://evil.example/x/exec"})), &st.token).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn end_to_end_through_the_stand_in_web_app() {
    let (app, st, _) = app("e2e").await;
    let s = script(None);
    st.feedback.set_poster(Arc::new(UreqPoster));
    call(&app, "PUT", "/api/config", Some(json!({"feedback_url": s.url})), &st.token).await;
    let (code, v) =
        call(&app, "POST", "/api/feedback", Some(json!({"type": "Question", "what": "how do I pair?"})), &st.token)
            .await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let seen = s.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["POST", "GET"]);
    let posted: Value = serde_json::from_str(&seen[0].2).unwrap();
    assert_eq!((posted["id"].clone(), posted["what"].clone()), (v["id"].clone(), json!("how do I pair?")));
}
