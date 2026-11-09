//! What the setup page and the components ask of this Mac (installer.py's `run`, `shutil.which`,
//! install_flow.py's `stream_process`, platform checks, Claude Code's settings file), behind one
//! trait: the real Mac ([`MacSystem`]) or a pretend one ([`FakeSystem`], tests and the fake stick:
//! nothing runs `gh`, `claude`, `curl` or touches `~/.claude/` there).

use crate::AppState;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Called with each piece of output as it comes; returns bytes to write to the program's stdin.
pub type OnChunk<'a> = &'a mut dyn FnMut(&str) -> Option<Vec<u8>>;

pub trait System: Send + Sync {
    /// Runs a program, waits; -> (exit code, stdout + stderr). A missing program: `(None, why)`.
    fn run(&self, argv: &[String], timeout: Duration) -> (Option<i32>, String);
    /// Runs a program, handing its output (stdout and stderr) over as it comes. `cancel` stops it.
    /// -> the exit code (`None`: timed out, cancelled or didn't start).
    fn stream(
        &self,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cancel: &AtomicBool,
        on_chunk: OnChunk,
    ) -> Option<i32>;
    /// A program on the PATH or in Homebrew's / ~/.local/bin.
    fn which(&self, name: &str) -> Option<PathBuf>;
    /// Claude Code (config `claude_path`, else where it installs).
    fn claude(&self, cfg: &serde_json::Map<String, serde_json::Value>) -> Option<PathBuf>;
    /// Claude Code's user settings (`~/.claude/settings.json`).
    fn claude_settings(&self) -> PathBuf;
    /// This program (the hooks run it with `--hook …`).
    fn exe(&self) -> PathBuf;
    /// `uname -m` (`arm64`), the macOS version (`26.1`), free bytes in the home folder.
    fn machine(&self) -> (String, String, Option<u64>);
    /// BlackHole 2ch's driver is installed.
    fn blackhole(&self) -> bool;
    /// Starts the speech model's download + load (st.spawn_speech()).
    fn start_speech(&self, st: &Arc<AppState>);
}

/// PATH with the places Claude Code, gh and Homebrew install to (an app's PATH is short).
pub fn agent_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = [format!("{home}/.local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    let cur = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    extra
        .iter()
        .cloned()
        .chain(cur.split(':').filter(|p| !extra.iter().any(|e| e == p)).map(str::to_string))
        .collect::<Vec<_>>()
        .join(":")
}

pub const BLACKHOLE_DRIVERS: &[&str] =
    &["/Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver", "/Library/Audio/Plug-Ins/HAL/BlackHole.driver"];

/// The real Mac.
pub struct MacSystem;

impl System for MacSystem {
    fn run(&self, argv: &[String], timeout: Duration) -> (Option<i32>, String) {
        let mut out = String::new();
        let cancel = AtomicBool::new(false);
        let code = self.stream(argv, &[], timeout, &cancel, &mut |s| {
            out.push_str(s);
            None
        });
        (code, out)
    }

    fn stream(
        &self,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cancel: &AtomicBool,
        on_chunk: OnChunk,
    ) -> Option<i32> {
        let (prog, args) = argv.split_first()?;
        // stdout and stderr in one stream, in order (`2>&1`), like Popen(stderr=STDOUT)
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg("exec \"$@\" 2>&1").arg("sh").arg(prog).args(args);
        cmd.env("PATH", agent_path());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().ok()?;
        let mut stdout = child.stdout.take()?;
        let mut stdin = child.stdin.take();
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let expired = Arc::new(AtomicBool::new(false));
        // the watchdog: timeout or cancel kills it
        let stop = Arc::new(AtomicBool::new(false));
        {
            // the reader loop below turns `cancel` into `stop`
            let (child, finished, expired, stop) = (child.clone(), finished.clone(), expired.clone(), stop.clone());
            std::thread::spawn(move || {
                let t0 = std::time::Instant::now();
                while !finished.load(Ordering::SeqCst) {
                    if t0.elapsed() > timeout || stop.load(Ordering::SeqCst) {
                        expired.store(true, Ordering::SeqCst);
                        let _ = child.lock().unwrap().kill();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        loop {
            if cancel.load(Ordering::SeqCst) {
                stop.store(true, Ordering::SeqCst);
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(b) => {
                    let s = String::from_utf8_lossy(&b).into_owned();
                    if let Some(input) = on_chunk(&s)
                        && let Some(i) = stdin.as_mut()
                    {
                        let _ = i.write_all(&input).and_then(|_| i.flush());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(stdin);
        let status = loop {
            if let Ok(Some(s)) = child.lock().unwrap().try_wait() {
                break Some(s);
            }
            if expired.load(Ordering::SeqCst) {
                break child.lock().unwrap().wait().ok();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        finished.store(true, Ordering::SeqCst);
        if expired.load(Ordering::SeqCst) {
            return None;
        }
        status.and_then(|s| s.code())
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        agent_path().split(':').map(|d| Path::new(d).join(name)).find(|p| p.is_file())
    }

    fn claude(&self, cfg: &serde_json::Map<String, serde_json::Value>) -> Option<PathBuf> {
        crate::brain::claude_path(cfg)
    }

    fn claude_settings(&self) -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".claude").join("settings.json")
    }

    fn exe(&self) -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("stickybot-headless"))
    }

    fn machine(&self) -> (String, String, Option<u64>) {
        let s = |argv: &[&str]| {
            let (code, out) = self.run(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>(), Duration::from_secs(5));
            if code == Some(0) { out.trim().to_string() } else { String::new() }
        };
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let df = s(&["/bin/df", "-k", &home]);
        (s(&["/usr/bin/uname", "-m"]), s(&["/usr/bin/sw_vers", "-productVersion"]), free_from_df(&df))
    }

    fn blackhole(&self) -> bool {
        BLACKHOLE_DRIVERS.iter().any(|p| Path::new(p).exists())
    }

    fn start_speech(&self, st: &Arc<AppState>) {
        st.spawn_speech();
    }
}

/// The free bytes from `df -k <dir>`'s output (the 4th column of the last line).
pub fn free_from_df(out: &str) -> Option<u64> {
    let line = out.lines().rfind(|l| !l.trim().is_empty())?;
    line.split_whitespace().nth(3)?.parse::<u64>().ok().map(|k| k * 1024)
}

/// A pretend Mac: answers from a script, records what ran.
pub struct FakeSystem {
    /// `"gh auth status"` -> (exit code, output); unknown commands exit 127
    pub answers: Mutex<HashMap<String, (i32, String)>>,
    /// what `stream` prints (in pieces) and its exit code, by the command's first two words
    pub streams: Mutex<HashMap<String, (Vec<String>, i32)>>,
    /// after a stream ran: these answers change (e.g. `gh auth status` succeeds once logged in)
    pub after_stream: Mutex<HashMap<String, (i32, String)>>,
    pub programs: Mutex<HashMap<String, PathBuf>>,
    pub claude: Mutex<Option<PathBuf>>,
    pub settings: PathBuf,
    pub exe: PathBuf,
    pub mac: Mutex<(String, String, Option<u64>)>,
    pub blackhole: AtomicBool,
    /// everything run / streamed, and what was written to stdin
    pub ran: Mutex<Vec<String>>,
    pub stdin: Mutex<Vec<u8>>,
}

impl FakeSystem {
    pub fn new(home: &Path) -> Self {
        Self {
            answers: Mutex::default(),
            streams: Mutex::default(),
            after_stream: Mutex::default(),
            programs: Mutex::default(),
            claude: Mutex::new(None),
            settings: home.join("fake-claude").join("settings.json"),
            exe: PathBuf::from("/Applications/StickyBot (Rust).app/Contents/MacOS/stickybot-app"),
            mac: Mutex::new(("arm64".into(), "26.1".into(), Some(100_000_000_000))),
            blackhole: AtomicBool::new(false),
            ran: Mutex::default(),
            stdin: Mutex::default(),
        }
    }

    pub fn answer(&self, cmd: &str, code: i32, out: &str) {
        self.answers.lock().unwrap().insert(cmd.into(), (code, out.into()));
    }

    pub fn program(&self, name: &str) {
        self.programs.lock().unwrap().insert(name.into(), PathBuf::from(format!("/opt/homebrew/bin/{name}")));
    }

    fn key(argv: &[String]) -> String {
        let mut parts: Vec<String> = argv.iter().map(|a| a.rsplit('/').next().unwrap_or(a).to_string()).collect();
        parts.truncate(3);
        parts.join(" ")
    }
}

impl System for FakeSystem {
    fn run(&self, argv: &[String], _timeout: Duration) -> (Option<i32>, String) {
        let key = Self::key(argv);
        self.ran.lock().unwrap().push(argv.join(" "));
        let a = self.answers.lock().unwrap();
        match a.get(&key).or_else(|| a.get(&key.split(' ').take(2).collect::<Vec<_>>().join(" "))) {
            Some((c, o)) => (Some(*c), o.clone()),
            None => (Some(127), format!("{key}: not found")),
        }
    }

    fn stream(
        &self,
        argv: &[String],
        _env: &[(String, String)],
        _timeout: Duration,
        cancel: &AtomicBool,
        on_chunk: OnChunk,
    ) -> Option<i32> {
        let key = Self::key(argv);
        self.ran.lock().unwrap().push(argv.join(" "));
        let two = key.split(' ').take(2).collect::<Vec<_>>().join(" ");
        let script = {
            let s = self.streams.lock().unwrap();
            s.get(&key).or_else(|| s.get(&two)).cloned()
        };
        let (chunks, code) = script.unwrap_or((vec![format!("{key}: not found\n")], 127));
        for c in chunks {
            if cancel.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(input) = on_chunk(&c) {
                self.stdin.lock().unwrap().extend(input);
            }
        }
        let after: Vec<(String, (i32, String))> = self.after_stream.lock().unwrap().drain().collect();
        self.answers.lock().unwrap().extend(after);
        Some(code)
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.programs.lock().unwrap().get(name).cloned()
    }

    fn claude(&self, _cfg: &serde_json::Map<String, serde_json::Value>) -> Option<PathBuf> {
        self.claude.lock().unwrap().clone()
    }

    fn claude_settings(&self) -> PathBuf {
        self.settings.clone()
    }

    fn exe(&self) -> PathBuf {
        self.exe.clone()
    }

    fn machine(&self) -> (String, String, Option<u64>) {
        self.mac.lock().unwrap().clone()
    }

    fn blackhole(&self) -> bool {
        self.blackhole.load(Ordering::SeqCst)
    }

    /// Pretends to download and load the model.
    fn start_speech(&self, st: &Arc<AppState>) {
        use crate::speech::Phase;
        st.speech.set_phase(Phase::Downloading { done: 50, total: 100 });
        st.speech.set_phase(Phase::Loading);
        st.speech.set_phase(Phase::Ready { engine: "fake".into(), load_s: 0.1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_output() {
        let out = "Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on\n\
                   /dev/disk3s5 971350180 500000000 400000000 56% 1 2 0% /System/Volumes/Data\n";
        assert_eq!(free_from_df(out), Some(400_000_000 * 1024));
        assert_eq!(free_from_df(""), None);
    }

    #[test]
    fn the_real_runner_streams_output_and_stdin() {
        // only /bin/sh and /bin/cat: nothing that logs in or installs anything
        let sys = MacSystem;
        let argv: Vec<String> = ["/bin/sh", "-c", "printf 'Press Enter '; read x; echo got:$x; exit 3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut text = String::new();
        let cancel = AtomicBool::new(false);
        let code = sys.stream(&argv, &[], Duration::from_secs(10), &cancel, &mut |s| {
            text.push_str(s);
            text.contains("Press Enter").then(|| b"yes\n".to_vec()).filter(|_| !text.contains("got"))
        });
        assert_eq!(code, Some(3));
        assert!(text.contains("got:yes"), "{text}");
        let (code, out) =
            sys.run(&["/bin/sh".into(), "-c".into(), "echo err >&2; exit 0".into()], Duration::from_secs(5));
        assert_eq!((code, out.trim()), (Some(0), "err"));
        assert!(matches!(sys.run(&["/nonexistent/x".into()], Duration::from_secs(5)).0, Some(126 | 127)));
        let (code, _) = sys.run(&["/bin/sleep".into(), "5".into()], Duration::from_millis(200));
        assert_eq!(code, None);
    }
}
