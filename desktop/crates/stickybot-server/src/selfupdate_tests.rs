use super::*;
use crate::Options;
use crate::appctl::FakeApp;
use crate::releases::Source;
use crate::releases::tests::FakeGitHub;
use axum::body::Body;
use axum::extract::Request;
use http_body_util::BodyExt;
use std::path::PathBuf;
use stick_link::{Backend, LinkState, fake::FakeStick};
use tower::ServiceExt;

/// An app version no real release reaches, so "newer" holds without touching the current one.
const NEWER: &str = "99.0.0";

// --- pure pieces ------------------------------------------------------------------------------------------------------

fn lat(version: &str, app: bool, updater: bool) -> Value {
    json!({"tag": format!("v{version}"), "version": version, "app_asset": app.then_some("StickyBot-x-macos-arm64.zip"),
           "updater_asset": updater.then_some("latest.json")})
}

fn plan_of(l: Option<&Value>, current: &str) -> Result<Plan, Refusal> {
    plan(l, current, DEFAULT_REPO, "stable", None, None, false)
}

#[test]
fn stable_and_beta_read_different_update_files() {
    let stable = endpoint(DEFAULT_REPO, "stable", "v2.7.0", None, false).unwrap();
    assert_eq!(stable, "https://github.com/4shishkumar/stickybot/releases/latest/download/latest.json");
    // stable never looks at the tag: GitHub's "latest" is never a prerelease
    assert_eq!(endpoint(DEFAULT_REPO, "stable", "../x", None, false).unwrap(), stable);
    let beta = endpoint(DEFAULT_REPO, "beta", "v2.7.0-beta.1", None, false).unwrap();
    assert_eq!(beta, "https://github.com/4shishkumar/stickybot/releases/download/v2.7.0-beta.1/latest.json");
    // a stable release picked while on beta is simply its own latest.json
    assert!(endpoint(DEFAULT_REPO, "beta", "v2.7.0", None, false).unwrap().ends_with("/download/v2.7.0/latest.json"));
    // the tag goes into a URL: only what a tag is made of
    for bad in ["", "v1/../../x", "v1?x=y", "v1 2", "v1#a", "v1%2f"] {
        assert!(endpoint(DEFAULT_REPO, "beta", bad, None, false).is_err(), "{bad:?}");
    }
}

#[test]
fn nothing_but_a_local_server_in_a_debug_build_changes_the_endpoint() {
    let local = "http://127.0.0.1:9123/latest.json";
    // a debug build or a test: a local fake release server
    assert_eq!(endpoint(DEFAULT_REPO, "stable", "v1.0.0", Some(local), true).unwrap(), local);
    assert_eq!(
        endpoint(DEFAULT_REPO, "stable", "v1.0.0", Some("http://localhost:8/x"), true).unwrap(),
        "http://localhost:8/x"
    );
    // anything else is ignored, with the real endpoint instead
    for evil in [
        "https://evil.example/latest.json",
        "http://evil.example/latest.json",
        "http://127.0.0.1:80@evil.example/x",
        "http://127.0.0.1/x",
        "https://127.0.0.1:9/x",
        "http://localhost.evil.example:80/x",
        "",
    ] {
        let e = endpoint(DEFAULT_REPO, "stable", "v1.0.0", Some(evil), true).unwrap();
        assert!(e.starts_with("https://github.com/4shishkumar/stickybot/"), "{evil}: {e}");
    }
    // a release build ignores even a local one
    let e = endpoint(DEFAULT_REPO, "stable", "v1.0.0", Some(local), false).unwrap();
    assert!(e.starts_with("https://github.com/4shishkumar/stickybot/"), "{e}");
    // another repo (STICKYBOT_RS_REPO) turns in-app install off instead of following it
    let why = endpoint("acme/rocket", "stable", "v1.0.0", None, false).unwrap_err();
    assert!(why.contains("another repo"), "{why}");
    assert!(local_url(local) && !local_url("http://127.0.0.1:99999/x"));
}

#[test]
fn every_reason_to_wait_is_named_and_being_paused_is_not_one() {
    assert_eq!(busy_reason(&Busy::default()), None);
    let one = |f: fn(&mut Busy)| {
        let mut b = Busy::default();
        f(&mut b);
        busy_reason(&b).unwrap()
    };
    assert!(one(|b| b.firmware = true).contains("firmware update"));
    assert!(one(|b| b.usb_flash = true).contains("flashed over USB"));
    assert!(one(|b| b.voice = true).contains("voice request"));
    assert!(one(|b| b.prompt = true).contains("permission prompt"));
    assert!(one(|b| b.asking = true).contains("question"));
    assert!(one(|b| b.wireless_mic = true).contains("wireless mic"));
    assert!(one(|b| b.face_install = true).contains("face"));
}

#[test]
fn errors_become_sentences_with_a_short_kind() {
    let all = [
        (InstallError::Signature("x".into()), "signature", "signing key"),
        (InstallError::Network("x".into()), "network", "no connection"),
        (
            InstallError::NotWritable("/Applications/StickyBot.app".into()),
            "not-writable",
            "/Applications/StickyBot.app",
        ),
        (InstallError::NoUpdate(String::new()), "no-update", "no newer StickyBot"),
        (InstallError::Other("odd".into()), "other", "odd"),
    ];
    for (e, kind, word) in all {
        assert_eq!(e.kind(), kind);
        assert!(e.words().contains(word), "{}", e.words());
        assert!(e.words().ends_with(['.', ')']) || e.words() == "odd", "{}", e.words());
    }
    assert_eq!(
        InstallError::Signature(String::new()).words(),
        "The download didn't match StickyBot's signing key, so it was not installed."
    );
}

#[test]
fn what_may_be_installed() {
    let ok = plan_of(Some(&lat("2.6.0", true, true)), "2.5.0").unwrap();
    assert_eq!((ok.version.as_str(), ok.tag.as_str()), ("2.6.0", "v2.6.0"));
    assert!(ok.endpoint.ends_with("/releases/latest/download/latest.json"));
    // nothing found yet / a release that isn't a Mac app
    assert_eq!(plan_of(None, "2.5.0"), Err(Refusal::NothingNew));
    assert_eq!(plan_of(Some(&lat("2.6.0", false, true)), "2.5.0"), Err(Refusal::NothingNew));
    // never a lower or an equal version
    for release in ["2.5.0", "2.4.9", "2.5.0-beta.1"] {
        let r = plan_of(Some(&lat(release, true, true)), "2.5.0");
        assert!(matches!(r, Err(Refusal::NotNewer { .. })), "{release}: {r:?}");
    }
    assert!(plan_of(Some(&lat("2.6.0", true, true)), "2.6.0").unwrap_err().words().contains("isn't newer"));
    // a release without its update file keeps the download page
    assert_eq!(plan_of(Some(&lat("2.6.0", true, false)), "2.5.0"), Err(Refusal::NoUpdateFile));
    // the installer's word comes first-hand
    let l = lat("2.6.0", true, true);
    let r = plan(
        Some(&l),
        "2.5.0",
        DEFAULT_REPO,
        "stable",
        Some("StickyBot is running from a temporary copy".into()),
        None,
        false,
    );
    assert_eq!(r.unwrap_err().words(), "StickyBot is running from a temporary copy.");
    // another repo
    assert!(matches!(plan(Some(&l), "2.5.0", "acme/x", "stable", None, None, false), Err(Refusal::Unavailable(_))));
    // beta: the picked tag's own file
    let beta = plan(Some(&lat("2.7.0-beta.1", true, true)), "2.5.0", DEFAULT_REPO, "beta", None, None, false).unwrap();
    assert!(beta.endpoint.ends_with("/releases/download/v2.7.0-beta.1/latest.json"));
}

#[test]
fn pending_against_the_version_that_runs() {
    let p = |to: &str, at: f64| Pending { from: "2.5.0".into(), to: to.into(), at };
    assert_eq!(resolve_pending(None, "2.6.0", 100.0), Outcome::Nothing);
    assert_eq!(
        resolve_pending(Some(&p("2.6.0", 90.0)), "2.6.0", 100.0),
        Outcome::Updated { from: "2.5.0".into(), to: "2.6.0".into() }
    );
    assert_eq!(
        resolve_pending(Some(&p("2.6.0", 90.0)), "2.5.0", 100.0),
        Outcome::DidntTake { to: "2.6.0".into(), now: "2.5.0".into() }
    );
    // an install that died a week ago has nothing to do with this start
    assert_eq!(resolve_pending(Some(&p("2.6.0", 0.0)), "2.5.0", 7.0 * 86400.0), Outcome::Nothing);
}

#[test]
fn the_automatic_install_needs_everything_at_once() {
    let ready = AutoFacts { on: true, check_on: true, can_install: true, idle_s: Some(660.0), ..Default::default() };
    assert!(auto_due(&ready));
    assert!(!auto_due(&AutoFacts { idle_s: None, ..ready.clone() }), "unknown idle time is not idle");
    assert!(!auto_due(&AutoFacts { on: false, ..ready.clone() }));
    assert!(!auto_due(&AutoFacts { check_on: false, ..ready.clone() }));
    assert!(!auto_due(&AutoFacts { can_install: false, ..ready.clone() }));
    assert!(!auto_due(&AutoFacts { running: true, ..ready.clone() }));
    assert!(!auto_due(&AutoFacts { busy: true, ..ready.clone() }));
    assert!(!auto_due(&AutoFacts { idle_s: Some(120.0), ..ready.clone() }), "2 minutes since a button");
    assert!(!auto_due(&AutoFacts { idle_s: Some(599.0), ..ready.clone() }));
    assert!(!auto_due(&AutoFacts { tried_s: Some(3600.0), ..ready.clone() }), "tried an hour ago");
    assert!(auto_due(&AutoFacts { tried_s: Some(25.0 * 3600.0), ..ready }));
}

#[test]
fn the_phases_and_what_they_are_called() {
    assert_eq!(Phase::Downloading.state(), Some("downloading"));
    assert_eq!(Phase::Installing.state(), Some("installing"));
    assert_eq!(Phase::Restarting.state(), Some("restarting"));
    assert_eq!(Phase::Error.state(), Some("error"));
    // waiting is `available` + a reason, not an eighth state
    assert_eq!((Phase::Waiting.state(), Phase::Idle.state()), (None, None));
    assert!(Phase::Waiting.running() && !Phase::Error.running() && !Phase::Idle.running());
}

#[test]
fn progress_is_throttled_but_the_last_one_always_goes() {
    let run = Run::load(None);
    assert!(run.begin().is_some());
    assert!(run.begin().is_none(), "one install at a time");
    assert!(run.progress(10, Some(100)), "the first");
    assert!(!run.progress(20, Some(100)), "within 200 ms");
    assert!(!run.progress(30, Some(100)));
    assert!(run.progress(100, Some(100)), "the last");
    let f = run.fields("https://example.invalid/");
    assert_eq!(f["progress"], json!({"bytes": 100, "total": 100}));
    // no total: indeterminate, still reported
    let run = Run::load(None);
    run.begin();
    run.progress(5, None);
    assert_eq!(run.fields("")["progress"], json!({"bytes": 5, "total": null}));
}

#[test]
fn cancel_drops_a_download_and_dismisses_an_error_but_not_a_replace() {
    let run = Run::load(None);
    assert!(!run.cancel(), "nothing to cancel");
    run.begin();
    assert!(run.cancel() && run.cancelled());
    run.set_phase(Phase::Installing);
    assert!(!run.cancel(), "too late: the bundle is being replaced");
    run.fail("other", "x".into());
    assert_eq!(run.phase(), Phase::Error);
    assert_eq!(run.fields("p")["install_error"], json!({"kind": "other", "message": "x", "download": "p"}));
    assert!(run.cancel());
    assert_eq!(run.phase(), Phase::Idle);
    assert_eq!(run.fields("p")["install_error"], Value::Null);
    // try again after an error
    run.fail("other", "x".into());
    assert!(run.begin().is_some());
    assert_eq!(run.fields("p")["install_error"], Value::Null);
}

#[test]
fn pending_and_tries_survive_a_restart() {
    let run = Run::load(None);
    run.set_pending(Some(Pending { from: "2.5.0".into(), to: "2.6.0".into(), at: 5.0 }));
    run.mark_tried("2.6.0");
    let mut saved = Map::new();
    run.save_into(&mut saved);
    let again = Run::load(Some(&Value::Object(saved)));
    assert!(again.tried_at("2.6.0").is_some());
    assert!(matches!(
        again.s.lock().unwrap().pending.as_ref(),
        Some(p) if p.to == "2.6.0" && p.from == "2.5.0"
    ));
}

// --- the install, with a fake installer and a fake app ----------------------------------------------------------------

struct T {
    st: Arc<AppState>,
    inst: Arc<FakeInstaller>,
    app: Arc<FakeApp>,
    fake: FakeStick,
    home: PathBuf,
}

fn home(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("stickybot-rs-selfupdate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    h
}

fn state_in(home: &std::path::Path) -> (Arc<AppState>, FakeStick) {
    let opts = Options { port: 8670, home: home.to_path_buf(), fake_stick: true, import_from: None };
    let fake = FakeStick::new();
    (AppState::new(&opts, Backend::Fake(fake.clone())).unwrap(), fake)
}

async fn connect(st: &Arc<AppState>) {
    st.link.spawn();
    st.update_config(json!({"stick": {"address": "FAKE", "name": "fake"}}).as_object().unwrap().clone()).unwrap();
    st.link.set_target(st.stick()); // a restart: the saved stick, as `start` does
    let mut w = st.link.watch_state();
    tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected)).await.unwrap().unwrap();
}

/// An app with a fake installer and a fake app control; `tag` (when given) is the release the check finds
/// (with the Mac app, and with its update file when `updater`).
async fn setup(name: &str, tag: Option<&str>, updater: bool) -> T {
    let home = home(name);
    let (st, fake) = state_in(&home);
    let inst = FakeInstaller::new(tag.unwrap_or(NEWER).trim_start_matches('v'));
    st.updates.run.set_installer(inst.clone());
    let app = Arc::new(FakeApp::default());
    *st.setup.app.lock().unwrap() = app.clone();
    st.updates.run.timings.lock().unwrap().poll = Duration::from_millis(20);
    if let Some(tag) = tag {
        let tag = tag.to_string();
        st.updates.set_sources(Arc::new(move || {
            let mut g = FakeGitHub::default();
            g.add_release(&tag, "9.9.0", false);
            g.add_app(&tag);
            if updater {
                g.add_updater(&tag);
            }
            vec![Box::new(g) as Box<dyn Source>]
        }));
        let cfg = st.cfg.lock().unwrap().clone();
        st.updates.check(&cfg).await;
    }
    T { st, inst, app, fake, home }
}

async fn offered(name: &str) -> T {
    setup(name, Some(&format!("v{NEWER}")), true).await
}

async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("gave up waiting for: {what}");
}

fn status(st: &AppState) -> Value {
    st.updates.full(&st.cfg.lock().unwrap().clone())
}

/// The `state` of every `update` event the hub saw since `rx` was made, without repeats.
fn states(rx: &mut tokio::sync::broadcast::Receiver<Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if ev["kind"] == "update" {
            let s = ev["state"].as_str().unwrap_or("?").to_string();
            if out.last() != Some(&s) {
                out.push(s);
            }
        }
    }
    out
}

fn events_log(home: &std::path::Path) -> String {
    std::fs::read_to_string(home.join("events.jsonl")).unwrap_or_default()
}

#[tokio::test]
async fn an_install_runs_to_the_end_and_restarts_once() {
    let t = offered("flow").await;
    connect(&t.st).await;
    let (_, mut rx) = t.st.hub.subscribe(0);
    let s = status(&t.st);
    assert_eq!((s["state"].as_str(), s["install"]["can_update"].as_bool()), (Some("available"), Some(true)));
    start(&t.st, "test").unwrap();
    assert_eq!(status(&t.st)["state"], "downloading");
    until("the restart", || *t.app.restarts.lock().unwrap() == 1).await;
    assert_eq!(states(&mut rx), ["downloading", "installing", "restarting"]);
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(status(&t.st)["state"], "restarting");
    // it asked for the stable file of this repo, and nothing else
    assert_eq!(
        *t.inst.endpoints.lock().unwrap(),
        ["https://github.com/4shishkumar/stickybot/releases/latest/download/latest.json"]
    );
    // the stick was told before the replace, and said goodbye before the relaunch
    assert!(events_log(&t.home).contains("Updating StickyBot"));
    let wrote = t.fake.writes().iter().map(|w| String::from_utf8_lossy(w).to_string()).collect::<String>();
    assert!(wrote.contains("\"bye\""), "{wrote}");
    // what the restart will find
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(t.home.join("updates.json")).unwrap()).unwrap();
    assert_eq!(
        (saved["pending"]["to"].as_str(), saved["pending"]["from"].as_str()),
        (Some(NEWER), Some(env!("CARGO_PKG_VERSION")))
    );
    assert!(saved["tried"][NEWER].is_number());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(*t.app.restarts.lock().unwrap(), 1, "once");
}

#[tokio::test]
async fn progress_reaches_the_pages_while_downloading() {
    let t = offered("progress").await;
    *t.inst.delay.lock().unwrap() = Duration::from_millis(600);
    start(&t.st, "test").unwrap();
    until("some progress", || status(&t.st)["progress"]["bytes"].as_u64().is_some_and(|b| b >= 1000)).await;
    let s = status(&t.st);
    assert_eq!((s["state"].as_str(), s["progress"]["total"].as_u64()), (Some("downloading"), Some(3000)));
    until("done", || *t.app.restarts.lock().unwrap() == 1).await;
    assert_eq!(status(&t.st)["progress"], Value::Null);
}

#[tokio::test]
async fn a_second_install_while_one_runs_is_refused() {
    let t = offered("second").await;
    *t.inst.delay.lock().unwrap() = Duration::from_millis(300);
    start(&t.st, "test").unwrap();
    assert_eq!(start(&t.st, "test"), Err(Refusal::Running));
    assert!(Refusal::Running.words().contains("already"));
    until("done", || *t.app.restarts.lock().unwrap() == 1).await;
    assert_eq!(t.inst.endpoints.lock().unwrap().len(), 1, "downloaded once");
}

#[tokio::test]
async fn nothing_is_downloaded_when_it_must_not_be_installed() {
    // no check yet
    let t = setup("none", None, true).await;
    assert_eq!(start(&t.st, "test"), Err(Refusal::NothingNew));
    // a release that is this very version, and an older one
    for (name, tag) in [("equal", format!("v{}", env!("CARGO_PKG_VERSION"))), ("older", "v1.0.0".to_string())] {
        let t = setup(name, Some(&tag), true).await;
        let s = status(&t.st);
        assert_eq!(
            (s["agent_update"].as_bool(), s["install"]["can_update"].as_bool()),
            (Some(false), Some(false)),
            "{tag}"
        );
        assert!(matches!(start(&t.st, "test"), Err(Refusal::NotNewer { .. })), "{tag}");
        assert!(t.inst.endpoints.lock().unwrap().is_empty(), "{tag}");
    }
    // a release cut without its update file
    let t = setup("nofile", Some(&format!("v{NEWER}")), false).await;
    let s = status(&t.st);
    assert_eq!((s["agent_update"].as_bool(), s["install"]["can_update"].as_bool()), (Some(true), Some(false)));
    assert!(s["install"]["why"].as_str().unwrap().contains("no update file"));
    assert_eq!(s["latest"]["updater_asset"], Value::Null);
    assert_eq!(start(&t.st, "test"), Err(Refusal::NoUpdateFile));
    // an installer that can't here (translocated, read-only, not a bundle)
    let t = offered("unavailable").await;
    *t.inst.why.lock().unwrap() = Some("StickyBot is running from a temporary read-only copy".into());
    let s = status(&t.st);
    assert_eq!(s["install"]["can_update"], false);
    assert!(s["install"]["why"].as_str().unwrap().contains("temporary read-only copy"));
    assert!(matches!(start(&t.st, "test"), Err(Refusal::Unavailable(_))));
    assert!(t.inst.endpoints.lock().unwrap().is_empty());
    assert_eq!(status(&t.st)["state"], "available");
}

#[tokio::test]
async fn an_update_that_is_not_the_announced_one_is_not_installed() {
    let t = offered("mismatch").await;
    *t.inst.version.lock().unwrap() = "2.5.9".into(); // the check said 99.0.0
    start(&t.st, "test").unwrap();
    until("error", || status(&t.st)["state"] == "error").await;
    let e = status(&t.st)["install_error"].clone();
    assert_eq!(e["kind"], "other");
    assert!(e["message"].as_str().unwrap().contains("not the v99.0.0 that was announced"), "{e}");
    assert!(e["download"].as_str().unwrap().starts_with("https://"));
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(*t.app.restarts.lock().unwrap(), 0);
    assert!(!events_log(&t.home).contains("Updating StickyBot"), "no toast: nothing was going to be replaced");
    // an older one is just as wrong
    *t.inst.version.lock().unwrap() = "1.0.0".into();
    start(&t.st, "test").unwrap();
    until("error again", || status(&t.st)["install_error"]["message"].as_str().is_some_and(|m| m.contains("v1.0.0")))
        .await;
}

#[tokio::test]
async fn a_bad_signature_replaces_nothing_and_says_so() {
    let t = offered("signature").await;
    *t.inst.download_fails.lock().unwrap() = Some(InstallError::Signature("minisign".into()));
    start(&t.st, "test").unwrap();
    until("error", || status(&t.st)["state"] == "error").await;
    let e = status(&t.st)["install_error"].clone();
    assert_eq!(e["kind"], "signature");
    assert!(e["message"].as_str().unwrap().contains("signing key"));
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(*t.app.restarts.lock().unwrap(), 0);
    // Try again works once the download is fine
    *t.inst.download_fails.lock().unwrap() = None;
    start(&t.st, "test").unwrap();
    until("restart", || *t.app.restarts.lock().unwrap() == 1).await;
    assert_eq!(status(&t.st)["install_error"], Value::Null);
}

#[tokio::test]
async fn a_failed_replace_forgets_what_it_was_about_to_do() {
    let t = offered("replacefail").await;
    *t.inst.install_fails.lock().unwrap() = Some(InstallError::NotWritable("/Applications/StickyBot.app".into()));
    start(&t.st, "test").unwrap();
    until("error", || status(&t.st)["state"] == "error").await;
    assert_eq!(status(&t.st)["install_error"]["kind"], "not-writable");
    assert_eq!(*t.app.restarts.lock().unwrap(), 0);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(t.home.join("updates.json")).unwrap()).unwrap();
    assert_eq!(saved["pending"], Value::Null);
}

#[tokio::test]
async fn it_waits_while_a_firmware_update_runs_and_goes_on_by_itself() {
    let t = offered("gate").await;
    connect(&t.st).await;
    let job = crate::firmware::Job::new();
    t.st.jobs.add(job.clone());
    start(&t.st, "test").unwrap();
    until("waiting", || status(&t.st)["waiting"].is_string()).await;
    let s = status(&t.st);
    assert_eq!(s["state"], "available", "verified, not yet installing");
    assert_eq!(s["waiting"], "busy — will install when idle: a firmware update is running");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 0, "nothing replaced while it runs");
    assert_eq!(*t.app.restarts.lock().unwrap(), 0);
    job.finish(json!({"done": true, "ok": true}));
    until("the restart", || *t.app.restarts.lock().unwrap() == 1).await;
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(status(&t.st)["waiting"], Value::Null);
}

#[tokio::test]
async fn being_paused_does_not_hold_an_update_back() {
    let t = offered("paused").await;
    connect(&t.st).await;
    t.st.link.pause(true);
    until("paused", || t.st.link.paused()).await;
    assert_eq!(busy_reason(&Busy::of(&t.st)), None);
    start(&t.st, "test").unwrap();
    until("the restart", || *t.app.restarts.lock().unwrap() == 1).await;
}

#[tokio::test]
async fn a_prompt_on_the_stick_holds_it_too() {
    let t = offered("prompt").await;
    let b = Busy { prompt: true, ..Default::default() };
    assert!(busy_reason(&b).is_some());
    // busy() of a real app state sees the approvals and the jobs
    assert_eq!(busy_reason(&Busy::of(&t.st)), None);
    t.st.jobs.add(crate::firmware::Job::new());
    assert!(Busy::of(&t.st).firmware);
}

#[tokio::test]
async fn cancel_drops_a_waiting_install() {
    let t = offered("cancelwait").await;
    t.st.jobs.add(crate::firmware::Job::new()); // never ends
    start(&t.st, "test").unwrap();
    until("waiting", || status(&t.st)["waiting"].is_string()).await;
    assert!(cancel(&t.st));
    until("idle", || t.st.updates.run.phase() == Phase::Idle).await;
    let s = status(&t.st);
    assert_eq!((s["state"].as_str(), s["waiting"].clone()), (Some("available"), Value::Null));
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(!cancel(&t.st), "nothing left to cancel");
}

#[tokio::test]
async fn cancel_stops_a_download() {
    let t = offered("canceldl").await;
    *t.inst.delay.lock().unwrap() = Duration::from_secs(30);
    start(&t.st, "test").unwrap();
    until("downloading", || t.st.updates.run.phase() == Phase::Downloading).await;
    assert!(cancel(&t.st));
    until("idle", || t.st.updates.run.phase() == Phase::Idle).await;
    assert_eq!(*t.app.restarts.lock().unwrap(), 0);
}

#[tokio::test]
async fn waiting_too_long_gives_up_in_words() {
    let t = offered("giveup").await;
    t.st.updates.run.timings.lock().unwrap().max_wait = Duration::from_millis(120);
    t.st.jobs.add(crate::firmware::Job::new());
    start(&t.st, "test").unwrap();
    until("error", || status(&t.st)["state"] == "error").await;
    assert!(status(&t.st)["install_error"]["message"].as_str().unwrap().contains("still busy"));
    assert_eq!(*t.app.restarts.lock().unwrap(), 0);
}

#[tokio::test]
async fn the_headless_app_says_where_to_download_instead() {
    let t = offered("headless").await;
    t.st.updates.run.set_installer(Arc::new(NoInstaller));
    let s = status(&t.st);
    assert_eq!((s["agent_update"].as_bool(), s["install"]["can_update"].as_bool()), (Some(true), Some(false)));
    assert!(s["install"]["why"].as_str().unwrap().contains("Only the menu bar app updates itself"));
    assert_eq!(s["install"]["download"], format!("https://example.invalid/v{NEWER}"));
    assert!(matches!(start(&t.st, "test"), Err(Refusal::Unavailable(_))));
}

#[tokio::test]
async fn a_relaunch_that_fails_says_the_update_is_in_place() {
    struct NoRestart;
    impl crate::appctl::AppControl for NoRestart {
        fn restart(&self) -> Result<(), String> {
            Err("no window server".into())
        }
        fn show_window(&self, _p: &str) -> bool {
            false
        }
    }
    let t = offered("norestart").await;
    *t.st.setup.app.lock().unwrap() = Arc::new(NoRestart);
    start(&t.st, "test").unwrap();
    until("error", || status(&t.st)["state"] == "error").await;
    assert!(status(&t.st)["install_error"]["message"].as_str().unwrap().contains("is installed"));
}

// --- update_auto ----------------------------------------------------------------------------------------------------------

/// The app started a moment ago; `later(min)` is "min minutes from now" for `auto_tick_at`.
fn later(min: f64) -> f64 {
    crate::hub::now() + min * 60.0
}

/// A stick button pressed `secs` before `now`.
fn input_before(st: &AppState, now: f64, secs: f64) {
    *st.last_input.lock().unwrap() = Some(now - secs);
}

#[tokio::test]
async fn the_automatic_install_starts_only_when_idle_for_ten_minutes() {
    let t = offered("auto").await;
    assert_eq!(t.st.cfg.lock().unwrap()["update_auto"], false, "off by default");
    assert!(!auto_tick_at(&t.st, later(30.0)), "off: nothing starts");
    t.st.update_config(json!({"update_auto": true}).as_object().unwrap().clone()).unwrap();
    // 11 minutes in, a button two minutes ago
    let now = later(11.0);
    input_before(&t.st, now, 120.0);
    assert!(!auto_tick_at(&t.st, now), "a button two minutes ago");
    assert_eq!(t.st.updates.run.phase(), Phase::Idle);
    input_before(&t.st, now, 660.0);
    t.st.jobs.add(crate::firmware::Job::new());
    assert!(!auto_tick_at(&t.st, now), "busy");
    let t = offered("auto2").await;
    t.st.update_config(json!({"update_auto": true}).as_object().unwrap().clone()).unwrap();
    input_before(&t.st, now, 660.0);
    assert!(auto_tick_at(&t.st, now));
    until("the restart", || *t.app.restarts.lock().unwrap() == 1).await;
}

#[tokio::test]
async fn nothing_installs_by_itself_in_the_first_ten_minutes_after_start_up() {
    let t = offered("autostart").await;
    t.st.update_config(json!({"update_auto": true}).as_object().unwrap().clone()).unwrap();
    // no button since start-up: idle is counted from the start, not "forever"
    assert!(!auto_tick(&t.st), "right after the start");
    assert!(!auto_tick_at(&t.st, later(2.0)), "80 s .. 2 min after the start");
    assert!(!auto_tick_at(&t.st, later(9.5)));
    assert_eq!(t.st.updates.run.phase(), Phase::Idle);
    assert!(t.inst.endpoints.lock().unwrap().is_empty());
    // ten quiet minutes since the start and no button: now
    assert!(auto_tick_at(&t.st, later(10.5)));
    until("the restart", || *t.app.restarts.lock().unwrap() == 1).await;
    // a button 12 minutes after the start, then 5 minutes quiet: not yet
    let t = offered("autostart2").await;
    t.st.update_config(json!({"update_auto": true}).as_object().unwrap().clone()).unwrap();
    let now = later(17.0);
    input_before(&t.st, now, 300.0);
    assert!(!auto_tick_at(&t.st, now));
}

#[tokio::test]
async fn a_failed_automatic_install_is_not_retried_in_a_loop() {
    let t = offered("autoonce").await;
    t.st.update_config(json!({"update_auto": true}).as_object().unwrap().clone()).unwrap();
    *t.inst.download_fails.lock().unwrap() = Some(InstallError::Network("offline".into()));
    let now = later(11.0);
    assert!(auto_tick_at(&t.st, now));
    until("error", || status(&t.st)["state"] == "error").await;
    *t.inst.download_fails.lock().unwrap() = None;
    assert!(!auto_tick_at(&t.st, now + 60.0), "tried a minute ago");
    assert_eq!(t.inst.endpoints.lock().unwrap().len(), 1);
    // a day later, it may again (the person's own button works any time)
    t.st.updates.run.cancel(); // dismiss the error
    assert!(auto_tick_at(&t.st, now + 25.0 * 3600.0));
}

#[tokio::test]
async fn the_automatic_install_needs_the_update_check_on() {
    let t = offered("autocheck").await;
    t.st.update_config(json!({"update_auto": true, "update_check": false}).as_object().unwrap().clone()).unwrap();
    assert!(!auto_tick_at(&t.st, later(30.0)));
}

#[tokio::test]
async fn a_cancel_during_the_last_moments_never_answers_ok_while_installing() {
    let t = offered("cancelrace").await;
    t.st.jobs.add(crate::firmware::Job::new()); // holds it in `waiting`
    start(&t.st, "test").unwrap();
    until("waiting", || status(&t.st)["waiting"].is_string()).await;
    // cancel is decided under the lock that enters Installing: once it says true, nothing is replaced
    assert!(cancel(&t.st));
    until("idle", || t.st.updates.run.phase() == Phase::Idle).await;
    assert_eq!(t.inst.installed.load(std::sync::atomic::Ordering::Relaxed), 0);
    // and the other way round: once installing, cancel says no
    let t = offered("cancelrace2").await;
    start(&t.st, "test").unwrap();
    until("the restart", || *t.app.restarts.lock().unwrap() == 1).await;
    assert!(!cancel(&t.st));
}

// --- after the restart ----------------------------------------------------------------------------------------------------

fn write_pending(home: &std::path::Path, to: &str) {
    std::fs::create_dir_all(home).unwrap();
    let at = crate::hub::now() - 5.0;
    std::fs::write(home.join("updates.json"), json!({"pending": {"from": "2.5.0", "to": to, "at": at}}).to_string())
        .unwrap();
}

fn toasts(home: &std::path::Path, title: &str) -> usize {
    events_log(home).lines().filter(|l| l.contains(title)).count()
}

#[tokio::test]
async fn updated_to_is_said_once_across_two_starts() {
    let home = home("after");
    write_pending(&home, env!("CARGO_PKG_VERSION"));
    let (st, _fake) = state_in(&home);
    let (_, mut rx) = st.hub.subscribe(0);
    connect(&st).await;
    after_restart(&st);
    let s = status(&st);
    assert_eq!(s["updated"]["to"], env!("CARGO_PKG_VERSION"));
    assert_eq!(s["updated"]["from"], "2.5.0");
    assert_eq!(s["state"], "idle");
    let mut kinds = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        kinds.push(ev["kind"].as_str().unwrap_or("").to_string());
    }
    assert!(kinds.contains(&"updated".to_string()), "{kinds:?}");
    until("the toast", || toasts(&home, "Updated to v") == 1).await;
    assert!(events_log(&home).contains(&format!("Updated to v{}", env!("CARGO_PKG_VERSION"))));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(home.join("updates.json")).unwrap()).unwrap();
    assert_eq!(saved["pending"], Value::Null, "cleared");
    // the next start: nothing
    let (again, _) = state_in(&home);
    connect(&again).await;
    after_restart(&again);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(status(&again)["updated"], Value::Null);
    assert_eq!(toasts(&home, "Updated to v"), 1);
}

#[tokio::test]
async fn the_toast_waits_for_the_stick() {
    let home = home("toastwait");
    let (st, _) = state_in(&home);
    let weak = Arc::downgrade(&st);
    let task = tokio::spawn(toast_when_connected(
        weak.clone(),
        "2.6.0".into(),
        Duration::from_millis(20),
        Duration::from_secs(5),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(toasts(&home, "Updated to v2.6.0"), 0, "no stick yet");
    connect(&st).await;
    let ev = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap().expect("sent");
    assert_eq!((ev["kind"].as_str(), ev["title"].as_str()), (Some("update"), Some("Updated to v2.6.0")));
    // no stick within the time: no toast, no hanging
    let home2 = self::home("toastnone");
    let (st2, _) = state_in(&home2);
    assert!(
        toast_when_connected(
            Arc::downgrade(&st2),
            "2.6.0".into(),
            Duration::from_millis(10),
            Duration::from_millis(50)
        )
        .await
        .is_none()
    );
}

#[tokio::test]
async fn an_update_that_did_not_take_is_reported_once() {
    let home = home("didnttake");
    write_pending(&home, NEWER);
    let (st, _) = state_in(&home);
    after_restart(&st);
    let s = status(&st);
    assert_eq!(s["state"], "error");
    let m = s["install_error"]["message"].as_str().unwrap();
    assert!(m.contains(&format!("didn't take — still on v{}", env!("CARGO_PKG_VERSION"))), "{m}");
    assert!(m.contains(NEWER));
    assert_eq!(s["updated"], Value::Null);
    assert_eq!(toasts(&home, "Updated to v"), 0);
    // once: the next start says nothing
    let (again, _) = state_in(&home);
    after_restart(&again);
    assert_eq!(status(&again)["state"], "idle");
}

// --- the API ------------------------------------------------------------------------------------------------------------

fn req(method: &str, path: &str, token: Option<&str>) -> Request {
    let mut b = Request::builder().method(method).uri(path).header("host", "localhost:8670");
    if let Some(t) = token {
        b = b.header("x-stick-token", t);
    }
    b.body(Body::empty()).unwrap()
}

async fn call(app: &axum::Router, r: Request) -> (axum::http::StatusCode, Value) {
    let resp = app.clone().oneshot(r).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_api_starts_refuses_and_cancels() {
    let t = offered("api").await;
    let app = crate::api::router(t.st.clone());
    let tok = Some(t.st.token.as_str());
    // GET keeps every old field and adds the new ones
    let (s, v) = call(&app, req("GET", "/api/update", None)).await;
    assert_eq!(s, axum::http::StatusCode::OK);
    for k in [
        "enabled",
        "channel",
        "checking",
        "checked_at",
        "error",
        "note",
        "current",
        "latest",
        "agent_update",
        "firmware_update",
        "available",
        "title",
        "install",
        "firmware_updaters",
    ] {
        assert!(v.get(k).is_some(), "old field {k} is gone");
    }
    for k in ["kind", "path", "can_update", "why", "head", "branch", "download", "instructions"] {
        assert!(v["install"].get(k).is_some(), "old install.{k} is gone");
    }
    for k in ["state", "progress", "waiting", "install_error", "updated", "auto"] {
        assert!(v.get(k).is_some(), "new field {k} is missing");
    }
    assert_eq!(
        (v["state"].as_str(), v["auto"].as_bool(), v["latest"]["updater_asset"].as_str()),
        (Some("available"), Some(false), Some("latest.json"))
    );
    assert_eq!(v["title"], format!("Update available: v{NEWER}"));
    assert_eq!(v["install"]["download"], format!("https://example.invalid/v{NEWER}"));
    // the status the menu and the pill read carries the state too
    let (_, st) = call(&app, req("GET", "/api/status", None)).await;
    assert_eq!(st["update"]["state"], "available");
    // POSTs need the token
    for path in ["/api/update/install", "/api/update/cancel", "/api/update/agent"] {
        let (s, _) = call(&app, req("POST", path, None)).await;
        assert_eq!(s, axum::http::StatusCode::FORBIDDEN, "{path}");
    }
    // nothing to cancel yet
    let (s, v) = call(&app, req("POST", "/api/update/cancel", tok)).await;
    assert_eq!(s, axum::http::StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("no download to cancel"));
    // start (slowly), a second one is refused with the current state
    *t.inst.delay.lock().unwrap() = Duration::from_millis(400);
    let (s, v) = call(&app, req("POST", "/api/update/install", tok)).await;
    assert_eq!(s, axum::http::StatusCode::ACCEPTED);
    assert_eq!(v["state"], "downloading");
    let (s, v) = call(&app, req("POST", "/api/update/install", tok)).await;
    assert_eq!(s, axum::http::StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("already"));
    assert_eq!(v["state"], "downloading");
    let (s, _) = call(&app, req("POST", "/api/update/agent", tok)).await;
    assert_eq!(s, axum::http::StatusCode::CONFLICT, "the old path is the same as /install");
    // cancel the download
    let (s, v) = call(&app, req("POST", "/api/update/cancel", tok)).await;
    assert_eq!(s, axum::http::StatusCode::OK);
    assert!(v.get("state").is_some());
    until("idle", || t.st.updates.run.phase() == Phase::Idle).await;
    // the old path starts an install too
    *t.inst.delay.lock().unwrap() = Duration::ZERO;
    let (s, v) = call(&app, req("POST", "/api/update/agent", tok)).await;
    assert_eq!(s, axum::http::StatusCode::ACCEPTED);
    assert_eq!(v["state"], "downloading");
    until("restart", || *t.app.restarts.lock().unwrap() == 1).await;
}

#[tokio::test]
async fn the_api_in_the_headless_app_still_answers_409_with_where_to_download() {
    let t = offered("apiheadless").await;
    t.st.updates.run.set_installer(Arc::new(NoInstaller));
    let app = crate::api::router(t.st.clone());
    let (s, v) = call(&app, req("POST", "/api/update/agent", Some(t.st.token.as_str()))).await;
    assert_eq!(s, axum::http::StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("Only the menu bar app updates itself"));
    assert_eq!(v["install"]["download"], format!("https://example.invalid/v{NEWER}"));
    assert_eq!(v["install"]["can_update"], false);
}

#[tokio::test]
async fn update_auto_is_a_setting_that_must_be_true_or_false() {
    let t = setup("config", None, true).await;
    let app = crate::api::router(t.st.clone());
    let tok = Some(t.st.token.as_str());
    let put = |body: Value| {
        Request::builder()
            .method("PUT")
            .uri("/api/config")
            .header("host", "localhost:8670")
            .header("x-stick-token", t.st.token.as_str())
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let _ = tok;
    let (s, v) = call(&app, put(json!({"update_auto": true}))).await;
    assert_eq!((s, v["update_auto"].as_bool()), (axum::http::StatusCode::OK, Some(true)));
    assert_eq!(t.st.updates.status(&t.st.cfg.lock().unwrap().clone(), false)["auto"], true);
    let (s, v) = call(&app, put(json!({"update_auto": "yes"}))).await;
    assert_eq!(s, axum::http::StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("update_auto"));
    // the file a fresh start reads
    let saved = std::fs::read_to_string(t.st.paths.config()).unwrap();
    assert!(saved.contains("\"update_auto\": true"), "{saved}");
}
