//! The app's log (`__main__.py` `setup_logging` + `EventLog`): `RUST_LOG` (default `info`) to
//! stderr as before, and every line also to `~/.stickybot-rs/app.log` (rotated at 2 MB, 2 old
//! files kept: `app.log.1`, `app.log.2`) and to the dashboard's live feed (`log` events, the Logs
//! page's "Follow live"). `GET /api/logs?n=` reads the file's last lines.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const MAX_BYTES: u64 = 2_000_000;
pub const KEEP: usize = 2;

/// A log file that rotates by size (Python's `RotatingFileHandler`).
pub struct RotatingFile {
    path: PathBuf,
    max: u64,
    keep: usize,
    file: Option<File>,
    size: u64,
}

impl RotatingFile {
    pub fn new(path: impl Into<PathBuf>, max: u64, keep: usize) -> Self {
        Self { path: path.into(), max, keep, file: None, size: 0 }
    }

    fn rotated(&self, n: usize) -> PathBuf {
        let mut s = self.path.as_os_str().to_owned();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    }

    fn rotate(&mut self) {
        self.file = None;
        let _ = fs::remove_file(self.rotated(self.keep));
        for n in (1..self.keep).rev() {
            let _ = fs::rename(self.rotated(n), self.rotated(n + 1));
        }
        if self.keep > 0 {
            let _ = fs::rename(&self.path, self.rotated(1));
        } else {
            let _ = fs::remove_file(&self.path);
        }
    }

    /// Appends one line (adds the newline).
    pub fn write_line(&mut self, line: &str) {
        let n = line.len() as u64 + 1;
        if self.file.is_some() && self.size + n > self.max {
            self.rotate();
        }
        if self.file.is_none() {
            if let Some(d) = self.path.parent() {
                let _ = fs::create_dir_all(d);
            }
            match OpenOptions::new().create(true).append(true).open(&self.path) {
                Ok(f) => {
                    self.size = f.metadata().map(|m| m.len()).unwrap_or(0);
                    self.file = Some(f);
                    if self.size + n > self.max && self.size > 0 {
                        self.rotate();
                        return self.write_line(line);
                    }
                }
                Err(_) => return,
            }
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(format!("{line}\n").as_bytes()).is_ok()
        {
            self.size += n;
        }
    }
}

type Live = Box<dyn Fn(&str) + Send + Sync>;

struct Tee {
    inner: env_logger::Logger,
    file: Mutex<RotatingFile>,
    live: Mutex<Option<Live>>,
}

static TEE: OnceLock<&'static Tee> = OnceLock::new();

impl log::Log for Tee {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.inner.enabled(m)
    }

    fn log(&self, record: &log::Record) {
        if !self.inner.matches(record) {
            return;
        }
        self.inner.log(record);
        let msg = format!("{}: {}", short_target(record.target()), record.args());
        let time = chrono::Local::now().format("%H:%M:%S");
        let line = if record.level() <= log::Level::Warn {
            format!("{time} {} {msg}", record.level())
        } else {
            format!("{time} {msg}")
        };
        if let Ok(mut f) = self.file.lock() {
            f.write_line(&line);
        }
        // try_lock: a log line from inside the live feed itself must not deadlock
        if let Ok(live) = self.live.try_lock()
            && let Some(f) = live.as_ref()
        {
            f(&msg);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// `stickybot_server::wmic` -> `wmic` (Python's logger names are short too).
fn short_target(t: &str) -> &str {
    t.rsplit("::").next().unwrap_or(t)
}

/// Sets up logging for the app: stderr (`RUST_LOG`, default `info`) + `log_file` + the live feed
/// (once [`set_live`] is called). Call once, first thing; later calls do nothing.
pub fn init(log_file: &Path) {
    if TEE.get().is_some() {
        return;
    }
    let inner = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    let max = inner.filter();
    let tee: &'static Tee = Box::leak(Box::new(Tee {
        inner,
        file: Mutex::new(RotatingFile::new(log_file, MAX_BYTES, KEEP)),
        live: Mutex::new(None),
    }));
    if log::set_logger(tee).is_ok() {
        log::set_max_level(max);
        let _ = TEE.set(tee);
    }
}

/// Where log lines go besides stderr and the file: the dashboard's feed (`log` events).
pub fn set_live(f: Option<Live>) {
    if let Some(t) = TEE.get() {
        *t.live.lock().unwrap() = f;
    }
}

/// The last `n` lines of the log file (server.py `logs`).
pub fn tail(path: &Path, n: usize) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-logs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rotates_by_size_and_keeps_two() {
        let d = tmp("rotate");
        let p = d.join("app.log");
        let mut f = RotatingFile::new(&p, 100, 2);
        for i in 0..40 {
            f.write_line(&format!("line {i:03} ........")); // 21 bytes each
        }
        assert!(fs::metadata(&p).unwrap().len() <= 100);
        assert!(d.join("app.log.1").exists() && d.join("app.log.2").exists());
        assert!(!d.join("app.log.3").exists());
        let last = tail(&p, 1);
        assert_eq!(last, vec!["line 039 ........".to_string()]);
        // a new writer appends to the existing file (and rotates it when full)
        let mut g = RotatingFile::new(&p, 100, 2);
        g.write_line("again");
        assert_eq!(tail(&p, 1), vec!["again".to_string()]);
    }

    #[test]
    fn tail_reads_the_last_lines() {
        let d = tmp("tail");
        let p = d.join("app.log");
        assert!(tail(&p, 5).is_empty());
        fs::write(&p, "a\nb\nc\n").unwrap();
        assert_eq!(tail(&p, 2), vec!["b", "c"]);
        assert_eq!(tail(&p, 10), vec!["a", "b", "c"]);
        assert_eq!(short_target("stickybot_server::wmic"), "wmic");
    }
}
