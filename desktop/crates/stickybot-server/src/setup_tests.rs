//! test_install_flow.py, test_installer.py (the dashboard parts) and the onboarding / restart /
//! window routes, against a pretend Mac (`FakeSystem`) and the fake stick.
use super::*;
use crate::Options;
use crate::sysrun::FakeSystem;
use axum::body::Body;
use axum::extract::Request;
use http_body_util::BodyExt;
use stick_link::{Backend, LinkState, fake::FakeStick};
use tower::ServiceExt;

const PORT: u16 = 8670;
const ORDER: [&str; 9] = ["welcome", "model", "claude", "hooks", "mcp", "github", "wispr", "stick", "done"];

struct T {
    app: Router,
    st: Arc<AppState>,
    sys: Arc<FakeSystem>,
}

impl T {
    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call("GET", path, None, false).await
    }
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call("POST", path, Some(body), true).await
    }
    async fn call(&self, method: &str, path: &str, body: Option<Value>, token: bool) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(path).header("host", format!("localhost:{PORT}"));
        if token {
            b = b.header("x-stick-token", self.st.token.clone());
        }
        let r = b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_default()).unwrap();
        let resp = self.app.clone().oneshot(r).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, v)
    }
    /// A job's events (the stream ends when the job is done).
    async fn events(&self, kind: &str, job: &Value) -> Vec<Value> {
        let (_, body) = self.get(&format!("/api/{kind}/jobs/{}", job.as_str().unwrap())).await;
        body.as_str()
            .unwrap()
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    async fn run(&self, step: &str, body: Value) -> Vec<Value> {
        let (s, v) = self.post(&format!("/api/install/{step}/run"), body).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        tokio::time::timeout(Duration::from_secs(10), self.events("install", &v["job"])).await.unwrap()
    }
    async fn component(&self, id: &str, action: &str) -> Vec<Value> {
        let (s, v) = self.post(&format!("/api/components/{id}/{action}"), json!({})).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        tokio::time::timeout(Duration::from_secs(10), self.events("components", &v["job"])).await.unwrap()
    }
    fn states(v: &Value) -> HashMap<String, String> {
        v["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["id"].as_str().unwrap().to_string(), s["state"].as_str().unwrap().to_string()))
            .collect()
    }
}

fn last(ev: &[Value]) -> &Value {
    ev.last().expect("events")
}

fn has_line(ev: &[Value], part: &str) -> bool {
    ev.iter().any(|e| e["line"].as_str().is_some_and(|l| l.contains(part)))
}

async fn app(name: &str, connect: bool) -> T {
    let home = std::env::temp_dir().join(format!("stickybot-rs-setup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let opts = Options { port: PORT, home: home.clone(), fake_stick: true, import_from: None };
    let st = AppState::new(&opts, Backend::Fake(FakeStick::new())).unwrap();
    // no real audio and no process taps: the wireless mic needs BlackHole here
    let ca = macos_audio::fake::FakeCoreAudio::new();
    let out = macos_audio::fake::FakeOutputs::new(vec![
        macos_audio::fake::FakeOutputs::speakers(),
        macos_audio::fake::FakeOutputs::blackhole(),
    ]);
    st.wmic.use_audio(macos_audio::Audio::fake(&ca, &out, false));
    let sys = Arc::new(FakeSystem::new(&home));
    st.setup.set_sys(sys.clone());
    // `claude mcp …` never runs for real either
    *st.claude_runner.lock().unwrap() = Arc::new(|_: &std::path::Path, args: &[String]| {
        if args.get(1).map(String::as_str) == Some("get") {
            (false, "No MCP server named stick".to_string())
        } else {
            (true, "ok".to_string())
        }
    });
    st.link.spawn();
    if connect {
        st.update_config(json!({"stick": {"address": "FAKE", "name": "fake"}}).as_object().unwrap().clone()).unwrap();
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected))
            .await
            .unwrap()
            .unwrap();
    }
    T { app: crate::api::router(st.clone()), st, sys }
}

#[tokio::test]
async fn steps_come_in_order_with_states() {
    let t = app("order", false).await;
    let (s, v) = t.get("/api/install").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["steps"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ORDER);
    let by: HashMap<&str, &Value> =
        v["steps"].as_array().unwrap().iter().map(|s| (s["id"].as_str().unwrap(), s)).collect();
    assert_eq!(by["welcome"]["state"], "done");
    assert!(!by["welcome"]["data"]["checks"].as_array().unwrap().is_empty());
    assert_eq!((by["model"]["state"].as_str(), by["model"]["kind"].as_str()), (Some("todo"), Some("recommended")));
    assert_eq!((by["github"]["state"].as_str(), &by["github"]["data"]["gh"]), (Some("todo"), &json!(false)));
    assert_eq!(by["wispr"]["data"]["pinned"], BLACKHOLE_PIN);
    assert_eq!(by["hooks"]["data"]["preview"]["hooks"].as_object().unwrap().len(), 3);
    assert_eq!(by["mcp"]["state"], "unavailable"); // no Claude Code
    assert_eq!((v["next"].as_str(), v["pending"].as_bool()), (Some("model"), Some(true)));
    assert!(by.values().all(|s| ["done", "todo", "skipped", "unavailable"].contains(&s["state"].as_str().unwrap())));
    // looking never changed anything
    assert!(!t.sys.settings.exists());
}

#[tokio::test]
async fn welcome_flags_a_mac_that_cant_run_it() {
    let t = app("welcome", false).await;
    *t.sys.mac.lock().unwrap() = ("x86_64".into(), String::new(), Some(10));
    let (_, s) = t.get("/api/install/welcome").await;
    let levels: HashMap<&str, &str> = s["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["label"].as_str().unwrap(), c["level"].as_str().unwrap()))
        .collect();
    assert_eq!((s["state"].as_str(), s["detail"].as_str()), (Some("unavailable"), Some("this Mac can't run it")));
    assert_eq!((levels["Apple Silicon"], levels["macOS"], levels["Free disk space"]), ("err", "err", "warn"));
}

#[tokio::test]
async fn skip_and_unskip_are_kept_in_the_config() {
    let t = app("skip", false).await;
    let (_, v) = t.post("/api/install/model/skip", json!({})).await;
    assert_eq!(T::states(&v)["model"], "skipped");
    assert_ne!(v["next"], "model");
    assert_eq!(v["skipped"], json!(["model"]));
    assert_eq!(crate::config::load(&t.st.paths)["install_skipped"], json!(["model"]));
    t.post("/api/install/model/skip", json!({})).await; // twice: still once
    assert_eq!(t.st.cfg.lock().unwrap()["install_skipped"], json!(["model"]));
    let (_, v) = t.post("/api/install/model/unskip", json!({})).await;
    assert_eq!(T::states(&v)["model"], "todo");
    assert_eq!(crate::config::load(&t.st.paths)["install_skipped"], json!([]));
}

#[tokio::test]
async fn pending_ignores_skipped_recommended_steps() {
    let t = app("pending", false).await;
    assert!(quick_pending(&t.st));
    for sid in ["model", "stick"] {
        t.post(&format!("/api/install/{sid}/skip"), json!({})).await;
    }
    let (_, v) = t.get("/api/install").await;
    assert_eq!(v["pending"], false);
    assert!(!quick_pending(&t.st));
}

#[tokio::test]
async fn unknown_steps_and_bad_actions() {
    let t = app("bad", false).await;
    assert_eq!(t.get("/api/install/nope").await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.post("/api/install/nope/skip", json!({})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.post("/api/install/nope/run", json!({})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.post("/api/install/model/fly", json!({})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.post("/api/install/welcome/skip", json!({})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.post("/api/install/stick/run", json!({})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.post("/api/install/wispr/run", json!({"carrier": "usb"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.get("/api/install/jobs/nope").await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.post("/api/install/jobs/nope/cancel", json!({})).await.0, StatusCode::NOT_FOUND);
    let (_, d) = t.get("/api/install/done").await;
    assert_eq!(d["id"], "done");
    assert!(d["data"]["summary"].is_object());
}

#[tokio::test]
async fn changes_need_the_token() {
    let t = app("token", false).await;
    for path in [
        "/api/install/model/skip",
        "/api/install/hooks/run",
        "/api/install/github/repo",
        "/api/onboarding",
        "/api/components/claude/install",
        "/api/restart",
        "/api/window",
    ] {
        assert_eq!(t.call("POST", path, Some(json!({})), false).await.0, StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(t.st.cfg.lock().unwrap()["install_skipped"], json!([]));
    assert!(!t.sys.settings.exists());
}

#[tokio::test]
async fn one_step() {
    let t = app("one", false).await;
    let (_, s) = t.get("/api/install/hooks").await;
    assert_eq!((s["id"].as_str(), s["state"].as_str()), (Some("hooks"), Some("todo")));
    assert!(s["runnable"] == true && s["skippable"] == true);
    assert_eq!(s["needs"], json!({}));
    let (_, s) = t.get("/api/install/model").await;
    assert_eq!(s["needs"]["download"], MODEL_SIZE);
}

#[tokio::test]
async fn hooks_step_adds_the_hooks_only_when_asked() {
    let t = app("hooks", false).await;
    t.get("/api/install").await;
    assert!(!t.sys.settings.exists()); // looking never changes anything
    let ev = t.run("hooks", json!({})).await;
    assert_eq!(last(&ev), &json!({"done": true, "ok": true, "error": null}));
    let saved = crate::hooks::read_settings(&t.sys.settings).unwrap();
    assert_eq!(crate::hooks::state(&saved, &t.sys.exe), (3, 3, 0));
    let cmd = saved["hooks"]["PermissionRequest"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.ends_with("--hook permission"), "{cmd}");
    assert_eq!(t.get("/api/install/hooks").await.1["state"], "done");
    // the Components card removes them again
    let ev = t.component("claude", "uninstall").await;
    assert_eq!(last(&ev)["ok"], true, "{ev:?}");
    assert_eq!(crate::hooks::state(&crate::hooks::read_settings(&t.sys.settings).unwrap(), &t.sys.exe), (0, 0, 0));
}

#[tokio::test]
async fn the_python_apps_hooks_are_left_alone_before_the_switch_over() {
    let t = app("pyhooks", false).await;
    let py = |s: &str| json!({"type": "command", "command": format!("/v/bin/python /x/stick_agent/{s}")});
    let settings = json!({"hooks": {"PermissionRequest": [{"hooks": [py("hook.py")]}],
                                    "UserPromptSubmit": [{"hooks": [py("claude_hooks.py")]}],
                                    "Stop": [{"hooks": [py("claude_hooks.py")]}]}});
    crate::hooks::write_settings(&t.sys.settings, &settings).unwrap();
    let (_, s) = t.get("/api/install/hooks").await;
    assert_eq!(s["state"], "done");
    assert!(s["detail"].as_str().unwrap().contains("StickyBot (Python)"));
    let ev = t.run("hooks", json!({})).await;
    assert_eq!(last(&ev)["ok"], false);
    assert!(last(&ev)["error"].as_str().unwrap().contains("switch-over"));
    assert_eq!(crate::hooks::read_settings(&t.sys.settings).unwrap(), settings); // untouched
    // a broken settings file is never overwritten
    std::fs::write(&t.sys.settings, "{nope").unwrap();
    assert_eq!(t.get("/api/install/hooks").await.1["state"], "unavailable");
    let ev = t.run("hooks", json!({})).await;
    assert_eq!(last(&ev)["ok"], false);
    assert_eq!(std::fs::read_to_string(&t.sys.settings).unwrap(), "{nope");
}

#[tokio::test]
async fn mcp_needs_claude_and_installs_through_claude_code() {
    let t = app("mcp", false).await;
    assert_eq!(t.get("/api/install/mcp").await.1["state"], "unavailable");
    *t.sys.claude.lock().unwrap() = Some("/Users/x/.local/bin/claude".into());
    let (_, s) = t.get("/api/install/mcp").await;
    assert_eq!(s["state"], "todo");
    assert!(s["data"]["command"].as_str().unwrap().contains("mcp add"));
    let ev = t.run("mcp", json!({})).await;
    assert_eq!(last(&ev)["ok"], true, "{ev:?}");
    assert!(has_line(&ev, "added"));
}

#[tokio::test]
async fn claude_step_states() {
    let t = app("claude", false).await;
    let (_, s) = t.get("/api/install/claude").await;
    assert_eq!((s["state"].as_str(), &s["data"]["installed"]), (Some("todo"), &json!(false)));
    assert_eq!(s["data"]["install_cmd"], CLAUDE_INSTALL);
    *t.sys.claude.lock().unwrap() = Some("/x/claude".into());
    t.sys.answer("claude --version", 0, "2.1.0 (Claude Code)\n");
    t.sys.answer("claude auth status", 1, "");
    assert_eq!(t.get("/api/install/claude").await.1["detail"], "installed (2.1.0 (Claude Code)), not logged in");
    t.sys.answer("claude auth status", 0, "{\"loggedIn\": true}");
    let (_, s) = t.get("/api/install/claude").await;
    assert_eq!((s["state"].as_str(), s["detail"].as_str()), (Some("done"), Some("2.1.0 (Claude Code) · logged in")));
    let ev = t.run("claude", json!({})).await;
    assert!(has_line(&ev, "already installed"));
}

#[tokio::test]
async fn the_claude_installer_runs_when_asked() {
    let t = app("claudeinst", false).await;
    t.sys.streams.lock().unwrap().insert("sh -c".into(), (vec!["Installing Claude Code...\n".into()], 0));
    let ev = t.run("claude", json!({})).await;
    assert!(has_line(&ev, "Installing Claude Code..."));
    // the pretend installer doesn't put claude anywhere
    assert!(last(&ev)["error"].as_str().unwrap().contains("isn't in ~/.local/bin"));
    assert!(t.sys.ran.lock().unwrap().iter().any(|r| r.contains(CLAUDE_INSTALL)));
}

#[tokio::test]
async fn repo_is_validated_and_sets_alerts_and_quick_commands() {
    let t = app("repo", false).await;
    for bad in ["", "acme", "acme/web/x", "../etc", "acme/..", "a b/c", "acme/web;rm"] {
        assert_eq!(t.post("/api/install/github/repo", json!({"repo": bad})).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, _) = t.post("/api/install/github/repo", json!({"repo": "a/b", "alerts": "yes"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = t.post("/api/install/github/repo", json!({"repo": "acme/web", "alerts": true})).await;
    assert_eq!((s, &v["ok"]), (StatusCode::OK, &json!(true)));
    let g = t.st.cfg.lock().unwrap()["github"].clone();
    assert_eq!((g["primary_repo"].as_str(), &g["on"]), (Some("acme/web"), &json!(true)));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(t.st.paths.commands()).unwrap()).unwrap();
    let urls: Vec<&str> = doc["commands"].as_array().unwrap().iter().filter_map(|c| c["open_url"].as_str()).collect();
    assert!(urls.contains(&"https://github.com/acme/web/pull/{n}"));
    assert!(urls.contains(&"https://app.devin.ai/review/acme/web/pull/{n}"));
    assert!(!urls.iter().any(|u| u.contains("<org>")));
    // a second change replaces the first; alerts None keeps the watcher as it is
    set(&t.st, json!({"github": {"on": false, "primary_repo": "", "interval_s": 300}})).unwrap();
    let g = set_primary_repo(&t.st, "other/app", None).unwrap();
    assert_eq!(g, json!({"on": false, "primary_repo": "other/app", "interval_s": 300}));
    let text = std::fs::read_to_string(t.st.paths.commands()).unwrap();
    assert!(text.contains("https://github.com/other/app/pull/{n}") && !text.contains("acme/web"));
    assert_eq!(onboarding_state(&t.st).await["steps"]["repo"], true);
}

#[test]
fn device_code_parsing() {
    let pipe = "! First copy your one-time code: 1A2B-3C4D\nOpen this URL to continue in your web browser: https://github.com/login/device\n";
    let tty = "\x1b[0;33m!\x1b[0m First copy your one-time code: \x1b[1mWXYZ-0987\x1b[0m\n\x1b[1mPress Enter\x1b[0m to open https://github.com/login/device in your browser... ";
    assert_eq!(parse_device_code(pipe), Some(("1A2B-3C4D".into(), GH_DEVICE_URL.into())));
    assert_eq!(parse_device_code(tty), Some(("WXYZ-0987".into(), GH_DEVICE_URL.into())));
    assert_eq!(parse_device_code("! First copy your one-time code: 1A2B"), None);
    assert_eq!(parse_device_code(""), None);
    assert_eq!(
        gh_login_args("gh"),
        ["gh", "auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web"]
    );
}

async fn gh_login(needs_enter: bool) {
    let t = app(if needs_enter { "ghtty" } else { "ghpipe" }, false).await;
    t.sys.program("gh");
    t.sys.answer("gh auth status", 1, "not logged in");
    let mut chunks = vec!["! First copy your one-time code: QWER-7890\n".to_string()];
    if needs_enter {
        chunks.push("Press Enter to open https://github.com/login/device in your browser... ".into());
    }
    chunks.push("\n✓ Authentication complete.\n".into());
    t.sys.streams.lock().unwrap().insert("gh auth login".into(), (chunks, 0));
    t.sys.after_stream.lock().unwrap().insert("gh auth status".into(), (0, "Logged in".into()));
    let ev = t.run("github", json!({})).await;
    assert!(ev.contains(&json!({"code": "QWER-7890", "url": GH_DEVICE_URL})), "{ev:?}");
    assert!(!has_line(&ev, "QWER-7890")); // shown big, not in the log
    assert!(has_line(&ev, "••••-••••"));
    assert_eq!(last(&ev)["ok"], true, "{ev:?}");
    assert_eq!(*t.sys.stdin.lock().unwrap(), if needs_enter { b"\n".to_vec() } else { Vec::new() });
    let (_, s) = t.get("/api/install/github").await;
    assert_eq!(&s["data"]["logged_in"], &json!(true));
    assert_eq!(&s["data"]["login"], &Value::Null);
    assert_eq!((s["state"].as_str(), s["detail"].as_str()), (Some("todo"), Some("logged in — pick your main repo")));
    t.post("/api/install/github/repo", json!({"repo": "acme/web", "alerts": true})).await;
    assert_eq!(t.get("/api/install/github").await.1["state"], "done");
}

#[tokio::test]
async fn gh_login_job_shows_the_code_and_finishes() {
    gh_login(false).await;
    gh_login(true).await;
}

#[tokio::test]
async fn gh_login_that_fails() {
    let t = app("ghfail", false).await;
    let ev = t.run("github", json!({})).await;
    assert!(last(&ev)["error"].as_str().unwrap().contains("brew install gh"));
    t.sys.program("gh");
    t.sys.streams.lock().unwrap().insert("gh auth login".into(), (vec!["error connecting to github.com\n".into()], 1));
    let ev = t.run("github", json!({})).await;
    let l = last(&ev);
    assert!(l["done"] == true && l["ok"] == false && l["error"].as_str().unwrap().contains("gh auth login"));
}

#[tokio::test]
async fn model_step_follows_the_download() {
    let t = app("model", false).await;
    let ev = t.run("model", json!({})).await;
    assert!(has_line(&ev, "is ready"), "{ev:?}");
    assert_eq!(last(&ev)["ok"], true);
    let ev = t.run("model", json!({})).await; // loaded already
    assert!(has_line(&ev, "already"));
}

#[tokio::test]
async fn wispr_step_turns_the_mic_on() {
    let t = app("wispr", false).await;
    let (_, s) = t.get("/api/install/wispr").await;
    assert_eq!((&s["data"]["tap_mode"], &s["data"]["blackhole"]), (&json!(false), &json!(false)));
    assert!(s["detail"].as_str().unwrap().contains(BLACKHOLE_CASK));
    let ev = t.run("wispr", json!({})).await;
    assert!(last(&ev)["error"].as_str().unwrap().contains(BLACKHOLE_CASK));
    assert_eq!(t.st.cfg.lock().unwrap()["wireless_mic"], false);
    let ev = t.run("wispr", json!({"carrier": "tap"})).await;
    assert!(last(&ev)["error"].as_str().unwrap().contains("14.2"));
    t.sys.blackhole.store(true, Ordering::SeqCst);
    let ev = t.run("wispr", json!({})).await;
    assert_eq!(last(&ev)["ok"], true, "{ev:?}");
    let c = t.st.cfg.lock().unwrap().clone();
    assert_eq!((&c["wireless_mic"], c["wireless_mic_carrier"].as_str()), (&json!(true), Some("auto")));
    let (_, s) = t.get("/api/install/wispr").await;
    assert_eq!(s["state"], "done");
    assert_eq!(s["needs"]["terminal"], BLACKHOLE_CASK);
    // the Components card turns it off
    t.component("wireless_mic", "uninstall").await;
    assert_eq!(t.st.cfg.lock().unwrap()["wireless_mic"], false);
}

#[tokio::test]
async fn a_running_job_can_be_cancelled() {
    let t = app("cancel", false).await;
    t.sys.program("gh");
    // a login that takes long (the pretend stream stops when cancelled)
    let chunks: Vec<String> = (0..200_000).map(|_| "waiting\n".to_string()).collect();
    t.sys.streams.lock().unwrap().insert("gh auth login".into(), (chunks, 0));
    let (_, v) = t.post("/api/install/github/run", json!({})).await;
    let job = v["job"].as_str().unwrap().to_string();
    let (s, v2) = t.post("/api/install/github/run", json!({})).await;
    if s == StatusCode::CONFLICT {
        assert_eq!(v2["job"], job.as_str());
        assert_eq!(t.post(&format!("/api/install/jobs/{job}/cancel"), json!({})).await.0, StatusCode::OK);
    }
    let ev = tokio::time::timeout(Duration::from_secs(30), t.events("install", &json!(job))).await.unwrap();
    assert_eq!(last(&ev)["done"], true);
    assert_eq!(t.post(&format!("/api/install/jobs/{job}/cancel"), json!({})).await.0, StatusCode::CONFLICT);
}

// --- components ------------------------------------------------------------------------------------------

#[tokio::test]
async fn components_listing_and_errors() {
    let t = app("comps", false).await;
    let (s, v) = t.get("/api/components").await;
    assert_eq!(s, StatusCode::OK);
    let list = v.as_array().unwrap();
    let ids: Vec<&str> = list.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["core", "model", "claude", "github", "firmware", "wireless_mic", "mcp"]);
    for c in list {
        for k in ["title", "description", "status", "detail", "needs", "from_dashboard", "removable", "default"] {
            assert!(c.get(k).is_some(), "{k} in {c}");
        }
    }
    assert_eq!((list[0]["from_dashboard"].as_bool(), list[2]["status"].as_str()), (Some(false), Some("missing")));
    assert_eq!(t.post("/api/components/nope/install", json!({})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.post("/api/components/core/install", json!({})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.post("/api/components/firmware/uninstall", json!({})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.post("/api/components/claude/fly", json!({})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.get("/api/components/jobs/nope").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn component_job_streams_and_installs() {
    let t = app("compjob", false).await;
    let ev = t.component("claude", "install").await;
    assert_eq!(last(&ev), &json!({"done": true, "ok": true, "error": null}));
    assert!(has_line(&ev, "hooks added"));
    assert_eq!(t.get("/api/components").await.1[2]["status"], "installed");
    // GitHub: not available without gh, reported as the job's error
    let ev = t.component("github", "install").await;
    assert!(last(&ev)["error"].as_str().unwrap().contains("brew install gh"));
    t.sys.program("gh");
    t.sys.answer("gh auth status", 0, "");
    assert_eq!(last(&t.component("github", "install").await)["ok"], true);
    assert_eq!(t.st.cfg.lock().unwrap()["github"]["on"], true);
    t.component("github", "uninstall").await;
    assert_eq!(t.st.cfg.lock().unwrap()["github"]["on"], false);
}

// --- onboarding, restart, window -------------------------------------------------------------------------

#[tokio::test]
async fn onboarding_marks_and_reset() {
    let t = app("onboard", true).await;
    let (_, v) = t.get("/api/onboarding").await;
    assert_eq!(v["steps"].as_object().unwrap().len(), 6);
    assert_eq!((&v["steps"]["paired"], &v["steps"]["picked"]), (&json!(true), &json!(true)));
    assert_eq!(v["steps"]["firmware"], true); // no bundle here: nothing to update to
    assert_eq!(v["steps"]["tested"], false);
    let (_, v) = t.post("/api/onboarding", json!({"step": "tested", "done": true})).await;
    assert_eq!((&v["steps"]["tested"], &v["done"]), (&json!(true), &json!(false)));
    let (_, v) = t.post("/api/onboarding", json!({"step": "paired", "done": false})).await;
    assert_eq!(v["steps"]["paired"], false);
    assert_eq!(t.st.cfg.lock().unwrap()["onboarding"], json!({"tested": true, "paired": false}));
    let (_, v) = t.post("/api/onboarding", json!({"step": "reset"})).await;
    assert_eq!((&v["steps"]["paired"], &v["steps"]["tested"]), (&json!(true), &json!(false)));
    let (s, v) = t.post("/api/onboarding", json!({"step": "fly"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("firmware, paired"));
    // a spoken request counts as tried
    t.st.history(&json!({"t": 1, "heard": "hi", "source": "voice"}));
    assert_eq!(t.get("/api/onboarding").await.1["steps"]["tested"], true);
    let (_, s) = t.get("/api/install/stick").await;
    assert_eq!((s["state"].as_str(), s["detail"].as_str()), (Some("done"), Some("using fake")));
}

#[tokio::test]
async fn restart_and_window() {
    let t = app("appctl", false).await;
    // headless: no window, no restart
    assert_eq!(t.post("/api/window", json!({"page": "install"})).await.1, json!({"ok": false}));
    let (s, v) = t.post("/api/restart", json!({})).await;
    assert_eq!((s, &v["ok"]), (StatusCode::CONFLICT, &json!(false)));
    let fake = Arc::new(crate::appctl::FakeApp::default());
    *t.st.setup.app.lock().unwrap() = fake.clone();
    assert_eq!(t.post("/api/restart", json!({})).await.1, json!({"ok": true}));
    assert_eq!(*fake.restarts.lock().unwrap(), 1);
    assert_eq!(t.post("/api/window", json!({"page": "install"})).await.1, json!({"ok": true}));
    assert_eq!(t.post("/api/window", json!({})).await.1, json!({"ok": true}));
    assert_eq!(t.post("/api/window", json!({"page": "../x"})).await.1, json!({"ok": false}));
    assert_eq!(t.post("/api/window", json!({"page": "a b"})).await.1, json!({"ok": false}));
    assert_eq!(*fake.windows.lock().unwrap(), vec!["install".to_string(), String::new()]);
}
