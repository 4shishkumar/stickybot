//! Hold B to talk (core.py `_on_message` / `_handle_talk` / `handle_text`): the stick streams its
//! mic as ADPCM frames between `{"evt":"talk","state":"start"}` and `…"stop"`; the agent decodes
//! them, transcribes, and answers on the stick's screen with
//! `{"src":"agent","state":"thinking"|"working"|"done"|"error"|"cancelled","heard","text"}`.
//!
//! The Mac shortcut submits PCM directly through [`MacTurn`]. Both inputs share speech recognition
//! and the selected [`Brain`]. The turn owner determines where progress and the answer go.

use crate::AppState;
use crate::commands;
use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use stick_link::FromStick;
use stick_link::adpcm::{RATE, Recording};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

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

// Brain adapters compact results for the stick. Scope formatting to the actual ask future so a
// native reply keeps Unicode, paragraphs and its complete text, even during concurrent device turns.
tokio::task_local! { static MAC_REPLY: bool; }
pub(crate) fn brain_reply(text: &str) -> String {
    if MAC_REPLY.try_with(|mac| *mac).unwrap_or(false) { text.trim().to_owned() } else { plain(text) }
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
    /// The current provider request is running one or more permission hooks.
    PermissionHook(bool),
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
        let generation = st.link.bound_generation();
        tokio::spawn(async move {
            let mut pending: Option<String> = None;
            let mut shown: Option<String> = None;
            let mut last: Option<Instant> = None;
            let show = |text: String| {
                let st = st.clone();
                let heard = heard.clone();
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move {
                    if st.link.generation() == generation {
                        st.link.with_generation(generation, tell_stick(&st, "working", &heard, &text)).await;
                    }
                }
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

/// The reply view's text (bytes, UTF-8; the stick keeps 3072).
pub const READER_MAX: usize = 3000;
/// ... sent in parts this long after "done".
pub const PART_BYTES: usize = 480;

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*```.*$\n?").expect("valid"));
static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*|__|`").expect("valid"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*#{1,6}[ \t]*").expect("valid"));
static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*>[ \t]?").expect("valid"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^([ \t]*)[-*][ \t]+").expect("valid"));
static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+\n").expect("valid"));
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("valid"));

/// The whole reply for the stick's reply view (firmware 2.6.0; core.py `reader_text`): its
/// paragraphs and list items kept, markdown marks dropped, what the stick can draw, at most
/// `limit` bytes.
pub fn reader_text(text: &str, limit: usize) -> String {
    let t = FENCE.replace_all(text, "");
    let t = BOLD.replace_all(&t, "");
    let t = HEADING.replace_all(&t, "");
    let t = QUOTE.replace_all(&t, "");
    let t = BULLET.replace_all(&t, "${1}• ");
    let t = stick_text(&t);
    let t = TRAILING.replace_all(&t, "\n");
    let t = BLANKS.replace_all(&t, "\n\n").trim().to_string();
    if t.len() <= limit {
        return t;
    }
    let mut end = limit - 3;
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", t[..end].trim_end())
}

/// `text` in pieces of at most `size` UTF-8 bytes, never splitting a character (core.py `reply_parts`).
pub fn reply_parts(text: &str, size: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if !cur.is_empty() && cur.len() + ch.len_utf8() > size {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Shows `state` on the stick (core.py `tell_stick`). A "done" longer than the screen's 300
/// characters is followed by the whole reply in parts (`{"reply":{"rid","i","t"}}`) for the
/// reply view (firmware 2.6.0; older firmware takes them for hellos and keeps the 300).
pub async fn tell_stick(st: &AppState, state: &str, heard: &str, text: &str) -> bool {
    let mut msg = json!({"src": "agent", "state": state, "heard": cut(&stick_text(heard), 200),
                         "text": cut(&stick_text(text), 300)});
    let mut parts = Vec::new();
    let mut rid = 0;
    if state == "done" {
        *st.last_done.lock().unwrap() = text.to_string();
        if stick_text(text).chars().count() > 300 {
            parts = reply_parts(&reader_text(text, READER_MAX), PART_BYTES);
            rid = st.reply_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            msg["rid"] = json!(rid);
            msg["parts"] = json!(parts.len());
        }
    }
    let ok = st.link.send(&msg).await;
    for (i, p) in parts.iter().enumerate() {
        if !st.link.send(&json!({"src": "agent", "reply": {"rid": rid, "i": i, "t": p}})).await {
            break;
        }
    }
    ok
}

/// A device's brain has one turn at a time, including the capture and transcription preceding ask().
/// The owner stays reserved until that turn unwinds, even if the input is cancelled.
pub(crate) struct Owner {
    mac: bool,
    generation: u64,
    recording: AtomicBool,
    cancelled: AtomicBool,
    approval: watch::Sender<String>,
}

pub type MacDisplay = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

struct VoiceTurn {
    st: Arc<AppState>,
    owner: Arc<Owner>,
    display: Option<MacDisplay>,
}

const BUSY: &str = "Still working on the last request.";

impl VoiceTurn {
    fn reserve(st: Arc<AppState>, display: Option<MacDisplay>, generation: u64) -> Result<Self, String> {
        let mut slot = st.voice_owner.lock().unwrap();
        // The event loop may not have observed a quick reconnect yet. A buffer still waiting
        // for key-up from the old stick can never be submitted on the new connection.
        if slot.as_ref().is_some_and(|old| {
            !old.mac && old.generation != st.link.generation() && old.recording.load(Ordering::Acquire)
        }) {
            *slot = None;
        }
        if slot.is_some() || st.brain().busy() {
            return Err(BUSY.into());
        }
        let owner = Arc::new(Owner {
            mac: display.is_some(),
            generation,
            recording: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            approval: watch::channel(String::new()).0,
        });
        *slot = Some(owner.clone());
        drop(slot);
        Ok(Self { st, owner, display })
    }

    fn active(&self) -> bool {
        !self.owner.cancelled.load(Ordering::Acquire)
            && (self.owner.mac || self.st.link.generation() == self.owner.generation)
    }

    async fn show(&self, state: &str, heard: &str, text: &str) {
        if !self.active() {
            return;
        }
        if let Some(display) = &self.display {
            let phase = match state {
                "done" => "reply",
                "cancelled" => "error",
                phase => phase,
            };
            display(phase, heard, text);
        } else {
            tell_stick(&self.st, state, heard, text).await;
        }
    }
}

impl Drop for VoiceTurn {
    fn drop(&mut self) {
        let mut slot = self.st.voice_owner.lock().unwrap();
        if slot.as_ref().is_some_and(|owner| Arc::ptr_eq(owner, &self.owner)) {
            self.st.set_idle();
            *slot = None;
        }
    }
}

/// An in-memory reservation for the selected device's agent. Dropping an unsubmitted turn
/// discards the recording. No stick connection is required to ask the brain.
pub struct MacTurn(VoiceTurn);

impl MacTurn {
    pub fn begin(st: Arc<AppState>, notify: MacDisplay) -> Result<Self, String> {
        let generation = st.link.generation();
        let turn = VoiceTurn::reserve(st, Some(notify), generation)?;
        crate::speak::stop(&turn.st);
        turn.st.set_activity("listening", "Mac microphone");
        Ok(Self(turn))
    }

    /// Mono 16 kHz, captured by the native app. The callback remains scoped to this turn.
    pub async fn submit(self, audio: Vec<f32>) {
        self.0.show("transcribing", "", "").await;
        if let Err(e) = handle_audio(&self.0, audio, 0).await {
            self.0.show("error", "", &format!("Could not transcribe the recording: {e}")).await;
        }
    }
}

/// The Mac panel's cancel: stops only a keyboard-started turn (a stick turn is the stick's to cancel).
pub fn cancel_mac_turn(st: &Arc<AppState>) -> bool {
    let owner = st.voice_owner.lock().unwrap().clone();
    let Some(owner) = owner.filter(|owner| owner.mac) else {
        return false;
    };
    owner.cancelled.store(true, Ordering::Release);
    st.brain().cancel();
    crate::speak::stop(st);
    true
}

/// HTTP tool calls from a stick-originated voice turn must retain the original BLE generation.
pub(crate) fn voice_tool_generation(st: &AppState, ctx: &crate::tool_policy::Context) -> Option<u64> {
    let slot = st.voice_owner.lock().unwrap();
    slot.as_ref()
        .filter(|owner| {
            !owner.cancelled.load(Ordering::Acquire)
                && ctx.active()
                && st.mcp.session.current().is_some_and(|scope| scope.turn == ctx.scope.turn)
        })
        .map(|owner| owner.generation)
}

/// Identify the same live brain request across the native caller and its HTTP MCP connection.
pub(crate) fn mac_agent_owner(st: &AppState, ctx: &crate::tool_policy::Context) -> Option<Arc<Owner>> {
    let slot = st.voice_owner.lock().unwrap();
    slot.as_ref()
        .filter(|owner| {
            owner.mac
                && ctx.caller.brain()
                && ctx.active()
                && st.mcp.session.current().is_some_and(|s| s.turn == ctx.scope.turn)
        })
        .cloned()
}

impl Owner {
    /// Display-only notification; the broker still requires the device's exact, one-use grant.
    pub(crate) fn waiting_for_approval(&self, destination: &str) {
        self.approval.send_replace(format!("Confirm on your stick: {destination}"));
    }
}

/// A button can cancel only its generation. A connection change can cancel only a
/// previous generation. Either way a Mac-originated turn keeps its reservation.
fn cancel_stick_turn(st: &Arc<AppState>, button: Option<u64>) {
    let slot = st.voice_owner.lock().unwrap();
    if let Some(owner) = slot.as_ref() {
        let matches = match button {
            Some(generation) => owner.generation == generation,
            None => owner.generation != st.link.generation() || !st.link.connected(),
        };
        if owner.mac || !matches {
            return;
        }
        owner.cancelled.store(true, Ordering::Release);
        st.brain().cancel();
    } else if button.is_none() && st.link.connected() {
        return;
    }
    crate::speak::stop_stick(st);
}

type StickRecording = Option<(Recording, VoiceTurn)>;

fn discard_stale_recording(rec: &mut StickRecording) {
    if rec.as_ref().is_some_and(|(_, turn)| !turn.active()) {
        *rec = None;
    }
}

/// Listens to the link for talks and cancels. Runs for the life of the app.
pub fn spawn(st: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    let mut rx = st.link.incoming();
    let mut connection = st.link.watch_state();
    tokio::spawn(async move {
        let mut rec: StickRecording = None;
        loop {
            let message = tokio::select! {
                result = rx.recv() => result,
                result = connection.changed() => {
                    if result.is_err() { return; }
                    cancel_stick_turn(&st, None);
                    discard_stale_recording(&mut rec);
                    continue;
                }
            };
            match message {
                Ok((generation, message)) => receive(&st, generation, message, &mut rec).await,
                Err(broadcast::error::RecvError::Lagged(n)) => log::warn!("talk: fell behind the stick by {n}"),
                Err(_) => return,
            }
        }
    })
}

async fn receive(st: &Arc<AppState>, generation: u64, message: FromStick, rec: &mut StickRecording) {
    discard_stale_recording(rec);
    if generation != st.link.generation() {
        return;
    }
    st.link
        .with_generation(generation, async {
            match message {
                FromStick::Audio(frame) => {
                    // Wispr frames go to the wireless microphone.
                    if let Some((recording, turn)) = rec
                        && turn.active()
                        && !st.wmic.streaming()
                    {
                        recording.add(&frame);
                    }
                }
                FromStick::Message(msg) => on_message(st, &msg, rec),
            }
        })
        .await;
}

fn on_message(st: &Arc<AppState>, msg: &Value, rec: &mut StickRecording) {
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
            st.approvals.decide_from_device(id, d, st);
        }
        return;
    }
    if msg.get("src").and_then(Value::as_str) != Some("agent") {
        return;
    }
    match (msg.get("evt").and_then(Value::as_str), msg.get("state").and_then(Value::as_str)) {
        (Some("talk"), Some("start")) => {
            if rec.is_some() {
                return;
            }
            let generation = st.link.bound_generation();
            let Ok(turn) = VoiceTurn::reserve(st.clone(), None, generation) else {
                let st = st.clone();
                tokio::spawn(async move {
                    st.link.with_generation(generation, tell_stick(&st, "error", "", BUSY)).await;
                });
                return;
            };
            crate::speak::stop(st); // talking again interrupts a reply being read out
            turn.owner.recording.store(true, Ordering::Release);
            *rec = Some((Recording::new(), turn));
            st.set_activity("listening", "");
        }
        (Some("talk"), Some("stop")) => {
            if let Some((r, turn)) = rec.take() {
                turn.owner.recording.store(false, Ordering::Release);
                tokio::spawn(handle_stick_recording(turn, r));
            }
        }
        (Some("button_action"), _) => {
            // This path receives notifications from this stick's BLE link only.
            crate::buttons::pressed(st, msg);
        }
        (Some("speak"), _) => {
            if st.voice_owner.lock().unwrap().as_ref().is_some_and(|o| o.mac) {
                return;
            }
            // the reply view's "listen" (firmware 2.6.0)
            if msg.get("what").and_then(Value::as_str) == Some("stop") {
                log::info!("reading aloud: stopped on the stick");
                crate::speak::stop_stick(st);
            } else {
                st.pressed();
                crate::speak::speak_last(st);
            }
        }
        (Some("cancel"), _) => {
            log::info!("cancelled on the stick");
            cancel_stick_turn(st, Some(st.link.bound_generation()));
            *rec = None;
        }
        _ => {}
    }
}

/// Always ends by telling the stick something (it waits on "Thinking…").
pub fn handle_talk(st: Arc<AppState>, rec: Recording) -> impl std::future::Future<Output = ()> + Send {
    let generation = st.link.bound_generation();
    handle_talk_at(st, rec, generation)
}
fn handle_talk_at(st: Arc<AppState>, rec: Recording, generation: u64) -> impl std::future::Future<Output = ()> + Send {
    let link = st.link.clone();
    async move {
        if link.generation() != generation {
            return;
        }
        match VoiceTurn::reserve(st.clone(), None, generation) {
            Ok(turn) => handle_stick_recording(turn, rec).await,
            Err(e) => {
                link.with_generation(generation, tell_stick(&st, "error", "", &e)).await;
            }
        }
    }
}

async fn handle_stick_recording(turn: VoiceTurn, rec: Recording) {
    let link = turn.st.link.clone();
    link.with_generation(turn.owner.generation, async {
        if turn.active()
            && let Err(e) = handle_audio(&turn, rec.audio(), rec.lost()).await
        {
            log::error!("talk failed: {e:#}");
            turn.show("error", "", &cut(&format!("Something went wrong: {e}"), 120)).await;
        }
    })
    .await;
}

async fn handle_audio(turn: &VoiceTurn, audio: Vec<f32>, lost: usize) -> anyhow::Result<()> {
    let st = &turn.st;
    let secs = audio.len() as f64 / RATE as f64;
    if secs < 0.6 {
        let msg = if turn.owner.mac {
            "Too short — hold the shortcut while you speak."
        } else {
            "Too short - hold B while you speak."
        };
        turn.show("error", "", msg).await;
        return Ok(());
    }
    st.set_activity("transcribing", &format!("{secs:.1}s of audio"));
    if !st.speech.ready() {
        turn.show("error", "", "Speech model still loading, try again in a moment.").await;
        return Ok(());
    }
    let t = Instant::now();
    let text = st.speech.transcribe(audio).await?;
    if !turn.active() {
        return Ok(());
    }
    log::info!("heard ({secs:.1}s audio, {:.2}s, {lost} lost)", t.elapsed().as_secs_f64());
    if text.is_empty() {
        if !turn.owner.mac && secs < 2.5 {
            turn.show("cancelled", "", "").await;
        } else {
            turn.show("error", "", "Didn't catch that.").await;
        }
        return Ok(());
    }
    let source = if turn.owner.mac { "mac_voice" } else { "voice" };
    handle_text_inner(turn, text, source).await;
    Ok(())
}

fn ms_since(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

/// A request, spoken or typed (core.py `handle_text`).
pub fn handle_text(
    st: Arc<AppState>,
    text: String,
    source: &str,
) -> impl std::future::Future<Output = ()> + Send + use<> {
    let link = st.link.clone();
    let generation = link.bound_generation();
    let source = source.to_owned();
    async move {
        if link.generation() != generation {
            return;
        }
        match VoiceTurn::reserve(st.clone(), None, generation) {
            Ok(turn) => link.with_generation(generation, handle_text_inner(&turn, text, &source)).await,
            Err(e) => {
                link.with_generation(generation, tell_stick(&st, "error", "", &e)).await;
            }
        }
    }
}
async fn handle_text_inner(turn: &VoiceTurn, text: String, source: &str) {
    let st = turn.st.clone();
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
            turn.show("done", &text, reply).await;
            st.history(&with(json!({"via": "new_chat", "reply": reply})));
            return;
        }
        text = rest;
    }
    // Mac and assistant-profile turns go through the brain/broker. Legacy saved shortcuts can run commands.
    let shortcuts = {
        let cfg = st.cfg.lock().unwrap();
        let caller = match crate::brains::selected(&cfg) {
            "openrouter" => crate::tool_policy::Caller::OpenRouter,
            "local" => crate::tool_policy::Caller::Local,
            _ => crate::tool_policy::Caller::Claude,
        };
        crate::tool_policy::Profile::configured(&cfg, caller).allows("quick_command")
    };
    if !turn.owner.mac
        && shortcuts
        && let Some((cmd, slots)) = st.commands.find(&text)
    {
        if let Some(spec) = cmd.get("ir") {
            // an IR remote button (ir.rs): sent by the stick
            let (ok, result) = crate::ir::voice(&st, spec).await;
            st.count("quick");
            st.hub.emit("reply", json!({"text": result, "via": "quick", "ms": ms_since(started)}));
            turn.show(if ok { "done" } else { "error" }, &text, &result).await;
            st.history(&with(json!({"via": "quick", "command": cmd.get("name"), "reply": result})));
            return;
        }
        if let Some(what) = cmd.get("calendar").and_then(Value::as_str) {
            // a question about the day (connections/calendar.rs): answered from the Mac's Calendar, no model
            let (ok, result) = crate::connections::calendar::voice(&st, what).await;
            st.count("quick");
            st.hub.emit("reply", json!({"text": result, "via": "quick", "ms": ms_since(started)}));
            turn.show(if ok { "done" } else { "error" }, &text, &result).await;
            // the Activity log is a file: it keeps that the question was asked, not the meetings in the answer
            st.history(&with(json!({"via": "quick", "command": cmd.get("name"), "reply": "(from your calendar)"})));
            if ok {
                crate::speak::speak(&st, &result, true, false);
            }
            return;
        }
        let (apps, runner) = (st.apps.clone(), st.runner());
        let c = cmd.clone();
        let result =
            tokio::task::spawn_blocking(move || commands::run(&c, &slots, false, &apps, &runner)).await.ok().flatten();
        if let Some(result) = result {
            st.count("quick");
            st.hub.emit("reply", json!({"text": result, "via": "quick", "ms": ms_since(started)}));
            turn.show("done", &text, &result).await;
            st.history(&with(json!({"via": "quick", "command": cmd.get("name"), "reply": result})));
            crate::speak::speak(&st, &result, true, false);
            return;
        }
    }
    let brain = st.brain();
    if brain.busy() {
        turn.show("error", &text, "Still working on the last request.").await;
        return;
    }
    st.count("claude");
    st.set_activity("thinking", &text);
    turn.show("thinking", &text, "").await;
    let (live, interval) = {
        let cfg = st.cfg.lock().unwrap();
        let live = cfg.get("stream_replies").and_then(Value::as_bool).unwrap_or(true);
        let ms = cfg.get("stream_interval_ms").and_then(Value::as_f64).unwrap_or(300.0);
        (live, Duration::from_secs_f64((ms / 1000.0).max(0.1)))
    };
    let stream = (!turn.owner.mac).then(|| StickStream::new(st.clone(), text.clone(), interval));
    // speech sentence by sentence as the answer is written (speak.rs)
    let speech_cap = crate::speak::stream_speech(&st.cfg.lock().unwrap());
    let utter = speech_cap.map(|n| crate::speak::start_for(&st, Some(n), turn.owner.mac));
    let sentences = Arc::new(std::sync::Mutex::new(crate::speak::Sentences::default()));
    let (utter_ref, sentences_ref) = (utter.clone(), sentences.clone());
    let (etx, mut erx) = mpsc::unbounded_channel();
    let mut prompts = turn.owner.approval.subscribe();
    let mut ask = Box::pin(MAC_REPLY.scope(turn.owner.mac, brain.ask(text.clone(), etx)));
    let (st2, stream_ref, heard) = (st.clone(), &stream, text.as_str());
    let on_event = move |ev: BrainEvent| {
        let st = st2.clone();
        let stream = stream_ref;
        let (utter, sentences) = (utter_ref.clone(), sentences_ref.clone());
        async move {
            if !turn.active() {
                return;
            }
            match ev {
                BrainEvent::PermissionHook(waiting) => {
                    if turn.owner.mac {
                        if waiting {
                            turn.show("approval", heard, "Claude is requesting permission.").await;
                        } else {
                            turn.show("thinking", heard, "").await;
                        }
                    }
                }
                BrainEvent::Progress(line) => {
                    st.set_activity("working", &line);
                    st.hub.emit("progress", json!({"text": line}));
                    if let Some(stream) = stream {
                        stream.send_now(line).await;
                    } else {
                        turn.show("thinking", heard, &line).await;
                    }
                }
                BrainEvent::Text(answer) => {
                    if let Some(u) = &utter {
                        let done = sentences.lock().unwrap().feed(&answer);
                        for s in done {
                            u.add(&s);
                        }
                    }
                    if let Some(stream) = stream {
                        let shown = plain(&answer);
                        if live && !shown.is_empty() {
                            stream.push(shown);
                        }
                    } else {
                        turn.show(if answer.is_empty() { "thinking" } else { "answering" }, heard, &answer).await;
                    }
                }
            }
        }
    };
    let (ok, reply) = loop {
        tokio::select! {
            biased;
            Ok(()) = prompts.changed(), if turn.owner.mac => {
                let hint = prompts.borrow_and_update().clone();
                turn.show("approval", &text, &hint).await;
            }
            Some(ev) = erx.recv() => on_event(ev).await,
            r = &mut ask => break r,
        }
    };
    while let Ok(ev) = erx.try_recv() {
        on_event(ev).await;
    }
    let sent = match stream {
        Some(stream) => stream.close().await,
        None => 0,
    };
    if brain.cancelled() || !turn.active() {
        if turn.owner.mac {
            turn.show("error", &text, "The request was cancelled.").await;
        }
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
    if ok && !turn.owner.mac {
        *st.last_reply.lock().unwrap() = Some(json!({"t": crate::hub::now(), "text": reply}));
    }
    turn.show(if ok { "done" } else { "error" }, &text, &reply).await;
    st.history(&with(json!({"via": "claude", "ok": ok, "reply": reply})));
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
            crate::speak::speak_for(&st, &reply, false, false, turn.owner.mac);
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
        make_agent(name, stt_ready, true).await
    }

    async fn make_agent(name: &str, stt_ready: bool, watch_link: bool) -> T {
        let home = std::env::temp_dir().join(format!("stickybot-rs-talk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let opts = Options { port: 8670, home, fake_stick: true, import_from: None };
        let fake = FakeStick::new();
        // App discovery is also a fake: tests must not depend on /Applications or the user's apps.
        let apps = commands::Apps {
            fixed: Some([("slack".into(), "Slack".into()), ("fixture editor".into(), "Fixture Editor".into())].into()),
        };
        let st =
            AppState::new_session(&opts, Backend::Fake(fake.clone()), None, "primary".into(), None, Arc::new(apps))
                .unwrap();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let r = ran.clone();
        st.set_runner(Arc::new(move |a: &Action| {
            r.lock().unwrap().push(a.clone());
            String::new()
        }));
        st.link.spawn();
        if watch_link {
            spawn(st.clone());
        }
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
    async fn assistant_voice_does_not_execute_saved_shortcuts_before_the_brain() {
        let t = agent("assistant-shortcut", true).await;
        t.st.cfg.lock().unwrap().insert("tool_profiles".into(), json!({"claude":"assistant","openrouter":"assistant"}));
        t.st.commands
            .save(
                json!({"commands":[{"name":"Shortcut","phrases":["open docs"],"open_url":"https://example.com"}]})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        for brain in ["claude", "openrouter"] {
            t.st.cfg.lock().unwrap().insert("brain".into(), json!(brain));
            handle_text(t.st.clone(), "open docs".into(), "typed").await;
            assert!(t.ran.lock().unwrap().is_empty());
        }
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
                        Some(c) if c == "PERMISSION" => {
                            let _ = ev.send(BrainEvent::PermissionHook(true));
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

    type DisplayLog = Arc<Mutex<Vec<(String, String, String)>>>;
    fn mac_display() -> (MacDisplay, DisplayLog) {
        let log = DisplayLog::default();
        let events = log.clone();
        (
            Arc::new(move |phase, heard, detail| {
                events.lock().unwrap().push((phase.into(), heard.into(), detail.into()));
            }),
            log,
        )
    }

    #[tokio::test]
    async fn mac_turn_uses_the_same_brain_without_a_stick_or_a_legacy_shortcut() {
        let t = agent("mac-offline", true).await;
        t.st.link.pause(true);
        t.st.link.watch_state().wait_for(|s| *s == LinkState::Paused).await.unwrap();
        t.st.commands
            .save(
                json!({"commands":[{"name":"Docs","phrases":["open docs"],"open_url":"https://example.com"}]})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        *t.said.lock().unwrap() = Ok("open docs".into());
        let answer = "A detailed answer. ".repeat(30);
        let brain = Arc::new(FakeBrain::new(&["TOOL:Searching", answer.as_str()]));
        t.st.set_brain(brain.clone());
        let (display, events) = mac_display();
        MacTurn::begin(t.st.clone(), display).unwrap().submit(vec![0.; RATE as usize]).await;
        assert_eq!(&*brain.asked.lock().unwrap(), &["open docs"]);
        assert!(t.ran.lock().unwrap().is_empty());
        let ev = events.lock().unwrap();
        assert_eq!(
            ev.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
            ["transcribing", "thinking", "thinking", "answering", "reply"]
        );
        assert_eq!(ev.last().unwrap().1, "open docs");
        assert_eq!(ev.last().unwrap().2, answer.trim());
        assert!(t.fake.screens().is_empty());
        assert_eq!(history(&t)[0]["source"], "mac_voice");
    }

    #[tokio::test]
    async fn mac_reply_stays_out_of_the_stick_home_card_after_reconnect() {
        let t = agent("mac-private-home-card", true).await;
        t.st.cfg.lock().unwrap().insert("ticker".into(), json!(false));
        t.st.set_brain(Arc::new(FakeBrain::new(&["Previous stick answer"])));
        assert_eq!(talk(&t, 1.0).await.last().unwrap(), &pair("done", "Previous stick answer"));
        t.st.link.pause(true);
        t.st.link.watch_state().wait_for(|s| *s == LinkState::Paused).await.unwrap();
        t.st.set_brain(Arc::new(FakeBrain::new(&["Private Mac answer"])));
        let (display, events) = mac_display();
        MacTurn::begin(t.st.clone(), display).unwrap().submit(vec![0.; RATE as usize]).await;
        assert_eq!(events.lock().unwrap().last().unwrap().2, "Private Mac answer");
        assert!(t.st.hub.subscribe(30).0.iter().any(|e| e["kind"] == "reply" && e["text"] == "Private Mac answer"));
        t.st.link.pause(false);
        t.st.link.watch_state().wait_for(|s| *s == LinkState::Connected).await.unwrap();
        assert_eq!(crate::home::hello_extra(&t.st)["home"]["b"], "Previous stick answer");
        assert_eq!(*t.st.last_done.lock().unwrap(), "Previous stick answer");
    }

    #[tokio::test]
    async fn stick_inputs_cannot_cancel_a_mac_recording_or_request() {
        let t = agent("mac-own", true).await;
        let mut brain = FakeBrain::new(&["Working", " on it"]);
        brain.pause = Duration::from_millis(100);
        let brain = Arc::new(brain);
        t.st.set_brain(brain.clone());
        let (notify, events) = mac_display();
        let mac = MacTurn::begin(t.st.clone(), notify.clone()).unwrap();
        assert!(MacTurn::begin(t.st.clone(), notify.clone()).is_err());
        t.fake.talk(&frames(1.0));
        assert_eq!(ended(&t).await, vec![pair("error", BUSY)]);
        assert_eq!(t.st.agent_state(), "listening");
        let task = tokio::spawn(mac.submit(vec![0.; RATE as usize]));
        assert!(until(|| !brain.asked.lock().unwrap().is_empty()).await);
        t.fake.notify(&json!({"src":"agent","evt":"cancel"}));
        t.st.link.pause(true);
        t.st.link.watch_state().wait_for(|s| *s == LinkState::Paused).await.unwrap();
        task.await.unwrap();
        assert!(!brain.cancelled());
        assert_eq!(events.lock().unwrap().last().unwrap().0, "reply");
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
        let next = MacTurn::begin(t.st.clone(), notify.clone()).unwrap();
        drop(next); // Escape before key-up must never submit
        assert!(MacTurn::begin(t.st.clone(), notify).is_ok());
        assert_eq!(brain.asked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_mac_panel_cancels_only_its_own_request() {
        let t = agent("mac-cancel", true).await;
        let mut brain = FakeBrain::new(&["Working", " on it"]);
        brain.pause = Duration::from_millis(200);
        let brain = Arc::new(brain);
        t.st.set_brain(brain.clone());
        assert!(!cancel_mac_turn(&t.st)); // nothing running
        let (notify, events) = mac_display();
        let task = tokio::spawn(MacTurn::begin(t.st.clone(), notify.clone()).unwrap().submit(vec![0.; RATE as usize]));
        assert!(until(|| !brain.asked.lock().unwrap().is_empty()).await);
        assert!(cancel_mac_turn(&t.st));
        task.await.unwrap();
        assert!(brain.cancelled());
        assert!(events.lock().unwrap().iter().all(|(phase, _, _)| phase != "reply"));
        // the reservation is free again, and a stick turn is not the panel's to cancel
        assert!(MacTurn::begin(t.st.clone(), notify).is_ok());
        let stick = VoiceTurn::reserve(t.st.clone(), None, t.st.link.generation()).unwrap();
        assert!(!cancel_mac_turn(&t.st));
        assert!(stick.active());
    }

    #[tokio::test]
    async fn a_coalesced_reconnect_discards_old_audio_stop_and_cancel_without_blocking_new_turns() {
        // Drive the talk receiver ourselves so it sees only Connected after both link transitions.
        let t = make_agent("mac-coalesced-link", true, false).await;
        t.st.set_brain(Arc::new(FakeBrain::new(&["New generation answer"])));
        let event = |evt: &str, state: &str| FromStick::Message(json!({"src":"agent","evt":evt,"state":state}));
        let mut rec = None;
        for via_stick in [true, false] {
            let old = t.st.link.generation();
            receive(&t.st, old, event("talk", "start"), &mut rec).await;
            assert!(rec.is_some());
            t.st.link.pause(true);
            t.st.link.watch_state().wait_for(|s| *s == LinkState::Paused).await.unwrap();
            t.st.link.pause(false);
            t.st.link.watch_state().wait_for(|s| *s == LinkState::Connected).await.unwrap();
            let new = t.st.link.generation();
            assert_ne!(old, new);
            if via_stick {
                receive(&t.st, new, event("talk", "start"), &mut rec).await;
                // Outdated events and a delayed watch notification arrive while the new owner records.
                receive(&t.st, old, FromStick::Audio(frames(0.02).remove(0)), &mut rec).await;
                assert!(rec.as_ref().unwrap().0.audio().is_empty());
                receive(&t.st, old, event("talk", "stop"), &mut rec).await;
                receive(&t.st, old, event("cancel", ""), &mut rec).await;
                cancel_stick_turn(&t.st, None);
                assert!(rec.as_ref().unwrap().1.active());
                for frame in frames(1.0) {
                    receive(&t.st, new, FromStick::Audio(frame), &mut rec).await;
                }
                receive(&t.st, new, event("talk", "stop"), &mut rec).await;
                assert_eq!(ended(&t).await.last().unwrap(), &pair("done", "New generation answer"));
                assert!(until(|| t.st.voice_owner.lock().unwrap().is_none()).await);
            } else {
                let (notify, _) = mac_display();
                let mac = MacTurn::begin(t.st.clone(), notify.clone()).unwrap();
                receive(&t.st, old, event("cancel", ""), &mut rec).await;
                cancel_stick_turn(&t.st, None);
                assert!(rec.is_none());
                assert!(MacTurn::begin(t.st.clone(), notify).is_err());
                assert!(mac.0.active());
            }
        }
    }

    #[tokio::test]
    async fn mac_cannot_start_during_a_stick_recording_and_an_empty_mac_turn_finishes() {
        let t = agent("mac-collision", true).await;
        let (notify, events) = mac_display();
        t.fake.notify(&json!({"src":"agent","evt":"talk","state":"start"}));
        assert!(until(|| t.st.agent_state() == "listening").await);
        assert!(MacTurn::begin(t.st.clone(), notify.clone()).is_err());
        t.fake.notify(&json!({"src":"agent","evt":"cancel"}));
        assert!(until(|| t.st.agent_state() == "ready").await);
        MacTurn::begin(t.st.clone(), notify).unwrap().submit(vec![]).await;
        assert_eq!(events.lock().unwrap().last().unwrap().0, "error");
    }

    #[tokio::test]
    async fn keyboard_reply_speech_uses_mac_even_when_stick_output_is_selected() {
        let t = agent("mac-speech-output", true).await;
        let voice = Arc::new(crate::speak::tests::FakeVoice::default());
        *t.st.speaker.voice.lock().unwrap() = Arc::new(voice.clone());
        t.st.cfg.lock().unwrap().insert("speak_replies".into(), json!(true));
        t.st.cfg.lock().unwrap().insert("speak_output".into(), json!("stick"));
        t.st.set_brain(Arc::new(FakeBrain::new(&["A short answer."])));
        let (notify, _) = mac_display();
        MacTurn::begin(t.st.clone(), notify).unwrap().submit(vec![0.; RATE as usize]).await;
        assert!(until(|| !voice.played.lock().unwrap().is_empty()).await);
        assert_eq!(*voice.for_stick.lock().unwrap(), [false]);
        assert!(speech_msgs(&t.fake).is_empty());
    }

    #[tokio::test]
    async fn keyboard_provider_hook_reports_permission_without_claiming_a_stick_is_available() {
        let t = agent("mac-provider-hook", true).await;
        t.st.link.set_target(None);
        t.st.set_brain(Arc::new(FakeBrain::new(&["PERMISSION", "Done"])));
        let (notify, events) = mac_display();
        MacTurn::begin(t.st.clone(), notify).unwrap().submit(vec![0.; RATE as usize]).await;
        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|(phase, _, detail)| phase == "approval" && detail == "Claude is requesting permission.")
        );
        assert_eq!(events.last().unwrap().0, "reply");
    }

    #[tokio::test]
    async fn brain_adapter_keeps_full_mac_text_and_compact_stick_text() {
        let text = format!("**First paragraph**\n\n{}", "下一行。 ".repeat(100));
        let mac = MAC_REPLY.scope(true, async { brain_reply(&text) }).await;
        assert_eq!(mac, text.trim());
        assert_eq!(brain_reply(&text), plain(&text));
        assert!(brain_reply(&text).chars().count() <= 280);
    }

    #[tokio::test]
    async fn stick_open_over_http_keeps_the_source_voice_generation() {
        use axum::{body::Body, http::Request};
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let t = agent("stick-open-voice-generation", false).await;
        let app = crate::api::router(t.st.clone());
        t.st.devices.init(&t.st).unwrap();
        let ask = || {
            let request = Request::post("/api/devices/primary/mcp")
                .header("host", "localhost:8670")
                .header("authorization", format!("Bearer {}", t.st.mcp.local_token))
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"stick_open","arguments":{"app":"air"}}})
                    .to_string(),
                ))
                .unwrap();
            app.clone().oneshot(request)
        };
        let _scope = t.st.mcp.session.begin();
        let live = t.st.link.generation();
        for generation in [None, Some(live.wrapping_sub(1)), Some(live), None] {
            let _turn = generation.map(|g| VoiceTurn::reserve(t.st.clone(), None, g).unwrap());
            t.fake.set_stick("openTile", Value::Null);
            let response = ask().await.unwrap();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let reply: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(reply["result"]["isError"], generation != Some(live), "{reply}");
            assert_eq!(t.fake.stick_state()["openTile"] == "air", generation == Some(live));
        }
    }

    struct OpenTool(std::sync::Weak<AppState>);
    impl Brain for OpenTool {
        fn ask(&self, _text: String, _: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)> {
            let st = self.0.upgrade().unwrap();
            Box::pin(async move {
                let _scope = st.mcp.session.begin();
                let ctx = crate::tool_policy::Context::brain(&st, crate::tool_policy::Caller::Local).unwrap();
                let (answer, error, _) =
                    crate::mcp::call(&st, "open_url", &json!({"url":"https://example.org/docs"}), &ctx).await;
                (!error, answer)
            })
        }
        fn busy(&self) -> bool {
            false
        }
        fn cancel(&self) {}
        fn cancelled(&self) -> bool {
            false
        }
        fn forget(&self) {}
    }

    #[tokio::test]
    async fn mac_tool_waits_for_exact_stick_approval_and_fails_closed_offline() {
        let t = agent("mac-tool-approval", true).await;
        t.st.set_brain(Arc::new(OpenTool(Arc::downgrade(&t.st))));
        let (notify, events) = mac_display();
        let task = tokio::spawn(MacTurn::begin(t.st.clone(), notify.clone()).unwrap().submit(vec![0.; RATE as usize]));
        assert!(
            until(|| events
                .lock()
                .unwrap()
                .iter()
                .any(|(phase, _, hint)| phase == "approval" && hint.contains("https://example.org/docs")))
            .await
        );
        assert!(t.ran.lock().unwrap().is_empty());
        crate::approvals::approve_next(&t.st, "once").await;
        task.await.unwrap();
        {
            let ev = events.lock().unwrap();
            assert_eq!(ev.last().unwrap().0, "reply", "{ev:?}");
        }
        assert_eq!(t.ran.lock().unwrap().len(), 1);
        t.st.link.pause(true);
        t.st.link.watch_state().wait_for(|s| *s == LinkState::Paused).await.unwrap();
        MacTurn::begin(t.st.clone(), notify).unwrap().submit(vec![0.; RATE as usize]).await;
        let events = events.lock().unwrap();
        let (phase, _, detail) = events.last().unwrap();
        assert_eq!(phase, "error");
        assert!(detail.contains("Connect your stick"), "{detail}");
        assert_eq!(t.ran.lock().unwrap().len(), 1);
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

    // --- the reply view (firmware 2.6.0; test_reply_view.py) ----------------------------------------

    const LONG: &str = "## Release notes\n\nHere's what changed this week:\n\n- **Login**: the redirect bug is fixed, the session cookie survives the round trip.\n- Dark mode landed behind the `theme` toggle.\n\n```sh\nnpm run build\n```\n\n";

    fn long() -> String {
        format!(
            "{LONG}{}It's 28°C — nice.",
            "Still open: the billing refactor needs a review, and the CI cache keeps missing. ".repeat(8)
        )
    }

    /// The JSON lines the fake stick got.
    pub fn lines(fake: &FakeStick) -> Vec<Value> {
        String::from_utf8(fake.writes().concat())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    #[test]
    fn reader_text_keeps_paragraphs_and_drops_markdown() {
        let t = reader_text(&long(), READER_MAX);
        assert!(t.starts_with("Release notes\n\nHere's what changed this week:\n\n• Login: the redirect"), "{t}");
        assert!(t.contains("• Dark mode landed behind the theme toggle."));
        assert!(t.contains("npm run build") && !t.contains("```") && !t.contains("**") && !t.contains("##"));
        assert!(!t.contains("\n\n\n"));
        assert!(t.contains("28°C — nice."));
        let capped = reader_text(&"°".repeat(5000), READER_MAX);
        assert!(capped.len() <= READER_MAX && capped.ends_with('…'));
        assert_eq!(reader_text("short", READER_MAX), "short");
    }

    #[test]
    fn reply_parts_fit_and_join_back() {
        let t = reader_text(&long().repeat(3), READER_MAX);
        let parts = reply_parts(&t, PART_BYTES);
        assert_eq!(parts.concat(), t);
        assert!(parts.iter().all(|p| p.len() <= PART_BYTES));
        let dashes = reply_parts(&"—".repeat(200), 10);
        assert_eq!(dashes.len(), 67);
        assert!(dashes[..66].iter().all(|p| p == "———") && dashes[66] == "——");
        assert!(reply_parts("", 10).is_empty());
    }

    #[tokio::test]
    async fn a_long_answer_follows_done_in_parts() {
        let t = agent("reply-parts", false).await;
        let before = lines(&t.fake).len();
        tell_stick(&t.st, "done", "what changed", &long()).await;
        let sent: Vec<Value> = lines(&t.fake).into_iter().skip(before).filter(|m| m.get("hello").is_none()).collect();
        let (done, rest) = sent.split_first().unwrap();
        assert_eq!(done["state"], "done");
        assert_eq!(done["text"].as_str().unwrap().chars().count(), 300);
        assert_eq!(done["rid"], 1);
        assert_eq!(done["parts"].as_u64().unwrap() as usize, rest.len());
        assert!(rest.len() >= 2);
        for (i, m) in rest.iter().enumerate() {
            assert_eq!((m["reply"]["rid"].as_u64(), m["reply"]["i"].as_u64()), (Some(1), Some(i as u64)));
        }
        let joined: String = rest.iter().map(|m| m["reply"]["t"].as_str().unwrap()).collect();
        assert_eq!(joined, reader_text(&long(), READER_MAX));
        assert_eq!(*t.st.last_done.lock().unwrap(), long());
        // short: just "done"
        let before = lines(&t.fake).len();
        tell_stick(&t.st, "done", "weather", "It's 28°C.").await;
        let sent: Vec<Value> = lines(&t.fake).into_iter().skip(before).filter(|m| m.get("hello").is_none()).collect();
        assert_eq!(sent, vec![json!({"src": "agent", "state": "done", "heard": "weather", "text": "It's 28°C."})]);
    }

    fn speech_msgs(fake: &FakeStick) -> Vec<String> {
        lines(fake).iter().filter_map(|m| m.get("speech").and_then(Value::as_str).map(String::from)).collect()
    }

    async fn until(f: impl Fn() -> bool) -> bool {
        for _ in 0..300 {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn listen_reads_the_last_reply_and_says_when_it_ends() {
        let t = agent("listen", false).await;
        let v = Arc::new(crate::speak::tests::FakeVoice::default());
        *t.st.speaker.voice.lock().unwrap() = Arc::new(v.clone());
        tell_stick(&t.st, "done", "weather", "It's 28°C in Bangalore. Rain later. Take an umbrella.").await;
        t.fake.notify(&json!({"src": "agent", "evt": "speak", "what": "last"}));
        assert!(until(|| speech_msgs(&t.fake) == ["on", "off"]).await, "{:?}", speech_msgs(&t.fake));
        assert_eq!(*v.made.lock().unwrap(), ["It's 28°C in Bangalore.", "Rain later.", "Take an umbrella."]);
    }

    #[tokio::test]
    async fn listen_again_stops_it() {
        let t = agent("listen-stop", false).await;
        let v = Arc::new(crate::speak::tests::FakeVoice { play_ms: 5000, ..Default::default() });
        *t.st.speaker.voice.lock().unwrap() = Arc::new(v.clone());
        tell_stick(&t.st, "done", "x", "One. Two.").await;
        t.fake.notify(&json!({"src": "agent", "evt": "speak", "what": "last"}));
        assert!(until(|| !v.played.lock().unwrap().is_empty()).await);
        t.fake.notify(&json!({"src": "agent", "evt": "speak", "what": "stop"}));
        assert!(until(|| speech_msgs(&t.fake) == ["on", "off"]).await, "{:?}", speech_msgs(&t.fake));
        assert!(!t.st.speaker.speaking());
    }

    #[tokio::test]
    async fn listen_without_a_reply_says_off() {
        let t = agent("listen-none", false).await;
        t.fake.notify(&json!({"src": "agent", "evt": "speak", "what": "last"}));
        assert!(until(|| speech_msgs(&t.fake) == ["off"]).await, "{:?}", speech_msgs(&t.fake));
    }

    #[tokio::test]
    async fn listen_reads_a_long_reply_whole() {
        let t = agent("listen-long", false).await;
        let v = Arc::new(crate::speak::tests::FakeVoice::default());
        *t.st.speaker.voice.lock().unwrap() = Arc::new(v.clone());
        t.st.cfg.lock().unwrap().insert("speak_max_words".into(), json!(5));
        tell_stick(&t.st, "done", "x", &long()).await;
        t.fake.notify(&json!({"src": "agent", "evt": "speak", "what": "last"}));
        assert!(until(|| speech_msgs(&t.fake) == ["on", "off"]).await);
        let made = v.made.lock().unwrap().clone();
        assert!(made.iter().map(|s| s.split_whitespace().count()).sum::<usize>() > 100);
        assert!(!made.iter().any(|s| s.contains("npm run build")));
    }

    #[tokio::test]
    async fn quitting_says_bye() {
        let t = agent("bye-quit", false).await;
        assert!(t.st.bye_blocking(Duration::from_secs(1))); // on the runtime: started, not waited for
        assert!(until(|| lines(&t.fake).contains(&json!({"src": "agent", "bye": true}))).await);
    }
}

#[cfg(test)]
mod session_generation_tests {
    use super::*;
    #[tokio::test]
    async fn work_queued_before_disconnect_cannot_be_rebound_to_reconnected_session() {
        let t = super::tests::agent("queued-generation", false).await;
        let generation = t.st.link.generation();
        let text = handle_text(t.st.clone(), "This request belongs to the old session".into(), "typed");
        let recording = handle_talk_at(t.st.clone(), Recording::new(), generation);
        t.st.link.pause(true);
        let mut states = t.st.link.watch_state();
        states.wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
        t.st.link.pause(false);
        states.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        assert_ne!(t.st.link.generation(), generation);
        text.await;
        recording.await;
        assert_eq!(t.st.status()["stats"]["requests"], 0);
        assert!(!t.fake.screens().iter().any(|(_, heard, _)| heard.contains("old session")));
    }
}
