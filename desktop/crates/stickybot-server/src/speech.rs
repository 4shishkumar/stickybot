//! Speech to text on this Mac (stt.py's job): the model download, and one worker thread that loads
//! the engine once and transcribes each push-to-talk.
//!
//! Model: NVIDIA Parakeet TDT 0.6b v3, int8 ONNX export from sherpa-onnx's `asr-models` release
//! (the Phase-0 spike's pick, `spikes/STT.md`), 487 MB to download, 670 MB unpacked, in
//! `~/.stickybot-rs/models/`. The archive is checked against its SHA-256 before it's unpacked.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use stt::Stt;
use tokio::sync::oneshot;

pub const MODEL_NAME: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
pub const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2";
/// GitHub's digest for the asset (re-uploaded 2026-11-07; the Phase-0 spike's copy from before
/// differs). If k2-fsa replaces it again the check fails and the download is refused: update both.
pub const MODEL_SHA256: &str = "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9";
pub const MODEL_BYTES: u64 = 487_211_443;
/// The files the engine needs.
pub const MODEL_FILES: &[&str] = &["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"];
/// Words the model should prefer when unsure (config `stt_hints`).
pub const DEFAULT_HINTS: &[&str] = &["Tauri", "StickyBot", "Claude"];

/// Where the model is, and how to get it.
#[derive(Debug, Clone)]
pub struct ModelSource {
    pub url: String,
    pub sha256: String,
    /// the folder inside the archive (and in `models/`)
    pub name: String,
}

impl Default for ModelSource {
    fn default() -> Self {
        Self { url: MODEL_URL.into(), sha256: MODEL_SHA256.into(), name: MODEL_NAME.into() }
    }
}

/// What the speech model is doing (`GET /api/speech`, `speech` events).
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// nothing yet
    Idle,
    Downloading {
        done: u64,
        total: u64,
    },
    Checking,
    Unpacking,
    Loading,
    Ready {
        engine: String,
        load_s: f64,
    },
    Failed(String),
}

impl Phase {
    pub fn to_json(&self) -> Value {
        match self {
            Phase::Idle => json!({"phase": "idle"}),
            Phase::Downloading { done, total } => json!({
                "phase": "downloading", "done": done, "total": total,
                "pct": if *total > 0 { (*done as f64 * 100.0 / *total as f64).floor() } else { 0.0 },
            }),
            Phase::Checking => json!({"phase": "checking"}),
            Phase::Unpacking => json!({"phase": "unpacking"}),
            Phase::Loading => json!({"phase": "loading"}),
            Phase::Ready { engine, load_s } => json!({"phase": "ready", "engine": engine, "load_s": load_s}),
            Phase::Failed(e) => json!({"phase": "failed", "error": e}),
        }
    }
}

pub fn model_ready(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| dir.join(f).is_file())
}

/// Downloads (resuming a `.part`), checks and unpacks the model into `models/<name>/` unless it's
/// there; returns that folder. Blocking: run it on a blocking thread. `progress` gets every phase.
pub fn ensure_model(models: &Path, src: &ModelSource, progress: &dyn Fn(Phase)) -> Result<PathBuf> {
    let dir = models.join(&src.name);
    if model_ready(&dir) {
        return Ok(dir);
    }
    std::fs::create_dir_all(models).with_context(|| format!("{}", models.display()))?;
    let archive = models.join(format!("{}.tar.bz2", src.name));
    if !archive.is_file() {
        download(&src.url, &archive, progress)?;
    }
    progress(Phase::Checking);
    let got = sha256_file(&archive)?;
    if !got.eq_ignore_ascii_case(&src.sha256) {
        let _ = std::fs::remove_file(&archive);
        bail!(
            "the speech model download is damaged (SHA-256 {got}, expected {}); it will be fetched again",
            src.sha256
        );
    }
    progress(Phase::Unpacking);
    let tmp = models.join(format!(".unpack-{}", src.name));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let f = std::fs::File::open(&archive)?;
    tar::Archive::new(bzip2::read::BzDecoder::new(std::io::BufReader::new(f)))
        .unpack(&tmp)
        .context("unpacking the speech model")?;
    let inner = tmp.join(&src.name);
    let from = if inner.is_dir() { inner } else { tmp.clone() };
    if !model_ready(&from) {
        bail!("the speech model archive doesn't have {}", MODEL_FILES.join(", "));
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&from, &dir)?;
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_file(&archive);
    Ok(dir)
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// GET `url` into `to` (via `to.part`, resumed with a Range request when there is one).
fn download(url: &str, to: &Path, progress: &dyn Fn(Phase)) -> Result<()> {
    let part = to.with_extension("bz2.part");
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(20)).build();
    let mut req = agent.get(url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| anyhow!("downloading the speech model: {e}"))?;
    let resumed = have > 0 && resp.status() == 206;
    let len: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let total = if resumed { have + len } else { len };
    let mut done = if resumed { have } else { 0 };
    let mut out =
        std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part)?;
    let mut body = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    log::info!("downloading the speech model ({} MB){}", total / 1_000_000, if resumed { ", resuming" } else { "" });
    loop {
        let n = body.read(&mut buf).context("downloading the speech model")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if last.elapsed().as_millis() >= 250 {
            last = std::time::Instant::now();
            progress(Phase::Downloading { done, total });
        }
    }
    out.flush()?;
    progress(Phase::Downloading { done, total });
    if total > 0 && done != total {
        bail!("the speech model download stopped at {done} of {total} bytes (it resumes next time)");
    }
    std::fs::rename(&part, to)?;
    Ok(())
}

// --- the worker --------------------------------------------------------------------------------------

type Job = (Vec<f32>, oneshot::Sender<Result<String>>);
type OnChange = Arc<dyn Fn(&Phase) + Send + Sync>;

/// The speech engine on its own thread (sherpa-onnx isn't async; one model, one clip at a time).
pub struct Speech {
    phase: Mutex<Phase>,
    jobs: Mutex<Option<std::sync::mpsc::Sender<Job>>>,
    on_change: Mutex<Option<OnChange>>,
}

impl Default for Speech {
    fn default() -> Self {
        Self { phase: Mutex::new(Phase::Idle), jobs: Mutex::new(None), on_change: Mutex::new(None) }
    }
}

impl Speech {
    /// Called with every phase change (the server turns them into `speech` events).
    pub fn on_change(&self, f: impl Fn(&Phase) + Send + Sync + 'static) {
        *self.on_change.lock().unwrap() = Some(Arc::new(f));
    }

    pub fn set_phase(&self, p: Phase) {
        let changed = {
            let mut cur = self.phase.lock().unwrap();
            let changed = *cur != p;
            *cur = p.clone();
            changed
        };
        let cb = self.on_change.lock().unwrap().clone();
        if changed && let Some(cb) = cb {
            cb(&p);
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase.lock().unwrap().clone()
    }

    pub fn ready(&self) -> bool {
        matches!(self.phase(), Phase::Ready { .. })
    }

    /// Loads `engine` on a new thread (plus a warm-up on 1 s of silence: the first run is slow),
    /// then serves [`Speech::transcribe`].
    pub fn start(self: &Arc<Self>, mut engine: Box<dyn Stt>) {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        *self.jobs.lock().unwrap() = Some(tx);
        let me = self.clone();
        self.set_phase(Phase::Loading);
        std::thread::Builder::new()
            .name("stt".into())
            .spawn(move || {
                let t = std::time::Instant::now();
                if let Err(e) = engine.load() {
                    log::error!("speech model: {e:#}");
                    me.set_phase(Phase::Failed(format!("{e:#}")));
                    return;
                }
                let _ = engine.transcribe(&vec![0.0; 16_000], stt::RATE);
                let load_s = (t.elapsed().as_secs_f64() * 100.0).round() / 100.0;
                log::info!("speech model ready in {load_s:.1}s ({})", engine.name());
                me.set_phase(Phase::Ready { engine: engine.name(), load_s });
                while let Ok((audio, reply)) = rx.recv() {
                    let r = if audio.len() < 1600 { Ok(String::new()) } else { engine.transcribe(&audio, stt::RATE) };
                    let _ = reply.send(r);
                }
            })
            .expect("a thread for speech to text");
    }

    /// 16 kHz mono audio -> text ("" for silence or under 0.1 s).
    pub async fn transcribe(&self, audio: Vec<f32>) -> Result<String> {
        let tx = self.jobs.lock().unwrap().clone().ok_or_else(|| anyhow!("speech model not loaded yet"))?;
        let (rtx, rrx) = oneshot::channel();
        tx.send((audio, rtx)).map_err(|_| anyhow!("the speech worker stopped"))?;
        rrx.await.map_err(|_| anyhow!("the speech worker stopped"))?
    }
}

/// The real engine: sherpa-onnx Parakeet with word hints.
pub fn parakeet(dir: &Path, hints: &[String]) -> Box<dyn Stt> {
    let threads = std::thread::available_parallelism().map(|n| n.get().min(8) as i32).unwrap_or(4);
    let mut e = stt::sherpa::SherpaParakeet::new(dir, "cpu", threads);
    e.hints = hints.to_vec();
    Box::new(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A speech engine that says what it's told.
    pub struct Canned(pub String);
    impl Stt for Canned {
        fn load(&mut self) -> Result<()> {
            Ok(())
        }
        fn transcribe(&mut self, _a: &[f32], _r: u32) -> Result<String> {
            Ok(self.0.clone())
        }
        fn name(&self) -> String {
            "canned".into()
        }
    }

    #[tokio::test]
    async fn the_worker_loads_then_transcribes() {
        let s = Arc::new(Speech::default());
        assert!(s.transcribe(vec![0.0; 16000]).await.is_err()); // not started
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        s.on_change(move |p| s2.lock().unwrap().push(p.to_json()["phase"].as_str().unwrap().to_string()));
        s.start(Box::new(Canned("hello".into())));
        assert_eq!(s.transcribe(vec![0.1; 16000]).await.unwrap(), "hello");
        assert_eq!(s.transcribe(vec![0.1; 100]).await.unwrap(), ""); // under 0.1 s
        assert!(s.ready());
        assert_eq!(*seen.lock().unwrap(), vec!["loading", "ready"]);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-speech-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A tiny model archive like the release's: `<name>/{encoder,decoder,joiner}.int8.onnx + tokens.txt`.
    fn archive(dir: &Path, name: &str) -> (PathBuf, String) {
        let path = dir.join("m.tar.bz2");
        let f = std::fs::File::create(&path).unwrap();
        let mut t = tar::Builder::new(bzip2::write::BzEncoder::new(f, bzip2::Compression::fast()));
        for file in MODEL_FILES {
            let data = format!("fake {file}");
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            t.append_data(&mut h, format!("{name}/{file}"), data.as_bytes()).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
        let sha = sha256_file(&path).unwrap();
        (path, sha)
    }

    /// One-shot HTTP server for `body` (honours a Range request), on 127.0.0.1.
    fn serve(body: Vec<u8>, times: usize) -> String {
        use std::io::BufRead;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/m.tar.bz2", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for _ in 0..times {
                let (mut s, _) = l.accept().unwrap();
                let mut r = std::io::BufReader::new(s.try_clone().unwrap());
                let mut from = 0usize;
                loop {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                }
                let part = &body[from..];
                let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", part.len()).unwrap();
                s.write_all(part).unwrap();
            }
        });
        url
    }

    #[test]
    fn the_model_is_downloaded_checked_and_unpacked() {
        let d = tmp("dl");
        let (path, sha) = archive(&d, "m1");
        let body = std::fs::read(&path).unwrap();
        let models = d.join("models");
        let src = ModelSource { url: serve(body.clone(), 1), sha256: sha, name: "m1".into() };
        let phases = Mutex::new(Vec::new());
        let dir = ensure_model(&models, &src, &|p| phases.lock().unwrap().push(p)).unwrap();
        assert!(model_ready(&dir));
        assert_eq!(std::fs::read_to_string(dir.join("tokens.txt")).unwrap(), "fake tokens.txt");
        let p = phases.lock().unwrap();
        assert_eq!(p.last(), Some(&Phase::Unpacking));
        assert!(p.contains(&Phase::Downloading { done: body.len() as u64, total: body.len() as u64 }));
        assert!(!models.join("m1.tar.bz2").exists()); // the archive is removed
        // there now: nothing to do
        assert_eq!(ensure_model(&models, &src, &|_| panic!("no work")).unwrap(), dir);
    }

    #[test]
    fn a_partial_download_is_resumed() {
        let d = tmp("resume");
        let (path, sha) = archive(&d, "m2");
        let body = std::fs::read(&path).unwrap();
        let models = d.join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("m2.tar.bz2.part"), &body[..body.len() / 2]).unwrap();
        let src = ModelSource { url: serve(body, 1), sha256: sha, name: "m2".into() };
        assert!(model_ready(&ensure_model(&models, &src, &|_| {}).unwrap()));
    }

    #[test]
    fn a_damaged_download_is_refused_and_removed() {
        let d = tmp("sha");
        let (path, _) = archive(&d, "m3");
        let models = d.join("models");
        let src =
            ModelSource { url: serve(std::fs::read(&path).unwrap(), 1), sha256: "00".repeat(32), name: "m3".into() };
        let e = ensure_model(&models, &src, &|_| {}).unwrap_err().to_string();
        assert!(e.contains("damaged"), "{e}");
        assert!(!models.join("m3.tar.bz2").exists());
        assert!(!model_ready(&models.join("m3")));
    }
}
