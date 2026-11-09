//! Hold B to talk (core.py `_on_message` / `_handle_talk` / `handle_text`): the stick streams its
//! mic as ADPCM frames between `{"evt":"talk","state":"start"}` and `…"stop"`; the agent decodes
//! them, transcribes, and answers on the stick's screen with
//! `{"src":"agent","state":"thinking"|"working"|"done"|"error"|"cancelled","heard","text"}`.
//!
//! What it answers with: a "new chat" phrase, a quick command (commands.rs), or the brain
//! ([`Brain`]; until Claude is ported, [`NoBrain`] says so on the stick).

use crate::AppState;
use crate::commands;
use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use stick_link::FromStick;
use stick_link::adpcm::{RATE, Recording};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Characters the stick's fonts have beyond ASCII (m5-stick-s3/tools/make_fonts.py).
const STICK_EXTRA: &str = "°’‘“”–—…•·×";

/// Text the stick can draw: ASCII + its extra symbols; the rest transliterated or dropped.
pub fn stick_text(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii() || STICK_EXTRA.contains(ch) {
            out.push(ch);
        } else {
            out.extend(std::iter::once(ch).nfkd().filter(char::is_ascii));
        }
    }
    out
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

static NEW_CHAT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?is)^\s*(?:new (?:chat|conversation)|(?:let'?s )?start (?:over|again|a new (?:chat|conversation))|forget (?:that|everything)|reset (?:the )?(?:chat|conversation))(?: please)?\s*(?:[.,!?;:]+\s*(?P<rest>.*))?$",
    )
    .expect("valid")
});

/// `(true, rest)` if the request starts with a "new chat" phrase (brain.py `split_new_chat`).
pub fn split_new_chat(text: &str) -> (bool, String) {
    match NEW_CHAT.captures(text) {
        Some(m) => (true, m.name("rest").map(|r| r.as_str().trim().to_string()).unwrap_or_default()),
        None => (false, text.to_string()),
    }
}

static CODE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").expect("valid"));
static MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*_`#>]+").expect("valid"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid"));

/// A reply for the stick's screen: no markdown, at most 280 characters (brain.py `plain`).
pub fn plain(text: &str) -> String {
    let t = CODE_BLOCK.replace_all(text, "");
    let t = t.split("```").next().unwrap_or(""); // a code block still being written
    let t = MARKS.replace_all(t, "");
    let t = SPACES.replace_all(&t, " ").trim().to_string();
    if t.chars().count() <= 280 {
        return t;
    }
    let head: String = t.chars().take(277).collect();
    let head = match head.rfind(' ') {
        Some(i) => head[..i].to_string(),
        None => head,
    };
    format!("{head}…")
}

// --- the brain ---------------------------------------------------------------------------------------

/// What a brain reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum BrainEvent {
    /// a tool line ("Searching: weather"): every one is shown
    Progress(String),
    /// the answer so far ("" withdraws a preamble)
    Text(String),
}

/// Claude Code / OpenRouter behind one face (brains.py).
pub trait Brain: Send + Sync {
    fn busy(&self) -> bool;
    /// Answers `text`; `(ok, reply)`.
    fn ask(&self, text: String, events: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)>;
    fn cancel(&self);
    /// The last request was cancelled (say nothing more).
    fn cancelled(&self) -> bool;
    /// The next request starts a new conversation.
    fn forget(&self);
    /// Gets ready before the first request (the stick connected).
    fn prewarm(&self) {}
    fn status(&self) -> Value {
        Value::Null
    }
    /// The settings changed (a different brain may be picked now).
    fn switched(&self) {}
}

/// No brain yet: requests that aren't quick commands get an honest error on the stick.
pub struct NoBrain;

pub const NO_BRAIN: &str = "Claude isn't hooked up in the Rust app yet.";

impl Brain for NoBrain {
    fn busy(&self) -> bool {
        false
    }
    fn ask(&self, _text: String, _events: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)> {
        Box::pin(async { (false, NO_BRAIN.to_string()) })
    }
    fn cancel(&self) {}
    fn cancelled(&self) -> bool {
        false
    }
    fn forget(&self) {}
}

// --- the reply streamed to the stick ----------------------------------------------------------------

enum StreamCmd {
    Push(String),
    Now(String, oneshot::Sender<()>),
    Close(oneshot::Sender<()>),
}

/// The reply on the stick while the brain writes it (core.py `StickStream`): "working" updates,
/// the latest text only, at most one per `interval` and never two at once.
struct StickStream {
    tx: mpsc::UnboundedSender<StreamCmd>,
    sent: Arc<std::sync::atomic::AtomicUsize>,
}

impl StickStream {
    fn new(st: Arc<AppState>, heard: String, interval: Duration) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = sent.clone();
        tokio::spawn(async move {
            let mut pending: Option<String> = None;
            let mut shown: Option<String> = None;
            let mut last: Option<Instant> = None;
            let show = |text: String| {
                let st = st.clone();
                let heard = heard.clone();
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move { tell_stick(&st, "working", &heard, &text).await }
            };
            loop {
                let due = match (&pending, last) {
                    (Some(_), Some(l)) => Some(tokio::time::Instant::from_std(l + interval)),
                    (Some(_), None) => Some(tokio::time::Instant::now()),
                    _ => None,
                };
                let cmd = match due {
                    Some(d) => tokio::select! {
                        c = rx.recv() => c,
                        _ = tokio::time::sleep_until(d) => {
                            let text = pending.take().expect("pending");
                            last = Some(Instant::now());
                            shown = Some(text.clone());
                            show(text).await;
                            continue;
                        }
                    },
                    None => rx.recv().await,
                };
                match cmd {
                    Some(StreamCmd::Push(text)) => {
                        if pending.as_ref().or(shown.as_ref()) != Some(&text) {
                            pending = Some(text);
                        }
                    }
                    Some(StreamCmd::Now(text, done)) => {
                        pending = None;
                        last = Some(Instant::now());
                        shown = Some(text.clone());
                        show(text).await;
                        let _ = done.send(());
                    }
                    Some(StreamCmd::Close(done)) => {
                        let _ = done.send(());
                        return;
                    }
                    None => return,
                }
            }
        });
        Self { tx, sent }
    }

    fn push(&self, text: String) {
        let _ = self.tx.send(StreamCmd::Push(text));
    }

    async fn send_now(&self, text: String) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(StreamCmd::Now(text, tx)).is_ok() {
            let _ = rx.await;
        }
    }

    /// No more updates; returns once one being sent has gone (so "done" comes after it).
    async fn close(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(StreamCmd::Close(tx)).is_ok() {
            let _ = rx.await;
        }
        self.sent.load(std::sync::atomic::Ordering::Relaxed)
    }
}

// --- the flow ----------------------------------------------------------------------------------------

/// Shows `state` on the stick (core.py `tell_stick`).
pub async fn tell_stick(st: &AppState, state: &str, heard: &str, text: &str) -> bool {
    st.link
        .send(&json!({"src": "agent", "state": state, "heard": cut(&stick_text(heard), 200),
                      "text": cut(&stick_text(text), 300)}))
        .await
}

/// Listens to the link for talks and cancels. Runs for the life of the app.
pub fn spawn(st: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    let mut rx = st.link.incoming();
    tokio::spawn(async move {
        let mut rec: Option<Recording> = None;
        loop {
            match rx.recv().await {
                Ok(FromStick::Audio(frame)) => {
                    // a Wispr stream's frames are the wireless mic's (wmic.rs), not a talk's
                    if let Some(r) = rec.as_mut()
                        && !st.wmic.streaming()
                    {
                        r.add(&frame);
                    }
                }
                Ok(FromStick::Message(msg)) => on_message(&st, &msg, &mut rec),
                Err(broadcast::error::RecvError::Lagged(n)) => log::warn!("talk: fell behind the stick by {n}"),
                Err(_) => return,
            }
        }
    })
}

fn on_message(st: &Arc<AppState>, msg: &Value, rec: &mut Option<Recording>) {
    let evt = msg.get("evt").and_then(Value::as_str);
    let from_agent = msg.get("src").and_then(Value::as_str) == Some("agent");
    if msg.get("cmd").and_then(Value::as_str) == Some("permission")
        || (from_agent
            && (matches!(evt, Some("talk" | "cancel"))
                || (evt == Some("wispr") && msg.get("state").and_then(Value::as_str) != Some("stop"))))
    {
        st.pressed(); // a button press or a gesture (for stick_status)
    }
    if msg.get("cmd").and_then(Value::as_str) == Some("permission") {
        // A / B on an approval prompt
        let (id, decision) = (msg.get("id").and_then(Value::as_str), msg.get("decision").and_then(Value::as_str));
        if let (Some(id), Some(d)) = (id, decision) {
            st.approvals.decide(id, d, "stick");
        }
        return;
    }
    if msg.get("src").and_then(Value::as_str) != Some("agent") {
        return;
    }
    match (msg.get("evt").and_then(Value::as_str), msg.get("state").and_then(Value::as_str)) {
        (Some("talk"), Some("start")) => {
            crate::speak::stop(st); // talking again interrupts a reply being read out
            *rec = Some(Recording::new());
            st.set_activity("listening", "");
        }
        (Some("talk"), Some("stop")) => {
            if let Some(r) = rec.take() {
                tokio::spawn(handle_talk(st.clone(), r));
            }
        }
        (Some("cancel"), _) => {
            log::info!("cancelled on the stick");
            st.brain().cancel();
            crate::speak::stop(st);
            st.set_idle();
        }
        _ => {}
    }
}

/// Always ends by telling the stick something (it waits on "Thinking…").
pub async fn handle_talk(st: Arc<AppState>, rec: Recording) {
    if let Err(e) = handle_talk_inner(&st, rec).await {
        log::error!("talk failed: {e:#}");
        tell_stick(&st, "error", "", &cut(&format!("Something went wrong: {e}"), 120)).await;
        st.set_idle();
    }
}

async fn handle_talk_inner(st: &Arc<AppState>, rec: Recording) -> anyhow::Result<()> {
    let audio = rec.audio();
    let secs = audio.len() as f64 / RATE as f64;
    if secs < 0.6 {
        tell_stick(st, "error", "", "Too short - hold B while you speak.").await;
        st.set_idle();
        return Ok(());
    }
    st.set_activity("transcribing", &format!("{secs:.1}s of audio"));
    if !st.speech.ready() {
        tell_stick(st, "error", "", "Speech model still loading, try again in a moment.").await;
        st.set_idle();
        return Ok(());
    }
    let t = Instant::now();
    let lost = rec.lost();
    let text = st.speech.transcribe(audio).await?;
    log::info!("heard ({secs:.1}s audio, {:.2}s, {lost} lost): {text}", t.elapsed().as_secs_f64());
    if text.is_empty() && secs < 2.5 {
        // a quick press with no words: just close the screen quietly
        tell_stick(st, "cancelled", "", "").await;
        st.set_idle();
        return Ok(());
    }
    if text.is_empty() {
        tell_stick(st, "error", "", "Didn't catch that.").await;
        st.set_idle();
        return Ok(());
    }
    handle_text(st.clone(), text, "voice").await;
    Ok(())
}

fn ms_since(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

/// A request, spoken or typed (core.py `handle_text`).
pub async fn handle_text(st: Arc<AppState>, text: String, source: &str) {
    st.count("requests");
    let started = Instant::now();
    st.hub.emit("heard", json!({"text": text, "source": source}));
    let entry = json!({"t": crate::hub::now(), "heard": text, "source": source});
    let with = |extra: Value| {
        let mut e = entry.clone();
        if let (Some(e), Value::Object(x)) = (e.as_object_mut(), extra) {
            e.extend(x);
        }
        e
    };
    let mut text = text;
    let (reset, rest) = split_new_chat(&text);
    if reset {
        st.brain().forget();
        st.hub.emit("info", json!({"text": "Next request starts a new conversation"}));
        if rest.is_empty() {
            let reply = "OK, new chat.";
            st.hub.emit("reply", json!({"text": reply, "via": "quick", "ms": ms_since(started)}));
            tell_stick(&st, "done", &text, reply).await;
            st.history(&with(json!({"via": "new_chat", "reply": reply})));
            st.set_idle();
            return;
        }
        text = rest;
    }
    if let Some((cmd, slots)) = st.commands.find(&text) {
        let (apps, runner) = (st.apps.clone(), st.runner());
        let c = cmd.clone();
        let result =
            tokio::task::spawn_blocking(move || commands::run(&c, &slots, false, &apps, &runner)).await.ok().flatten();
        if let Some(result) = result {
            st.count("quick");
            st.hub.emit("reply", json!({"text": result, "via": "quick", "ms": ms_since(started)}));
            tell_stick(&st, "done", &text, &result).await;
            st.history(&with(json!({"via": "quick", "command": cmd.get("name"), "reply": result})));
            st.set_idle();
            crate::speak::speak(&st, &result, true, false);
            return;
        }
    }
    let brain = st.brain();
    if brain.busy() {
        tell_stick(&st, "error", &text, "Still working on the last request.").await;
        return;
    }
    st.count("claude");
    st.set_activity("thinking", &text);
    tell_stick(&st, "thinking", &text, "").await;
    let (live, interval) = {
        let cfg = st.cfg.lock().unwrap();
        let live = cfg.get("stream_replies").and_then(Value::as_bool).unwrap_or(true);
        let ms = cfg.get("stream_interval_ms").and_then(Value::as_f64).unwrap_or(300.0);
        (live, Duration::from_secs_f64((ms / 1000.0).max(0.1)))
    };
    let stream = StickStream::new(st.clone(), text.clone(), interval);
    // speech sentence by sentence as the answer is written (speak.rs)
    let speech_cap = crate::speak::stream_speech(&st.cfg.lock().unwrap());
    let utter = speech_cap.map(|n| crate::speak::start(&st, Some(n)));
    let sentences = Arc::new(std::sync::Mutex::new(crate::speak::Sentences::default()));
    let (utter_ref, sentences_ref) = (utter.clone(), sentences.clone());
    let (etx, mut erx) = mpsc::unbounded_channel();
    let mut ask = brain.ask(text.clone(), etx);
    let (st2, stream_ref) = (st.clone(), &stream);
    let on_event = move |ev: BrainEvent| {
        let st = st2.clone();
        let stream = stream_ref;
        let (utter, sentences) = (utter_ref.clone(), sentences_ref.clone());
        async move {
            match ev {
                BrainEvent::Progress(line) => {
                    st.set_activity("working", &line);
                    st.hub.emit("progress", json!({"text": line}));
                    stream.send_now(line).await;
                }
                BrainEvent::Text(answer) => {
                    if let Some(u) = &utter {
                        let done = sentences.lock().unwrap().feed(&answer);
                        for s in done {
                            u.add(&s);
                        }
                    }
                    let shown = plain(&answer);
                    if live && !shown.is_empty() {
                        stream.push(shown);
                    }
                }
            }
        }
    };
    let (ok, reply) = loop {
        tokio::select! {
            biased;
            Some(ev) = erx.recv() => on_event(ev).await,
            r = &mut ask => break r,
        }
    };
    while let Ok(ev) = erx.try_recv() {
        on_event(ev).await;
    }
    let sent = stream.close().await;
    if brain.cancelled() {
        // dismissed on the stick: say nothing more
        if let Some(u) = &utter {
            u.cancel();
        }
        st.history(&with(json!({"via": "claude", "ok": false, "reply": "(cancelled)"})));
        return;
    }
    if sent > 0 {
        log::info!("reply streamed to the stick in {sent} update{}", if sent == 1 { "" } else { "s" });
    }
    st.hub.emit("reply", json!({"text": reply, "via": "claude", "ok": ok, "ms": ms_since(started)}));
    if ok {
        *st.last_reply.lock().unwrap() = Some(json!({"t": crate::hub::now(), "text": reply}));
    }
    tell_stick(&st, if ok { "done" } else { "error" }, &text, &reply).await;
    st.history(&with(json!({"via": "claude", "ok": ok, "reply": reply})));
    st.set_idle();
    let started_speech = !sentences.lock().unwrap().text.is_empty();
    match utter {
        Some(u) if ok && started_speech && !u.cancelled() => {
            let rest = sentences.lock().unwrap().finish();
            for s in rest {
                u.add(&s);
            }
            u.close();
            let spoken = u.spoken();
            if !spoken.is_empty() {
                st.hub.emit("spoken", json!({"text": spoken.join(" ")}));
            }
        }
        u => {
            if let Some(u) = u {
                u.cancel();
            }
            crate::speak::speak(&st, &reply, false, false);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Options;
    use crate::commands::Action;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use stick_link::adpcm;
    use stick_link::fake::FakeStick;
    use stick_link::{Backend, LinkState};

    #[test]
    fn stick_text_like_python() {
        for (raw, safe) in [
            ("plain", "plain"),
            ("28°C — “quoted” …", "28°C — “quoted” …"),
            ("café naïve", "cafe naive"),
            ("emoji 😀 gone", "emoji  gone"),
            ("", ""),
        ] {
            assert_eq!(stick_text(raw), safe);
        }
    }

    #[test]
    fn new_chat_phrases() {
        assert_eq!(split_new_chat("Start over."), (true, String::new()));
        assert_eq!(split_new_chat("New chat. Weather?"), (true, "Weather?".into()));
        assert_eq!(split_new_chat("let's start a new conversation please"), (true, String::new()));
        assert_eq!(split_new_chat("new chat about rust"), (false, "new chat about rust".into()));
        assert_eq!(split_new_chat("what's new"), (false, "what's new".into()));
    }

    #[test]
    fn plain_for_the_screen() {
        assert_eq!(plain("**Bold** and `code`\n\n# Title"), "Bold and code Title");
        assert_eq!(plain("Look:\n```rust\nfn x() {}\n```\nDone."), "Look: Done.");
        assert_eq!(plain("Writing ```py\nprint("), "Writing");
        let long = "word ".repeat(100);
        let p = plain(&long);
        assert!(p.ends_with('…') && p.chars().count() <= 278);
    }

    // --- test_core.py, with the fake stick ------------------------------------------------------

    pub struct FakeStt {
        pub text: Arc<Mutex<Result<String, String>>>,
        pub calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl stt::Stt for FakeStt {
        fn load(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn transcribe(&mut self, _a: &[f32], _r: u32) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.text.lock().unwrap().clone().map_err(|e| anyhow::anyhow!(e))
        }
        fn name(&self) -> String {
            "fake".into()
        }
    }

    pub struct T {
        pub st: Arc<AppState>,
        pub fake: FakeStick,
        pub said: Arc<Mutex<Result<String, String>>>,
        pub calls: Arc<std::sync::atomic::AtomicUsize>,
        pub ran: Arc<Mutex<Vec<Action>>>,
    }

    pub async fn agent(name: &str, stt_ready: bool) -> T {
        let home = std::env::temp_dir().join(format!("stickybot-rs-talk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let opts = Options { port: 8670, home, fake_stick: true, import_from: None };
        let fake = FakeStick::new();
        let st = AppState::new(&opts, Backend::Fake(fake.clone())).unwrap();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let r = ran.clone();
        st.set_runner(Arc::new(move |a: &Action| {
            r.lock().unwrap().push(a.clone());
            String::new()
        }));
        st.link.spawn();
        spawn(st.clone());
        let said = Arc::new(Mutex::new(Ok("hello".to_string())));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        if stt_ready {
            st.speech.start(Box::new(FakeStt { text: said.clone(), calls: calls.clone() }));
            for _ in 0..200 {
                if st.speech.ready() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            calls.store(0, Ordering::Relaxed); // the warm-up
        }
        st.update_config(json!({"stick": {"address": "FAKE", "name": "fake"}}).as_object().unwrap().clone()).unwrap();
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected))
            .await
            .unwrap()
            .unwrap();
        T { st, fake, said, calls, ran }
    }

    pub fn frames(seconds: f64) -> Vec<Vec<u8>> {
        let n = (16000.0 * seconds) as usize;
        let pcm: Vec<i16> = (0..n).map(|i| (8000.0 * (i as f64 / 3.0).sin()) as i16).collect();
        adpcm::encode(&pcm, 320)
    }

    fn states(t: &T) -> Vec<(String, String)> {
        t.fake.screens().into_iter().map(|(s, _, text)| (s, text)).collect()
    }

    /// Until the stick has been told how it ended.
    pub async fn ended(t: &T) -> Vec<(String, String)> {
        for _ in 0..500 {
            let st = states(t);
            if st.last().is_some_and(|(s, _)| matches!(s.as_str(), "done" | "error" | "cancelled")) {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        states(t)
    }

    async fn talk(t: &T, seconds: f64) -> Vec<(String, String)> {
        t.fake.talk(&frames(seconds));
        ended(t).await
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    fn history(t: &T) -> Vec<Value> {
        std::fs::read_to_string(t.st.paths.history())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn quick_press_without_words_closes_quietly() {
        let t = agent("quick", true).await;
        *t.said.lock().unwrap() = Ok(String::new());
        assert_eq!(talk(&t, 1.0).await, vec![pair("cancelled", "")]);
        assert_eq!(t.st.agent_state(), "ready");
    }

    #[tokio::test]
    async fn too_short() {
        let t = agent("short", true).await;
        assert_eq!(talk(&t, 0.3).await, vec![pair("error", "Too short - hold B while you speak.")]);
        assert_eq!(t.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn long_silence_says_didnt_catch_that() {
        let t = agent("silence", true).await;
        *t.said.lock().unwrap() = Ok(String::new());
        assert_eq!(talk(&t, 3.0).await, vec![pair("error", "Didn't catch that.")]);
    }

    #[tokio::test]
    async fn transcription_crash_still_answers_the_stick() {
        let t = agent("crash", true).await;
        *t.said.lock().unwrap() = Err("There is no Stream(cpu, 1) in current thread.".into());
        let st = talk(&t, 1.5).await;
        assert_eq!(st.last().unwrap().0, "error");
        assert!(st.last().unwrap().1.contains("Something went wrong"));
        assert_eq!(t.st.agent_state(), "ready");
    }

    #[tokio::test]
    async fn model_not_loaded_yet() {
        let t = agent("noload", false).await;
        assert_eq!(talk(&t, 1.5).await, vec![pair("error", "Speech model still loading, try again in a moment.")]);
    }

    #[tokio::test]
    async fn quick_command() {
        let t = agent("cmd", true).await;
        t.st.commands
            .save(
                json!({"commands": [{"name": "GitHub PR", "phrases": ["pr {n}", "open pr {n}"],
                                        "open_url": "https://github.com/acme/app/pull/{n}"}]})
                .as_object()
                .unwrap(),
            )
            .unwrap();
        *t.said.lock().unwrap() = Ok("Open PR twelve thirty four.".into());
        assert_eq!(talk(&t, 1.5).await, vec![pair("done", "Opened GitHub PR #1234")]);
        assert_eq!(*t.ran.lock().unwrap(), vec![Action::Open(vec!["https://github.com/acme/app/pull/1234".into()])]);
        let h = history(&t);
        assert_eq!(h.last().unwrap()["via"], "quick");
        assert_eq!(h.last().unwrap()["command"], "GitHub PR");
        assert_eq!(h.last().unwrap()["heard"], "Open PR twelve thirty four.");
        assert_eq!(t.st.status()["stats"]["quick"], 1);
        // and the heard line went to the stick with it
        assert_eq!(t.fake.screens()[0].1, "Open PR twelve thirty four.");
    }

    #[tokio::test]
    async fn the_talk_shows_on_the_dashboard() {
        let t = agent("events", true).await;
        let (_, mut rx) = t.st.hub.subscribe(0);
        *t.said.lock().unwrap() = Ok("Open Slack".into());
        t.fake.talk(&frames(1.0));
        let mut kinds = Vec::new();
        while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            let k = ev["kind"].as_str().unwrap().to_string();
            if k == "state" {
                kinds.push(format!("state:{}", ev["state"].as_str().unwrap()));
            } else {
                kinds.push(k);
            }
            if kinds.last().is_some_and(|k| k == "reply") {
                break;
            }
        }
        assert_eq!(kinds[..3], ["state:listening", "state:transcribing", "heard"]);
        assert!(kinds.contains(&"reply".to_string()));
    }

    // --- with a brain ------------------------------------------------------------------------------

    /// Writes its answer in chunks: text is appended, `None` withdraws it (a preamble), "TOOL:x" is
    /// a tool line (test_core.py `StreamingBrain`).
    pub struct FakeBrain {
        pub chunks: Vec<Option<String>>,
        pub pause: Duration,
        pub ok: bool,
        pub busy: AtomicBool,
        pub cancelled: Arc<AtomicBool>,
        pub forgot: AtomicBool,
        pub asked: Arc<Mutex<Vec<String>>>,
        pub reply: Option<String>,
    }

    impl FakeBrain {
        pub fn new(chunks: &[&str]) -> Self {
            Self {
                chunks: chunks.iter().map(|c| (!c.is_empty()).then(|| c.to_string())).collect(),
                pause: Duration::ZERO,
                ok: true,
                busy: AtomicBool::new(false),
                cancelled: Arc::new(AtomicBool::new(false)),
                forgot: AtomicBool::new(false),
                asked: Arc::new(Mutex::new(Vec::new())),
                reply: None,
            }
        }
    }

    impl Brain for FakeBrain {
        fn busy(&self) -> bool {
            self.busy.load(Ordering::Relaxed)
        }
        fn ask(&self, text: String, ev: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)> {
            self.asked.lock().unwrap().push(text);
            let (chunks, pause, ok, cancelled, reply) =
                (self.chunks.clone(), self.pause, self.ok, self.cancelled.clone(), self.reply.clone());
            Box::pin(async move {
                let mut acc = String::new();
                for c in chunks {
                    match c {
                        None => {
                            acc.clear();
                            let _ = ev.send(BrainEvent::Text(String::new()));
                        }
                        Some(c) if c.starts_with("TOOL:") => {
                            let _ = ev.send(BrainEvent::Progress(c[5..].to_string()));
                        }
                        Some(c) => {
                            acc.push_str(&c);
                            let _ = ev.send(BrainEvent::Text(acc.clone()));
                        }
                    }
                    tokio::time::sleep(pause).await;
                    if cancelled.load(Ordering::Relaxed) {
                        return (false, "Stopped.".into());
                    }
                }
                (ok, reply.unwrap_or_else(|| acc.trim().to_string()))
            })
        }
        fn cancel(&self) {
            self.cancelled.store(true, Ordering::Relaxed);
        }
        fn cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Relaxed)
        }
        fn forget(&self) {
            self.forgot.store(true, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn without_a_brain_the_stick_says_so() {
        let t = agent("nobrain", true).await;
        *t.said.lock().unwrap() = Ok("what's the weather".into());
        assert_eq!(talk(&t, 2.0).await, vec![pair("thinking", ""), pair("error", NO_BRAIN)]);
    }

    #[tokio::test]
    async fn claude_request() {
        let t = agent("claude", true).await;
        let b = Arc::new(FakeBrain::new(&["TOOL:Searching: weather", "It's 28°C."]));
        t.st.set_brain(b.clone());
        *t.said.lock().unwrap() = Ok("what's the weather".into());
        let st = talk(&t, 2.0).await;
        assert_eq!(st[0], pair("thinking", ""));
        assert!(st.contains(&pair("working", "Searching: weather")));
        assert_eq!(st.last().unwrap(), &pair("done", "It's 28°C."));
        assert_eq!(*b.asked.lock().unwrap(), vec!["what's the weather"]);
        assert_eq!(t.st.last_reply.lock().unwrap().as_ref().unwrap()["text"], "It's 28°C.");
    }

    #[tokio::test]
    async fn claude_failure_is_an_error() {
        let t = agent("fail", true).await;
        let mut b = FakeBrain::new(&[]);
        b.ok = false;
        b.reply = Some("Claude Code failed.".into());
        t.st.set_brain(Arc::new(b));
        handle_text(t.st.clone(), "do the thing".into(), "typed").await;
        assert_eq!(states(&t).last().unwrap(), &pair("error", "Claude Code failed."));
        assert!(t.st.last_reply.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn busy_brain() {
        let t = agent("busy", true).await;
        let b = FakeBrain::new(&["x"]);
        b.busy.store(true, Ordering::Relaxed);
        t.st.set_brain(Arc::new(b));
        handle_text(t.st.clone(), "another one".into(), "typed").await;
        assert_eq!(states(&t), vec![pair("error", "Still working on the last request.")]);
    }

    #[tokio::test]
    async fn cancel_from_the_stick() {
        let t = agent("cancel", true).await;
        let mut b = FakeBrain::new(&["", "", ""]); // writes nothing, slowly
        b.pause = Duration::from_millis(100);
        t.st.set_brain(Arc::new(b));
        let task = tokio::spawn(handle_text(t.st.clone(), "long job".into(), "typed"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        t.fake.notify(&json!({"src": "agent", "evt": "cancel"}));
        task.await.unwrap();
        assert_eq!(states(&t).iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), vec!["thinking"]);
        assert_eq!(t.st.agent_state(), "ready");
    }

    #[tokio::test]
    async fn new_chat_phrase() {
        for (said, asked) in [("Start over.", vec![]), ("New chat. Weather?", vec!["Weather?"])] {
            let t = agent(&format!("newchat{}", asked.len()), true).await;
            let b = Arc::new(FakeBrain::new(&["It's sunny."]));
            t.st.set_brain(b.clone());
            handle_text(t.st.clone(), said.into(), "typed").await;
            assert!(b.forgot.load(Ordering::Relaxed));
            assert_eq!(*b.asked.lock().unwrap(), asked);
            if asked.is_empty() {
                assert_eq!(states(&t), vec![pair("done", "OK, new chat.")]);
            }
        }
    }

    #[tokio::test]
    async fn reply_streams_to_the_stick_throttled() {
        let t = agent("stream", true).await;
        t.st.cfg.lock().unwrap().insert("stream_interval_ms".into(), json!(100));
        let words: Vec<String> = (0..40).map(|i| format!("w{i} ")).collect();
        let mut b = FakeBrain::new(&words.iter().map(String::as_str).collect::<Vec<_>>());
        b.pause = Duration::from_millis(10);
        t.st.set_brain(Arc::new(b));
        handle_text(t.st.clone(), "count".into(), "typed").await;
        let st = states(&t);
        let working: Vec<&String> = st.iter().filter(|(s, _)| s == "working").map(|(_, t)| t).collect();
        assert_eq!(st[0], pair("thinking", ""));
        assert_eq!(st.last().unwrap().1, (0..40).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" "));
        assert!((2..=7).contains(&working.len()), "{} updates", working.len());
        assert!(working.windows(2).all(|w| w[0].len() <= w[1].len()));
        let h = history(&t);
        assert_eq!(h.last().unwrap()["reply"], t.st.last_reply.lock().unwrap().as_ref().unwrap()["text"]);
    }

    #[tokio::test]
    async fn streaming_off_shows_only_tool_lines() {
        let t = agent("nostream", true).await;
        t.st.cfg.lock().unwrap().insert("stream_replies".into(), json!(false));
        t.st.set_brain(Arc::new(FakeBrain::new(&["TOOL:Searching: x", "It's ", "sunny."])));
        handle_text(t.st.clone(), "weather".into(), "typed").await;
        assert_eq!(
            states(&t),
            vec![pair("thinking", ""), pair("working", "Searching: x"), pair("done", "It's sunny.")]
        );
    }

    #[tokio::test]
    async fn stick_text_limits_hold_while_streaming() {
        let t = agent("limits", true).await;
        t.st.cfg.lock().unwrap().insert("stream_interval_ms".into(), json!(100));
        let long = format!("Café ✨ **bold** {}", "word ".repeat(120));
        t.st.set_brain(Arc::new(FakeBrain::new(&[long.as_str()])));
        handle_text(t.st.clone(), "long".into(), "typed").await;
        for (s, _, text) in t.fake.screens() {
            if s == "working" {
                assert!(text.chars().count() <= 300);
                assert!(text.chars().all(|c| c.is_ascii() || STICK_EXTRA.contains(c)));
                assert!(!text.contains('*'));
            }
        }
    }

    #[tokio::test]
    async fn a_preamble_is_withdrawn() {
        let t = agent("preamble", true).await;
        t.st.set_brain(Arc::new(FakeBrain::new(&["Let me check. ", "", "TOOL:Searching: weather", "It's sunny."])));
        handle_text(t.st.clone(), "weather".into(), "typed").await;
        let st = states(&t);
        assert!(st.contains(&pair("working", "Searching: weather")));
        assert_eq!(st.last().unwrap(), &pair("done", "It's sunny."));
    }
}
