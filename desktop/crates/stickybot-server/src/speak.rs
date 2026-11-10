//! Reading replies aloud, sentence by sentence while Claude is still writing (a port of
//! `speech.py`; not `speech.rs`, which is speech *to text*).
//!
//! Each sentence is made with macOS `say` (offline voices) as soon as it's complete; the next one
//! is made while the current one plays, so speech is continuous. Output is the stick's speaker
//! (clips over Bluetooth) or the Mac's speakers (config `speak_output`; the Mac when the stick
//! isn't connected). The stick plays one clip at a time and a new clip's start packet stops the
//! one playing, so the next clip is only sent once the current one has finished; everything made
//! by then goes in that one clip (fewer gaps).

use crate::AppState;
use futures_util::future::BoxFuture;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Silence between sentences joined into one clip.
pub const GAP: Duration = Duration::from_millis(120);
/// The stick starts playing a little after the clip's last packet.
pub const PLAY_MARGIN: Duration = Duration::from_millis(150);
/// The stick's clip buffer.
pub const MAX_CLIP_S: usize = 30;

static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*_`#>|]+").unwrap());
static BULLETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*[-•][ \t]+").unwrap());
static ABBREV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\b(?:Mr|Mrs|Ms|Dr|St|vs|approx|e\.g|i\.e)|(?:^|\s)[A-Z])\.$").unwrap());
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w").unwrap());

/// Text for the voice: no code blocks (finished or still open), no markdown symbols.
pub fn speakable(text: &str) -> String {
    let t = CODE.replace_all(text, "\n");
    let t = t.split("```").next().unwrap_or("");
    let t = MARKS.replace_all(t, "");
    BULLETS.replace_all(&t, "").into_owned()
}

/// The sentence boundaries in `s` from byte `pos` on, as `(start, end, newline)`: `. ! ? …` (plus
/// closing quotes / brackets) followed by space, or a line break (speech.py `_BOUNDARY`).
fn boundaries(s: &str, pos: usize) -> Vec<(usize, usize, bool)> {
    const ENDS: &[char] = &['.', '!', '?', '…'];
    const CLOSERS: &[char] = &['"', '\'', '”', '’', ')', ']'];
    let mut out = Vec::new();
    let mut i = pos;
    while i < s.len() {
        let after_end = s[..i].chars().next_back().is_some_and(|c| ENDS.contains(&c));
        if after_end {
            let rest = &s[i..];
            let closers: usize = rest.chars().take_while(|c| CLOSERS.contains(c)).map(char::len_utf8).sum();
            let ws: usize = rest[closers..].chars().take_while(|c| c.is_whitespace()).map(char::len_utf8).sum();
            if ws > 0 {
                out.push((i, i + closers + ws, false));
                i += closers + ws;
                continue;
            }
        }
        if s[i..].starts_with('\n') {
            let n = s[i..].chars().take_while(|c| *c == '\n').count();
            out.push((i, i + n, true));
            i += n;
            continue;
        }
        i += s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    out
}

/// Cuts a reply that grows (or restarts) into complete sentences, each one once. `feed(text)`
/// takes the whole reply so far and returns the sentences completed since the last call; a
/// sentence is complete once the next one has started (so a short preamble that turns into a
/// tool call is never spoken). `finish()` returns the rest.
#[derive(Debug, Default)]
pub struct Sentences {
    pub text: String,
    said: String,
}

impl Sentences {
    pub fn feed(&mut self, text: &str) -> Vec<String> {
        if !text.starts_with(&self.text) {
            self.said.clear(); // a new message ("" = the text so far was a preamble)
        }
        self.text = text.to_string();
        self.cut(false)
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.cut(true)
    }

    fn cut(&mut self, last: bool) -> Vec<String> {
        let s = speakable(&self.text);
        let mut pos = if s.starts_with(&self.said) { self.said.len() } else { self.said.len().min(s.len()) };
        while !s.is_char_boundary(pos) {
            pos -= 1;
        }
        let mut out = Vec::new();
        for (start, end, newline) in boundaries(&s, pos) {
            if start < pos {
                continue;
            }
            let piece = &s[pos..start];
            if ABBREV.is_match(piece) && !newline {
                continue; // "Dr. Smith", "e.g. this": not the end
            }
            if end >= s.len() {
                break; // nothing after the boundary yet: the next sentence hasn't started
            }
            out.push(piece.to_string());
            pos = end;
        }
        if last {
            out.push(s[pos..].to_string());
            pos = s.len();
        }
        self.said = s[..pos].to_string();
        out.into_iter()
            .filter(|p| !p.trim().is_empty() && WORD.is_match(p))
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }
}

/// A sentence made into audio: 16 kHz samples for the stick, or a file for the Mac's speakers.
#[derive(Debug, Clone)]
pub struct Clip {
    pub pcm: Vec<i16>,
    pub path: Option<PathBuf>,
    pub text: String,
}

impl Drop for Clip {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Making and playing speech (tests swap it).
pub trait Voice: Send + Sync {
    /// `say` -> a clip (for the stick when `for_stick`, else a file for the Mac).
    fn synth(&self, sentence: String, voice: String, for_stick: bool) -> BoxFuture<'static, Result<Clip, String>>;
    /// Plays `batch` (one clip on the stick, or one after another on the Mac).
    fn play(&self, st: Arc<AppState>, batch: Vec<Clip>, voice: String) -> BoxFuture<'static, ()>;
}

/// macOS `say` / `afplay` (killed when the speech is stopped: `kill_on_drop`).
pub struct SystemVoice;

fn say_args(voice: &str) -> Vec<String> {
    if voice.is_empty() { Vec::new() } else { vec!["-v".into(), voice.into()] }
}

impl Voice for SystemVoice {
    fn synth(&self, sentence: String, voice: String, for_stick: bool) -> BoxFuture<'static, Result<Clip, String>> {
        Box::pin(async move {
            let mut b = [0u8; 6];
            let _ = getrandom::fill(&mut b);
            let path = std::env::temp_dir()
                .join(format!("stickybot-say-{}.aiff", b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
            let clip = Clip { pcm: Vec::new(), path: Some(path.clone()), text: sentence.clone() };
            let status = tokio::process::Command::new("say")
                .args(say_args(&voice))
                .arg("-o")
                .arg(&path)
                .arg(&sentence)
                .kill_on_drop(true)
                .status()
                .await
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("say failed ({status})"));
            }
            if !for_stick {
                return Ok(clip);
            }
            let p = path.clone();
            let pcm = tokio::task::spawn_blocking(move || crate::clips::load(&p, true))
                .await
                .map_err(|e| e.to_string())??;
            let mut clip = clip;
            clip.pcm = pcm.into_iter().take(crate::clips::RATE as usize * MAX_CLIP_S).collect();
            if let Some(p) = clip.path.take() {
                let _ = std::fs::remove_file(p);
            }
            Ok(clip)
        })
    }

    fn play(&self, st: Arc<AppState>, batch: Vec<Clip>, voice: String) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let files = batch.iter().any(|c| c.pcm.is_empty());
            if files || !st.link.connected() {
                for c in &batch {
                    let mut cmd = match &c.path {
                        Some(p) if c.pcm.is_empty() => {
                            let mut a = tokio::process::Command::new("afplay");
                            a.arg(p);
                            a
                        }
                        _ => {
                            // made for the stick, but it's gone: say it here
                            let mut s = tokio::process::Command::new("say");
                            s.args(say_args(&voice)).arg(&c.text);
                            s
                        }
                    };
                    let _ = cmd.kill_on_drop(true).status().await;
                }
                return;
            }
            play_on_stick(&st, &batch).await;
        })
    }
}

/// The batch as one clip on the stick (a short gap between sentences), then waits for it to play.
pub async fn play_on_stick(st: &Arc<AppState>, batch: &[Clip]) -> usize {
    let gap = vec![0i16; (crate::clips::RATE as f64 * GAP.as_secs_f64()) as usize];
    let mut pcm = Vec::new();
    for (i, c) in batch.iter().enumerate() {
        if i > 0 {
            pcm.extend_from_slice(&gap);
        }
        pcm.extend_from_slice(&c.pcm);
    }
    pcm.truncate(crate::clips::RATE as usize * MAX_CLIP_S);
    let t = std::time::Instant::now();
    st.link.send_audio(&crate::clips::packets(&pcm)).await;
    let secs = pcm.len() as f64 / crate::clips::RATE as f64;
    log::info!(
        "spoke {secs:.1} s on the stick ({} sentence{}, sent in {:.2} s)",
        batch.len(),
        if batch.len() == 1 { "" } else { "s" },
        t.elapsed().as_secs_f64()
    );
    tokio::time::sleep(Duration::from_secs_f64(secs) + PLAY_MARGIN).await; // don't cut it off
    pcm.len()
}

/// One reply being spoken: `add()` sentences, `close()` when there are no more.
pub struct Utterance {
    max_words: Option<usize>,
    state: Mutex<UState>,
    cancelled: AtomicBool,
    tx: mpsc::UnboundedSender<Option<String>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// [`Utterance::wait`] saw it end (its tasks are taken then)
    over: AtomicBool,
    ended: tokio::sync::Notify,
}

#[derive(Default)]
struct UState {
    words: usize,
    capped: bool,
    spoken: Vec<String>,
}

impl Utterance {
    fn new(st: Arc<AppState>, voice: Arc<dyn Voice>, max_words: Option<usize>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<String>>();
        let (ready_tx, mut ready_rx) = mpsc::unbounded_channel::<Option<Clip>>();
        let name = || st.cfg.lock().unwrap().get("speak_voice").and_then(Value::as_str).unwrap_or("").to_string();
        let for_stick = output(&st) == "stick";
        let (v1, v2, voice_name, st2) = (voice.clone(), voice, name(), st.clone());
        let vn2 = voice_name.clone();
        let make = tokio::spawn(async move {
            while let Some(Some(sentence)) = rx.recv().await {
                match v1.synth(sentence.clone(), voice_name.clone(), for_stick).await {
                    Ok(clip) => {
                        let _ = ready_tx.send(Some(clip));
                    }
                    Err(e) => log::warn!("speech failed: {e}"), // `say` missing, a voice that's gone…
                }
            }
            let _ = ready_tx.send(None);
        });
        let play = tokio::spawn(async move {
            let mut done = false;
            let mut carry: Option<Clip> = None; // made, but it didn't fit in the last clip
            while !done {
                let clip = match carry.take() {
                    Some(c) => c,
                    None => match ready_rx.recv().await {
                        Some(Some(c)) => c,
                        _ => return,
                    },
                };
                let mut secs = clip_seconds(&clip);
                let mut batch = vec![clip];
                while let Ok(more) = ready_rx.try_recv() {
                    match more {
                        // everything already made goes in one go, as long as the stick's buffer holds it
                        Some(c) if secs + GAP.as_secs_f64() + clip_seconds(&c) > MAX_CLIP_S as f64 => {
                            carry = Some(c);
                            break;
                        }
                        Some(c) => {
                            secs += GAP.as_secs_f64() + clip_seconds(&c);
                            batch.push(c);
                        }
                        None => {
                            done = true;
                            break;
                        }
                    }
                }
                v2.play(st2.clone(), batch, vn2.clone()).await;
            }
        });
        Arc::new(Self {
            max_words,
            state: Mutex::default(),
            cancelled: AtomicBool::new(false),
            tx,
            tasks: Mutex::new(vec![make, play]),
            over: AtomicBool::new(false),
            ended: tokio::sync::Notify::new(),
        })
    }

    /// Queues a sentence; false once cancelled, capped (`max_words`), or empty.
    pub fn add(&self, sentence: &str) -> bool {
        if self.cancelled() || sentence.trim().is_empty() {
            return false;
        }
        let mut s = self.state.lock().unwrap();
        if s.capped {
            return false;
        }
        let n = sentence.split_whitespace().count();
        if self.max_words.is_some_and(|m| s.words + n > m) {
            s.capped = true;
            return false;
        }
        s.words += n;
        s.spoken.push(sentence.to_string());
        self.tx.send(Some(sentence.to_string())).is_ok()
    }

    pub fn close(&self) {
        let _ = self.tx.send(None);
    }

    pub fn capped(&self) -> bool {
        self.state.lock().unwrap().capped
    }

    pub fn spoken(&self) -> Vec<String> {
        self.state.lock().unwrap().spoken.clone()
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Stops making and playing (the `say` / `afplay` processes are killed).
    pub fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::Relaxed) {
            return;
        }
        for t in self.tasks.lock().unwrap().iter() {
            t.abort();
        }
    }

    /// Until everything was made and played (or it was cancelled).
    pub async fn wait(&self) {
        let tasks: Vec<_> = std::mem::take(&mut *self.tasks.lock().unwrap());
        for t in tasks {
            let _ = t.await;
        }
        self.ended.notify_waiters();
        self.over.store(true, Ordering::Relaxed);
    }

    /// Until it has ended (played or cancelled), without taking the tasks from [`Utterance::wait`].
    async fn wait_ended(&self) {
        loop {
            if self.over.load(Ordering::Relaxed) || self.cancelled() {
                return;
            }
            let finished = {
                let t = self.tasks.lock().unwrap();
                !t.is_empty() && t.iter().all(|h| h.is_finished())
            };
            if finished {
                return;
            }
            tokio::select! {
                _ = self.ended.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }
}

/// How long a clip made for the stick plays (0 for a file for the Mac's speakers).
pub fn clip_seconds(c: &Clip) -> f64 {
    c.pcm.len() as f64 / crate::clips::RATE as f64
}

/// The speaker's state (in [`AppState`]).
pub struct Speaker {
    pub voice: Mutex<Arc<dyn Voice>>,
    current: Mutex<Option<Arc<Utterance>>>,
    /// speech is on (told to the stick: `{"src":"agent","speech":"on"|"off"}`, firmware 2.6.0)
    speaking: AtomicBool,
}

impl Default for Speaker {
    fn default() -> Self {
        Self { voice: Mutex::new(Arc::new(SystemVoice)), current: Mutex::new(None), speaking: AtomicBool::new(false) }
    }
}

impl Speaker {
    pub fn speaking(&self) -> bool {
        self.speaking.load(Ordering::Relaxed)
    }
}

/// Speech started / ended: the stick's reply view shows "Reading aloud".
fn changed(st: &AppState, on: bool) {
    if st.speaker.speaking.swap(on, Ordering::Relaxed) == on {
        return;
    }
    st.hub.emit("speech", json!({"on": on}));
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let link = st.link.clone();
    tokio::spawn(async move {
        link.send(&json!({"src": "agent", "speech": if on { "on" } else { "off" }})).await;
    });
}

/// Where speech goes right now: "stick" (its speaker) or "mac".
pub fn output(st: &AppState) -> &'static str {
    let on_stick = st.cfg.lock().unwrap().get("speak_output").and_then(Value::as_str).unwrap_or("stick") == "stick";
    if on_stick && st.link.connected() { "stick" } else { "mac" }
}

/// A new reply to speak (stops the one before; no "off" in between).
pub fn start(st: &Arc<AppState>, max_words: Option<usize>) -> Arc<Utterance> {
    if let Some(old) = st.speaker.current.lock().unwrap().take() {
        old.cancel();
    }
    let voice = st.speaker.voice.lock().unwrap().clone();
    let u = Utterance::new(st.clone(), voice, max_words);
    *st.speaker.current.lock().unwrap() = Some(u.clone());
    changed(st, true);
    // ended (all of it played, or nothing to say): "off", unless another one took its place
    let (st2, u2) = (Arc::downgrade(st), u.clone());
    tokio::spawn(async move {
        u2.wait_ended().await;
        let Some(st) = st2.upgrade() else { return };
        let mut cur = st.speaker.current.lock().unwrap();
        if cur.as_ref().is_some_and(|c| Arc::ptr_eq(c, &u2)) {
            *cur = None;
            drop(cur);
            changed(&st, false);
        }
    });
    u
}

pub fn stop(st: &AppState) {
    let had = st.speaker.current.lock().unwrap().take();
    if let Some(u) = had {
        u.cancel();
    }
    changed(st, false);
}

/// The stick's "listen" (its reply view, `{"evt":"speak","what":"last"}`): the last answer read
/// aloud whole, whatever the speak settings say.
pub fn speak_last(st: &Arc<AppState>) -> bool {
    let mut text = st.last_done.lock().unwrap().clone();
    if text.is_empty() {
        text = st
            .last_reply
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    }
    if speakable(&text).trim().is_empty() {
        log::info!("reading aloud: nothing to read");
        st.speaker.speaking.store(true, Ordering::Relaxed); // so the stick hears "off"
        changed(st, false);
        return false;
    }
    log::info!("reading aloud: the last reply ({} words)", text.split_whitespace().count());
    speak(st, &text, false, true)
}

fn max_words(cfg: &Map<String, Value>) -> usize {
    cfg.get("speak_max_words").and_then(Value::as_u64).filter(|n| *n > 0).unwrap_or(40) as usize
}

/// Speak the reply sentence by sentence while it's written (`speak_replies` + `speak_stream`).
pub fn stream_speech(cfg: &Map<String, Value>) -> Option<usize> {
    let on = cfg.get("speak_replies").and_then(Value::as_bool).unwrap_or(false)
        && cfg.get("speak_stream").and_then(Value::as_bool).unwrap_or(true);
    on.then(|| max_words(cfg))
}

/// core.py `should_speak`: replies aloud on, quick-command confirmations only with `speak_quick`,
/// and short ones only.
pub fn should_speak(cfg: &Map<String, Value>, text: &str, quick: bool) -> bool {
    if text.is_empty() || !cfg.get("speak_replies").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    if quick && !cfg.get("speak_quick").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    text.split_whitespace().count() <= max_words(cfg)
}

/// Reads a text aloud if it should be (or `force`); returns at once, the speech goes on in the
/// background (core.py `speak`).
pub async fn say(st: &Arc<AppState>, text: &str, force: bool) -> bool {
    speak(st, text, false, force)
}

pub fn speak(st: &Arc<AppState>, text: &str, quick: bool, force: bool) -> bool {
    let cfg = st.cfg.lock().unwrap().clone();
    if !force && !should_speak(&cfg, text, quick) {
        return false;
    }
    // sentence by sentence: a long text plays in clips the stick can hold, the first one soon
    let u = start(st, None);
    let mut cut = Sentences::default();
    let mut sentences = cut.feed(text);
    sentences.extend(cut.finish());
    for s in sentences {
        u.add(&s);
    }
    u.close();
    st.hub.emit("spoken", json!({"text": text}));
    true
}

/// Installed macOS voices `[{name, lang}]` from `say -v ?` output.
pub fn parse_voices(out: &str) -> Vec<Value> {
    out.lines()
        .filter_map(|line| {
            let head = line.split('#').next().unwrap_or("").trim_end();
            let (name, lang) = head.rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            (!name.is_empty() && !lang.is_empty()).then(|| json!({"name": name, "lang": lang}))
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn grow(text: &str, step: usize) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        (1..)
            .map(|i| i * step)
            .take_while(|&n| n < chars.len() + step)
            .map(|n| chars[..n.min(chars.len())].iter().collect())
            .collect()
    }

    fn cut(text: &str, step: usize) -> (Vec<String>, Vec<String>) {
        let mut s = Sentences::default();
        let mut out = Vec::new();
        for part in grow(text, step) {
            out.extend(s.feed(&part));
        }
        (out, s.finish())
    }

    // --- test_speech.py --------------------------------------------------------------------------

    #[test]
    fn sentences_as_they_complete() {
        for step in [1, 3, 50] {
            let (done, rest) = cut("It's 28°C in Bengaluru. Sunny all day! Want more? Bring a hat.", step);
            assert_eq!(done, ["It's 28°C in Bengaluru.", "Sunny all day!", "Want more?"], "{step}");
            assert_eq!(rest, ["Bring a hat."]);
        }
    }

    #[test]
    fn a_sentence_waits_for_the_next_one() {
        let mut s = Sentences::default();
        assert!(s.feed("Let me check the weather.").is_empty());
        assert_eq!(s.feed("Let me check the weather. It"), ["Let me check the weather."]);
    }

    #[test]
    fn no_split_inside_numbers_or_abbreviations() {
        let (done, rest) = cut("It costs 3.50 dollars, e.g. at Dr. Smith's shop. Version 2.1.3 is out. OK", 3);
        assert_eq!(done, ["It costs 3.50 dollars, e.g. at Dr. Smith's shop.", "Version 2.1.3 is out."]);
        assert_eq!(rest, ["OK"]);
    }

    #[test]
    fn restart_when_the_text_is_withdrawn() {
        let mut s = Sentences::default();
        s.feed("Let me look. ");
        assert!(s.feed("").is_empty());
        assert_eq!(s.feed("It's sunny. And"), ["It's sunny."]);
        assert_eq!(s.finish(), ["And"]);
    }

    #[test]
    fn markdown_and_code_are_not_spoken() {
        let (done, rest) = cut("**Done.** Here's the command:\n```sh\nrm -rf x.\n```\n- First item\n- Second one", 3);
        assert_eq!(done, ["Done.", "Here's the command:", "First item"]);
        assert_eq!(rest, ["Second one"]);
        assert_eq!(speakable("Text ```still open. more"), "Text ");
    }

    #[test]
    fn lines_are_boundaries() {
        let (done, rest) = cut("Three PRs need you\nPR 12 from Ana\nPR 13", 3);
        assert_eq!(done, ["Three PRs need you", "PR 12 from Ana"]);
        assert_eq!(rest, ["PR 13"]);
    }

    #[test]
    fn voices_list() {
        let out = "Albert              en_US    # Hello! My name is Albert.\nAmélie              fr_CA    # Bonjour\nGrandma (English (UK)) en_GB    # Hello\n";
        let v = parse_voices(out);
        assert_eq!(v[0], json!({"name": "Albert", "lang": "en_US"}));
        assert_eq!(v[1]["name"], "Amélie");
        assert_eq!(v[2], json!({"name": "Grandma (English (UK))", "lang": "en_GB"}));
    }

    // --- the queue (a fake voice, the fake stick) -------------------------------------------------

    #[derive(Default)]
    pub(crate) struct FakeVoice {
        pub synth_ms: u64,
        pub play_ms: u64,
        pub made: Mutex<Vec<String>>,
        pub played: Mutex<Vec<Vec<String>>>,
        pub fail: Option<String>,
    }

    impl Voice for Arc<FakeVoice> {
        fn synth(
            &self,
            sentence: String,
            _voice: String,
            _for_stick: bool,
        ) -> BoxFuture<'static, Result<Clip, String>> {
            let me = self.clone();
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(me.synth_ms)).await;
                if me.fail.as_deref() == Some(sentence.as_str()) {
                    return Err("no such voice".into());
                }
                me.made.lock().unwrap().push(sentence.clone());
                Ok(Clip { pcm: vec![0; 160], path: None, text: sentence })
            })
        }
        fn play(&self, _st: Arc<AppState>, batch: Vec<Clip>, _voice: String) -> BoxFuture<'static, ()> {
            let me = self.clone();
            Box::pin(async move {
                me.played.lock().unwrap().push(batch.iter().map(|c| c.text.clone()).collect());
                tokio::time::sleep(Duration::from_millis(me.play_ms)).await;
            })
        }
    }

    async fn rig(name: &str, v: FakeVoice) -> (crate::talk::tests::T, Arc<FakeVoice>) {
        let t = crate::talk::tests::agent(name, false).await;
        let v = Arc::new(v);
        *t.st.speaker.voice.lock().unwrap() = Arc::new(v.clone());
        (t, v)
    }

    #[tokio::test]
    async fn plays_in_order_and_batches_what_is_ready() {
        let (t, v) = rig("sp-order", FakeVoice { synth_ms: 10, play_ms: 150, ..Default::default() }).await;
        let u = start(&t.st, None);
        for s in ["One.", "Two.", "Three.", "Four."] {
            u.add(s);
        }
        u.close();
        tokio::time::timeout(Duration::from_secs(3), u.wait()).await.unwrap();
        assert_eq!(*v.made.lock().unwrap(), ["One.", "Two.", "Three.", "Four."]);
        let played = v.played.lock().unwrap().clone();
        assert_eq!(played.concat(), ["One.", "Two.", "Three.", "Four."]);
        assert_eq!(played[0], ["One."]); // the first one alone: speech starts at once
        assert!(played.len() < 4); // the rest joined while it played
    }

    #[tokio::test]
    async fn word_cap_stops_at_a_sentence() {
        let (t, v) = rig("sp-cap", FakeVoice::default()).await;
        let u = start(&t.st, Some(5));
        assert!(u.add("One two three."));
        assert!(!u.add("Four five six."));
        assert!(!u.add("Seven.") && u.capped());
        u.close();
        u.wait().await;
        assert_eq!(*v.made.lock().unwrap(), ["One two three."]);
    }

    #[tokio::test]
    async fn cancel_stops_making_and_playing_and_a_new_reply_stops_the_old_one() {
        let (t, v) = rig("sp-cancel", FakeVoice { synth_ms: 50, play_ms: 300, ..Default::default() }).await;
        let u = start(&t.st, None);
        u.add("One.");
        u.add("Two.");
        tokio::time::sleep(Duration::from_millis(100)).await; // "One." is playing
        stop(&t.st);
        assert!(!u.add("Three."));
        tokio::time::timeout(Duration::from_secs(1), u.wait()).await.unwrap();
        assert!(u.cancelled());
        assert_eq!(*v.played.lock().unwrap(), [vec!["One.".to_string()]]);
        assert!(!v.made.lock().unwrap().contains(&"Three.".to_string()));

        let old = start(&t.st, None);
        old.add("Old.");
        tokio::time::sleep(Duration::from_millis(80)).await;
        t.st.cfg.lock().unwrap().insert("speak_replies".into(), json!(true));
        assert!(say(&t.st, "New one.", false).await);
        let new = t.st.speaker.current.lock().unwrap().clone().unwrap();
        tokio::time::timeout(Duration::from_secs(2), new.wait()).await.unwrap();
        assert!(old.cancelled());
        assert_eq!(v.played.lock().unwrap().last().unwrap(), &["New one."]);
    }

    #[tokio::test]
    async fn say_failure_skips_the_sentence() {
        let (t, v) = rig("sp-fail", FakeVoice { fail: Some("Bad.".into()), ..Default::default() }).await;
        let u = start(&t.st, None);
        for s in ["Good.", "Bad.", "Fine."] {
            u.add(s);
        }
        u.close();
        tokio::time::timeout(Duration::from_secs(2), u.wait()).await.unwrap();
        assert_eq!(v.played.lock().unwrap().concat(), ["Good.", "Fine."]);
    }

    #[tokio::test]
    async fn should_speak_rules() {
        let mut cfg = crate::config::defaults();
        assert!(!should_speak(&cfg, "Hi.", false));
        cfg.insert("speak_replies".into(), json!(true));
        assert!(should_speak(&cfg, "Hi.", false));
        assert!(!should_speak(&cfg, "Opened Slack.", true));
        cfg.insert("speak_quick".into(), json!(true));
        assert!(should_speak(&cfg, "Opened Slack.", true));
        cfg.insert("speak_max_words".into(), json!(2));
        assert!(!should_speak(&cfg, "one two three", false));
        assert_eq!(stream_speech(&cfg), Some(2));
        cfg.insert("speak_stream".into(), json!(false));
        assert_eq!(stream_speech(&cfg), None);
    }

    #[tokio::test]
    async fn clips_are_joined_only_up_to_the_sticks_buffer() {
        struct Long(Arc<FakeVoice>);
        impl Voice for Long {
            fn synth(&self, s: String, _v: String, _f: bool) -> BoxFuture<'static, Result<Clip, String>> {
                Box::pin(async move { Ok(Clip { pcm: vec![0; 16000 * 12], path: None, text: s }) }) // 12 s each
            }
            fn play(&self, st: Arc<AppState>, batch: Vec<Clip>, voice: String) -> BoxFuture<'static, ()> {
                self.0.play(st, batch, voice)
            }
        }
        let (t, v) = rig("sp-buffer", FakeVoice { play_ms: 50, ..Default::default() }).await;
        *t.st.speaker.voice.lock().unwrap() = Arc::new(Long(v.clone()));
        let u = start(&t.st, None);
        for s in ["A", "B", "C", "D", "E", "F"] {
            u.add(s);
        }
        u.close();
        tokio::time::timeout(Duration::from_secs(3), u.wait()).await.unwrap();
        let played = v.played.lock().unwrap().clone();
        assert_eq!(played.concat(), ["A", "B", "C", "D", "E", "F"]); // nothing lost or cut
        assert!(played.iter().all(|b| b.len() <= 2), "{played:?}"); // 2 x 12 s fit in 30 s, 3 don't
    }

    #[tokio::test]
    async fn speech_on_and_off_reach_the_stick_once_each() {
        let (t, _v) = rig("sp-onoff", FakeVoice { play_ms: 50, ..Default::default() }).await;
        let msgs = || -> Vec<String> {
            crate::talk::tests::lines(&t.fake)
                .iter()
                .filter_map(|m| m.get("speech").and_then(Value::as_str).map(String::from))
                .collect()
        };
        t.st.cfg.lock().unwrap().insert("speak_replies".into(), json!(true));
        assert!(say(&t.st, "Old one.", false).await);
        assert!(say(&t.st, "New one.", false).await); // replaces it: no "off" in between
        for _ in 0..200 {
            if msgs().len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(msgs(), ["on", "off"]);
        stop(&t.st); // nothing playing: nothing more to say
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(msgs(), ["on", "off"]);
    }

    #[tokio::test]
    async fn stick_playback_joins_clips_and_waits_for_them() {
        let t = crate::talk::tests::agent("sp-stick", false).await;
        let a = Clip { pcm: vec![1; 1600], path: None, text: "A.".into() };
        let b = Clip { pcm: vec![1; 3200], path: None, text: "B.".into() };
        let started = tokio::time::Instant::now();
        let n = play_on_stick(&t.st, &[a, b]).await;
        assert_eq!(n, 1600 + 3200 + (16000.0 * GAP.as_secs_f64()) as usize);
        let clips = t.fake.clips();
        assert_eq!(clips.len(), 1); // one clip for both
        assert_eq!(u32::from_le_bytes(clips[0][0][1..5].try_into().unwrap()) as usize, n);
        assert!(started.elapsed() >= Duration::from_secs_f64(n as f64 / 16000.0) + PLAY_MARGIN);
    }
}
