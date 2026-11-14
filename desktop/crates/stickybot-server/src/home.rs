//! The stick's home screen and faces (a port of `home.py` and core.py's home / world parts): the
//! bottom card, the weather (`wx`), the faces' world (`world`), and the loops that keep them fresh
//! (weather + feed every minute, the GitHub watcher), the events, the pet.
//!
//! The card is pinned when something needs you (approvals waiting: urgent, orange on the stick),
//! then a line a Claude session put there (`put`, the stick tools later), then a meeting that is
//! about to start (Calendar connection); otherwise it rotates every 16 s between a meeting today,
//! your last answer (3 min), Claude Code sessions open, the feed, and a greeting. Sent as
//! `{"k","a","b","u"}` (+ `"i","n"` for the source dots).

use crate::AppState;
use crate::feed::Feed;
use crate::net::Http;
use crate::pet::Pet;
use crate::connections::agenda::MeetingCard;
use crate::talk::stick_text;
use crate::weather::Weather;
use crate::{events, github};
use chrono::Timelike;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const FRESH_REPLY_S: f64 = 180.0;
pub const ROTATE_S: f64 = 16.0;

/// Counts the Claude Code processes on this Mac that aren't ours (tests swap it).
pub type SessionCounter = Arc<dyn Fn() -> usize + Send + Sync>;

/// `ps`: processes named `claude`, not counting our own Claude Code (a child of this app).
pub fn claude_sessions() -> usize {
    let Ok(out) = std::process::Command::new("ps").args(["-axo", "pid=,ppid=,comm="]).output() else { return 0 };
    let me = std::process::id().to_string();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            let mut it = l.split_whitespace();
            let (_, ppid) = (it.next(), it.next());
            let comm = it.collect::<Vec<_>>().join(" ");
            comm.rsplit('/').next() == Some("claude") && ppid != Some(me.as_str())
        })
        .count()
}

pub fn greeting(hour: u32) -> Value {
    let part = match hour {
        5..=11 => "morning",
        12..=16 => "afternoon",
        _ => "evening",
    };
    json!({"k": "hello", "a": format!("Good {part}"), "b": "Hold B to ask me anything."})
}

#[derive(Default)]
struct Rotation {
    slot: usize,
    last_turn: f64,
    current: Option<Value>,
    /// "progress" | "pin" -> (card, until, at)
    slots: HashMap<String, (Value, f64, f64)>,
}

/// Everything behind the home screen and the faces.
pub struct Home {
    pub weather: Mutex<Weather>,
    pub feed: Mutex<Feed>,
    pub pet: Mutex<Pet>,
    /// the GitHub watcher's state (`github_state.json`), `None` before the first look
    pub gh_state: Mutex<Option<Value>>,
    /// `{"ok", "error", "tracked"}` for `/api/status`
    gh_status: Mutex<(f64, String)>,
    pub notifier: events::Notifier,
    rot: Mutex<Rotation>,
    pub(crate) world_sent: Mutex<Option<Value>>,
    http: Mutex<Arc<dyn Http>>,
    gh: Mutex<Arc<dyn github::Gh>>,
    sessions: Mutex<SessionCounter>,
}

impl Home {
    pub fn new(home: &std::path::Path, http: Arc<dyn Http>, gh: Arc<dyn github::Gh>, sessions: SessionCounter) -> Self {
        let gh_state = std::fs::read_to_string(home.join("github_state.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object);
        // a new pet starts with the PRs the watcher already saw merged
        let merged = gh_state
            .as_ref()
            .and_then(|g| g["prs"].as_object())
            .map(|p| p.values().filter(|p| p["merged"] == true).count() as i64)
            .unwrap_or(0);
        Self {
            weather: Mutex::new(Weather::default()),
            feed: Mutex::new(Feed::default()),
            pet: Mutex::new(Pet::load(&home.join("pet.json"), merged)),
            gh_state: Mutex::new(gh_state),
            gh_status: Mutex::new((0.0, String::new())),
            notifier: events::Notifier::default(),
            rot: Mutex::new(Rotation::default()),
            world_sent: Mutex::new(None),
            http: Mutex::new(http),
            gh: Mutex::new(gh),
            sessions: Mutex::new(sessions),
        }
    }

    /// Shows a line on the card for `secs` (a Claude session's progress or pinned line).
    pub fn put(&self, slot: &str, a: &str, b: &str, secs: f64) {
        let now = crate::hub::now();
        self.rot.lock().unwrap().slots.insert(slot.into(), (json!({"k": "claude", "a": a, "b": b}), now + secs, now));
    }

    pub fn clear(&self, slot: &str) -> bool {
        self.rot.lock().unwrap().slots.remove(slot).is_some()
    }

    /// Reads the pet and the GitHub state again (after an import).
    pub fn reload(&self, home: &std::path::Path) {
        let gh = std::fs::read_to_string(home.join("github_state.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object);
        *self.gh_state.lock().unwrap() = gh;
        *self.pet.lock().unwrap() = Pet::load(&home.join("pet.json"), 0);
    }

    /// Swaps the network, `gh` and the process count (tests, the fake stick).
    pub fn set_sources(&self, http: Arc<dyn Http>, gh: Arc<dyn github::Gh>, sessions: SessionCounter) {
        *self.http.lock().unwrap() = http;
        *self.gh.lock().unwrap() = gh;
        *self.sessions.lock().unwrap() = sessions;
    }

    pub fn github_status(&self) -> Value {
        let (ok, err) = self.gh_status.lock().unwrap().clone();
        let tracked = self.gh_state.lock().unwrap().as_ref().and_then(|g| g["prs"].as_object().map(Map::len)).unwrap_or(0);
        json!({"ok": ok, "error": err, "tracked": tracked})
    }
}

/// What the card needs from the rest of the app.
pub struct CardInput {
    /// tools of the approvals waiting, in order
    pub prompts: Vec<String>,
    /// questions from Claude sessions: `(from, question)`
    pub asks: Vec<(String, String)>,
    /// `{"t", "text"}`
    pub last_reply: Option<Value>,
    pub ticker: bool,
    pub now: f64,
    pub hour: u32,
    /// the Calendar connection's next / current meeting, if it has one to show
    pub meeting: Option<MeetingCard>,
}

/// The card (home.py `Home.card`), before it's made stick-safe.
pub fn card(home: &Home, inp: &CardInput) -> Value {
    if let Some(tool) = inp.prompts.first() {
        let more = if inp.prompts.len() > 1 { format!(" +{}", inp.prompts.len() - 1) } else { String::new() };
        return json!({"k": "approval", "a": format!("Claude wants {tool}{more}"), "b": "A yes  ·  B no", "u": true});
    }
    if let Some((from, q)) = inp.asks.first() {
        return json!({"k": "approval", "a": format!("{from} asks"), "b": q, "u": true});
    }
    let mut rot = home.rot.lock().unwrap();
    rot.slots.retain(|_, v| v.1 > inp.now);
    if let Some((c, _, _)) = rot.slots.values().max_by(|a, b| a.2.total_cmp(&b.2)) {
        return c.clone();
    }
    // a meeting about to start (or just started) comes before the rotation, but not before something
    // that waits for a button or a line a Claude session put there; not urgent: orange is for buttons
    if let Some(m) = inp.meeting.as_ref().filter(|m| m.pinned) {
        return m.card.clone();
    }
    // rotation candidates, rebuilt each time
    let mut cands: Vec<Option<Value>> = Vec::new(); // None = the feed
    if let Some(m) = &inp.meeting {
        cands.push(Some(m.card.clone()));
    }
    if let Some(r) = &inp.last_reply
        && inp.now - r["t"].as_f64().unwrap_or(0.0) < FRESH_REPLY_S
    {
        cands.push(Some(json!({"k": "reply", "a": "Last answer", "b": r["text"]})));
    }
    let count = home.sessions.lock().unwrap().clone();
    let n = count();
    if n > 0 {
        cands.push(Some(json!({"k": "claude", "a": "Claude Code", "b": format!("{n} session{} open", if n != 1 { "s" } else { "" })})));
    }
    if inp.ticker && !home.feed.lock().unwrap().items.is_empty() {
        cands.push(None);
    }
    if cands.is_empty() {
        return greeting(inp.hour);
    }
    if rot.current.is_none() || inp.now - rot.last_turn >= ROTATE_S {
        rot.slot = (rot.slot + 1) % cands.len();
        rot.current = match &cands[rot.slot] {
            Some(c) => Some(c.clone()),
            None => home.feed.lock().unwrap().next_item(),
        };
        rot.last_turn = inp.now;
    }
    let mut c = rot.current.clone().unwrap_or_else(|| greeting(inp.hour));
    c["i"] = json!(rot.slot);
    c["n"] = json!(cands.len());
    c
}

fn card_input(st: &AppState) -> CardInput {
    let status = st.approvals.list();
    let prompts =
        status.as_array().into_iter().flatten().map(|p| p["tool"].as_str().unwrap_or("").to_string()).collect();
    // before the settings lock: the Calendar connection reads the settings too
    let meeting = crate::connections::calendar::meeting_card(st);
    let cfg = st.cfg.lock().unwrap();
    CardInput {
        prompts,
        asks: st
            .approvals
            .asks()
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| (a["from"].as_str().unwrap_or("").to_string(), a["question"].as_str().unwrap_or("").to_string()))
            .collect(),
        last_reply: st.last_reply.lock().unwrap().clone(),
        ticker: cfg.get("ticker").and_then(Value::as_bool).unwrap_or(true),
        now: crate::hub::now(),
        hour: chrono::Local::now().hour(),
        meeting,
    }
}

/// The card for the stick (core.py `home_card`): stick-safe text, cut to 60 / 140.
pub fn home_card(st: &AppState) -> Value {
    let mut c = card(&st.home, &card_input(st));
    let cut = |v: &Value, n: usize| json!(stick_text(v.as_str().unwrap_or("")).chars().take(n).collect::<String>());
    c["a"] = cut(&c["a"], 60);
    c["b"] = cut(&c["b"], 140);
    c
}

/// The hooks' session list. The Claude Code hooks the Python installer registered write it into
/// the Python app's folder (`claude_hooks.py`); after the switch-over (switch.rs) this app reads it
/// there (read-only), else its own folder's.
fn sessions_file(st: &AppState) -> std::path::PathBuf {
    match &st.import_from {
        Some(py) if crate::switch::switched(&st.paths.home) => py.join("claude_sessions.json"),
        _ => st.paths.home.join("claude_sessions.json"),
    }
}

/// The faces' data (pet.py `world`): PR counts, CI, Claude sessions, what waits, the pet.
pub fn world(st: &AppState) -> Value {
    let gh = st.home.gh_state.lock().unwrap().clone();
    let sessions: Vec<Value> = crate::pet::running_sessions(&sessions_file(st), crate::hub::now())
        .into_iter()
        .map(|mut s| {
            s["p"] = json!(stick_text(s["p"].as_str().unwrap_or("")));
            s
        })
        .collect();
    let mut pet = st.home.pet.lock().unwrap();
    let merged = pet.s["merged"].as_i64().unwrap_or(0);
    json!({"mergedToday": pet.merged_today(), "merged": merged, "ci": crate::pet::ci_state(gh.as_ref()),
           "asks": st.approvals.list().as_array().map_or(0, Vec::len),
           "reviews": gh.as_ref().and_then(|g| g["asks"].as_object().map(Map::len)).unwrap_or(0),
           "claude": sessions, "pet": pet.pet_json()})
}

/// `home`, `world` and `wx` for the hello (core.py `_hello_extra`; `wmic` / `wready` come from wmic.rs).
pub fn hello_extra(st: &AppState) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("home".into(), home_card(st));
    m.insert("world".into(), world(st));
    if let Some(mut wx) = st.home.weather.lock().unwrap().now.clone() {
        wx["at"] = json!(stick_text(wx["at"].as_str().unwrap_or("")));
        m.insert("wx".into(), wx);
    }
    m
}

/// The world now (a PR merged, Claude finished…), not with the next hello.
pub async fn push_world(st: &AppState, only_changed: bool) -> bool {
    let w = world(st);
    {
        let mut sent = st.home.world_sent.lock().unwrap();
        if only_changed && sent.as_ref() == Some(&w) {
            return false;
        }
        *sent = Some(w.clone());
    }
    st.link.send(&json!({"src": "agent", "world": w})).await
}

/// A Claude session's line on the card (`slot`: "progress" | "pin") for `secs`; pair it with
/// [`push_home`] so the stick shows it at once (the stick tools).
pub fn put(st: &AppState, slot: &str, a: &str, b: &str, secs: f64) {
    st.home.put(slot, a, b, secs);
}

/// Takes a session's line off the card; false if there was none.
pub fn clear(st: &AppState, slot: &str) -> bool {
    st.home.clear(slot)
}

/// The card now, not with the next hello (up to 8 s later).
pub async fn push_home(st: &AppState) -> bool {
    st.link.send(&json!({"src": "agent", "home": home_card(st)})).await
}

/// From the Claude Code Stop hook: "Claude finished in <project>" for long tasks; every finished
/// task feeds the pet (core.py `claude_finished`).
pub async fn claude_finished(st: &AppState, elapsed_s: i64, cwd: &str, prompt: &str) -> Option<Value> {
    st.home.pet.lock().unwrap().claude_done();
    push_world(st, false).await;
    let min = st.cfg.lock().unwrap().get("claude_done_min_s").and_then(Value::as_i64).unwrap_or(0);
    if elapsed_s < min {
        return None;
    }
    let base = cwd.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let project = if base.is_empty() { "a session" } else { base };
    let took = if elapsed_s >= 60 { format!("{} min", elapsed_s / 60) } else { format!("{elapsed_s} s") };
    let p: String = prompt.chars().take(60).collect();
    let sub = format!("took {took}{}", if p.is_empty() { String::new() } else { format!(" - {p}") });
    events::notify(st, "claude_done", &format!("Claude finished in {project}"), &sub, None, None, None).await
}

/// One GitHub poll (github.py `poll`): the query, the events, the pet, the saved state.
pub async fn github_poll(st: &AppState) -> Result<(), String> {
    let cfg = github::settings(&st.cfg.lock().unwrap());
    let gh = st.home.gh.lock().unwrap().clone();
    if !gh.available() {
        st.home.gh_status.lock().unwrap().1 = "gh CLI not found (brew install gh, then gh auth login)".into();
        return Ok(());
    }
    let primary = cfg.get("primary_repo").and_then(Value::as_str).unwrap_or("").to_string();
    let since = (chrono::Local::now() - chrono::Duration::days(14)).format("%Y-%m-%d").to_string();
    let data = gh.query(&primary, &since).await?;
    let old = st.home.gh_state.lock().unwrap().clone();
    let (new, changes) = github::merge(old.as_ref(), github::snapshot(&data), &primary, crate::hub::now());
    *st.home.gh_state.lock().unwrap() = Some(new.clone());
    for c in changes.iter().flatten() {
        match c {
            github::Change::PetMerged => st.home.pet.lock().unwrap().merged(),
            github::Change::PetGreen => st.home.pet.lock().unwrap().green(),
            github::Change::PetRelease => st.home.pet.lock().unwrap().release(),
            github::Change::Event(k, t, s) => {
                events::notify(st, k, t, s, None, None, None).await;
            }
        }
    }
    if let Err(e) = std::fs::write(st.paths.home.join("github_state.json"), new.to_string()) {
        log::warn!("github: can't save the state ({e})");
    }
    if old.is_some() {
        push_world(st, true).await; // CI, reviews and merges for the faces
    }
    *st.home.gh_status.lock().unwrap() = (crate::hub::now(), String::new());
    Ok(())
}

/// Weather + feed now (on a blocking thread: they use the network).
pub async fn refresh_weather(st: &Arc<AppState>, force: bool) {
    let st2 = st.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let cfg = st2.cfg.lock().unwrap().clone();
        let http = st2.home.http.lock().unwrap().clone();
        let now = crate::hub::now();
        // the network outside the locks: work on copies
        let mut w = std::mem::take(&mut *st2.home.weather.lock().unwrap());
        w.refresh(&*http, &cfg, force, now);
        *st2.home.weather.lock().unwrap() = w;
        let mut f = std::mem::take(&mut *st2.home.feed.lock().unwrap());
        f.refresh(&*http, &cfg, now, crate::feed::today());
        *st2.home.feed.lock().unwrap() = f;
    })
    .await;
}

/// Settings changed: weather / feed again (core.py `update_config`).
pub fn config_changed(st: &Arc<AppState>, changes: &Map<String, Value>) {
    if changes.contains_key("idle_content") {
        st.home.feed.lock().unwrap().due_now();
    }
    if changes.contains_key("weather") || changes.contains_key("weather_location") || changes.contains_key("idle_content") {
        let st = st.clone();
        let force = changes.contains_key("weather") || changes.contains_key("weather_location");
        tokio::spawn(async move { refresh_weather(&st, force).await });
    }
}

/// `{"evt":"game","id","score","best","new_best"}`: a game finished on the stick (firmware 2.5.0,
/// core.py `_on_game`): a little XP (capped a day, pet.rs) and a `game` event for the dashboard's
/// Games card; new XP goes to the stick's faces at once. Unknown games are ignored.
pub async fn on_game(st: &AppState, m: &Value) {
    let gid = m["id"].as_str().unwrap_or("");
    let new_best = m["new_best"] == json!(true);
    let xp = st.home.pet.lock().unwrap().game(gid, &m["score"], &m["best"], new_best);
    if !crate::pet::is_game(gid) {
        log::info!("game event for an unknown game {:?}: ignored", m["id"]);
        return;
    }
    log::info!("game {gid}: {}{}, +{xp} XP", m["score"], if new_best { " (new best)" } else { "" });
    st.hub.emit("game", json!({"id": gid, "score": m["score"], "best": m["best"], "new_best": new_best, "xp": xp}));
    if xp > 0 {
        push_world(st, true).await;
    }
}

/// The loops (core.py `run`): weather + feed every minute, the GitHub watcher, and the stick's
/// `event_shown` answers.
pub fn spawn(st: Arc<AppState>) {
    let s = st.clone();
    tokio::spawn(async move {
        loop {
            refresh_weather(&s, false).await;
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    let s = st.clone();
    tokio::spawn(async move {
        let mut backoff = 0u64;
        loop {
            let cfg = github::settings(&s.cfg.lock().unwrap());
            let mut wait = cfg.get("interval_s").and_then(Value::as_u64).unwrap_or(120).max(30);
            if cfg.get("on").and_then(Value::as_bool).unwrap_or(true) {
                match github_poll(&s).await {
                    Ok(()) => backoff = 0,
                    Err(e) => {
                        s.home.gh_status.lock().unwrap().1 = e.clone();
                        backoff = github::MAX_BACKOFF_S.min(wait.max(backoff * 2));
                        log::warn!("github: {e} (next try in {backoff} s)");
                        wait = backoff;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    });
    let mut msgs = st.link.messages();
    tokio::spawn(async move {
        loop {
            match msgs.recv().await {
                Ok(m) if m["src"] == "agent" && m["evt"] == "event_shown" => {
                    st.home.notifier.acked(m["id"].as_str().unwrap_or(""));
                }
                Ok(m) if m["src"] == "agent" && m["evt"] == "game" => on_game(&st, &m).await,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeHttp;
    use futures_util::future::BoxFuture;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct NoGh;
    impl github::Gh for NoGh {
        fn available(&self) -> bool {
            false
        }
        fn query(&self, _: &str, _: &str) -> BoxFuture<'static, Result<Value, String>> {
            Box::pin(async { Err("no gh".into()) })
        }
    }

    fn home(name: &str, sessions: usize) -> (Home, Arc<AtomicUsize>) {
        let d = std::env::temp_dir().join(format!("stickybot-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let n = Arc::new(AtomicUsize::new(sessions));
        let n2 = n.clone();
        (Home::new(&d, Arc::new(FakeHttp::default()), Arc::new(NoGh), Arc::new(move || n2.load(Ordering::Relaxed))), n)
    }

    fn inp(now: f64) -> CardInput {
        CardInput { prompts: vec![], asks: vec![], last_reply: None, ticker: true, now, hour: 9, meeting: None }
    }

    #[test]
    fn greetings_by_hour() {
        assert_eq!(greeting(9)["a"], "Good morning");
        assert_eq!(greeting(13)["a"], "Good afternoon");
        assert_eq!(greeting(22)["a"], "Good evening");
        assert_eq!(greeting(3)["a"], "Good evening");
        assert_eq!(greeting(3)["b"], "Hold B to ask me anything.");
    }

    #[test]
    fn approvals_and_asks_win_then_pins_then_rotation() {
        let (h, _) = home("pins", 0);
        let mut i = inp(1000.0);
        assert_eq!(card(&h, &i), greeting(9));
        i.prompts = vec!["Bash".into(), "Edit".into()];
        assert_eq!(card(&h, &i), json!({"k": "approval", "a": "Claude wants Bash +1", "b": "A yes  ·  B no", "u": true}));
        i.prompts.clear();
        i.asks = vec![("web".into(), "Ship it?".into())];
        assert_eq!(card(&h, &i), json!({"k": "approval", "a": "web asks", "b": "Ship it?", "u": true}));
        i.asks.clear();
        h.put("progress", "Tests", "3/10", 600.0);
        assert_eq!(card(&h, &i), json!({"k": "claude", "a": "Tests", "b": "3/10"}));
        assert!(h.clear("progress"));
    }

    fn meeting(pinned: bool) -> Option<MeetingCard> {
        Some(MeetingCard { card: json!({"k": "meeting", "a": "Standup", "b": "in 12 min · 10:30"}), pinned })
    }

    #[test]
    fn a_meeting_about_to_start_comes_before_the_rotation_but_not_before_what_waits_for_you() {
        let (h, _) = home("meeting-pin", 2);
        h.feed.lock().unwrap().items = vec![json!({"k": "fact", "a": "Fun fact", "b": "x"})];
        let mut i = inp(1000.0);
        i.last_reply = Some(json!({"t": 990.0, "text": "Sunny"}));
        i.meeting = meeting(true);
        let meeting_card = json!({"k": "meeting", "a": "Standup", "b": "in 12 min · 10:30"});
        // every turn of the rotation is the meeting: it is pinned, not urgent (no orange)
        for _ in 0..4 {
            assert_eq!(card(&h, &i), meeting_card);
            i.now += ROTATE_S;
        }
        // a session's line and then an approval stay on top of it
        h.put("pin", "PR 12", "waiting for review", 600.0);
        assert_eq!(card(&h, &i), json!({"k": "claude", "a": "PR 12", "b": "waiting for review"}));
        i.prompts = vec!["Bash".into()];
        assert_eq!(card(&h, &i)["k"], "approval");
        i.prompts.clear();
        i.asks = vec![("web".into(), "Ship it?".into())];
        assert_eq!(card(&h, &i)["a"], "web asks");
        // without the meeting nothing changes
        i.asks.clear();
        assert!(h.clear("pin"));
        i.meeting = None;
        assert_ne!(card(&h, &i)["k"], "meeting");
    }

    #[test]
    fn a_meeting_later_today_is_one_more_turn_of_the_rotation() {
        let (h, _) = home("meeting-rot", 0);
        let mut i = inp(1000.0);
        i.meeting = meeting(false);
        i.last_reply = Some(json!({"t": 990.0, "text": "Sunny"}));
        let mut seen = Vec::new();
        for _ in 0..4 {
            let c = card(&h, &i);
            assert_eq!(c["n"], 2);
            seen.push(c["k"].as_str().unwrap().to_string());
            i.now += ROTATE_S;
        }
        seen.sort();
        assert_eq!(seen, ["meeting", "meeting", "reply", "reply"]);
    }

    #[test]
    fn rotation_every_16_s() {
        let (h, n) = home("rot", 2);
        h.feed.lock().unwrap().items = vec![json!({"k": "fact", "a": "Fun fact", "b": "x"})];
        let mut i = inp(1000.0);
        i.last_reply = Some(json!({"t": 990.0, "text": "Sunny"}));
        let c1 = card(&h, &i);
        assert_eq!(c1["n"], 3);
        assert_eq!(c1["i"], 1);
        assert_eq!(c1["b"], "2 sessions open");
        i.now += 5.0;
        assert_eq!(card(&h, &i)["b"], "2 sessions open"); // not yet
        i.now += ROTATE_S;
        assert_eq!(card(&h, &i)["k"], "fact");
        i.now += ROTATE_S;
        assert_eq!(card(&h, &i)["b"], "Sunny");
        // the reply gets old, no sessions, no ticker: the greeting
        i.now += FRESH_REPLY_S;
        i.ticker = false;
        n.store(0, Ordering::Relaxed);
        assert_eq!(card(&h, &i), greeting(9));
        n.store(1, Ordering::Relaxed);
        i.now += ROTATE_S;
        assert_eq!(card(&h, &i)["b"], "1 session open");
    }
}
