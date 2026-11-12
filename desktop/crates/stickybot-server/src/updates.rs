//! Update checks (a port of `updates.py`, the firmware part): is there a StickyBot release with
//! newer firmware than the connected stick's? Checked 20 s after start-up and then every 24 h
//! (config `update_check`), on `update_channel`. A check lists the releases (`releases.rs`) and
//! downloads the latest release's firmware into `~/.stickybot-rs/firmware/<version>/` when it
//! isn't there yet, so "Update over Bluetooth" / "Update via USB" work straight away.
//!
//! A newer StickyBot.app — the latest release is newer than this app's version (`CARGO_PKG_VERSION`,
//! set by `tools/release_tool.py bump`) and has the `StickyBot-<version>-macos-arm64.zip` asset — is
//! `agent_update` ("Update available: vX"), with the release page in `install.download`. When that
//! release also has its `latest.json` (the signed update the release workflow makes) and the menu
//! bar app is the one running, `install.can_update` is true and `selfupdate.rs` installs it: the
//! status then also carries `state`, `progress`, `waiting`, `install_error`, `updated`. Otherwise
//! `install.why` says why, with the release page as the way out. State: `~/.stickybot-rs/updates.json`.

use crate::releases::{self, ReleaseError, Source};
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
pub const FIRST_CHECK: Duration = Duration::from_secs(20);
pub const TICK: Duration = Duration::from_secs(60);

/// The release asset with StickyBot.app for this Mac (the release workflow's `mac-app` job).
pub const APP_ASSET_SUFFIX: &str = "-macos-arm64.zip";

pub fn is_app_asset(name: &str) -> bool {
    name.starts_with("StickyBot-") && name.ends_with(APP_ASSET_SUFFIX)
}

/// Where releases come from (tests swap it for a fake GitHub).
pub type Sources = Arc<dyn Fn() -> Vec<Box<dyn Source>> + Send + Sync>;

pub fn channel(cfg: &Map<String, Value>) -> String {
    match cfg.get("update_channel").and_then(Value::as_str) {
        Some(c) if releases::CHANNELS.contains(&c) => c.to_string(),
        _ => "stable".into(),
    }
}

#[derive(Default)]
struct State {
    checking: bool,
    ran: bool,
    error: String,
    note: String,
    checked_at: f64,
    /// a release summary + "firmware" (its firmware version)
    latest: Option<Value>,
    stick_fw: Option<String>,
    /// release tags already toasted on the stick
    notified: Vec<String>,
}

pub struct Updates {
    path: PathBuf,
    firmware_root: PathBuf,
    repo: String,
    /// this app's version (the release it was built for: tools/release_tool.py `bump`)
    current: String,
    sources: Mutex<Sources>,
    s: Mutex<State>,
    lock: tokio::sync::Mutex<()>,
    /// installing the update itself (`selfupdate.rs`): the installer, the state, what was tried
    pub run: crate::selfupdate::Run,
}

impl Updates {
    pub fn new(home: &std::path::Path) -> Self {
        let path = home.join("updates.json");
        let mut s = State::default();
        let saved = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        if let Some(v) = saved.clone() {
            s.checked_at = v.get("checked_at").and_then(Value::as_f64).unwrap_or(0.0);
            s.latest = v.get("latest").filter(|l| l.is_object()).cloned();
            s.notified = v
                .get("notified")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
        }
        let repo = releases::repo();
        let r2 = repo.clone();
        Self {
            path,
            firmware_root: crate::firmware::firmware_root(home),
            repo,
            current: env!("CARGO_PKG_VERSION").to_string(),
            sources: Mutex::new(Arc::new(move || releases::sources(&r2))),
            s: Mutex::new(s),
            lock: tokio::sync::Mutex::new(()),
            run: crate::selfupdate::Run::load(saved.as_ref()),
        }
    }

    /// The last check's release (`tag`, `version`, `app_asset`, `updater_asset`, …).
    pub(crate) fn latest(&self) -> Option<Value> {
        self.s.lock().unwrap().latest.clone()
    }

    pub(crate) fn current(&self) -> &str {
        &self.current
    }

    pub(crate) fn repo(&self) -> &str {
        &self.repo
    }

    pub fn set_sources(&self, s: Sources) {
        *self.sources.lock().unwrap() = s;
    }

    pub(crate) fn save(&self) {
        let s = self.s.lock().unwrap();
        let keep = s.notified.len().saturating_sub(20);
        let mut v = json!({"checked_at": s.checked_at, "latest": s.latest, "notified": s.notified[keep..]});
        if let Some(o) = v.as_object_mut() {
            self.run.save_into(o);
        }
        if let Err(e) = std::fs::write(&self.path, serde_json::to_string_pretty(&v).unwrap_or_default()) {
            log::warn!("couldn't save {}: {e}", self.path.display());
        }
    }

    pub fn set_stick_fw(&self, v: Option<String>) {
        if v.is_some() {
            self.s.lock().unwrap().stick_fw = v;
        }
    }

    /// updates.py `status` (+ the notes with `notes`).
    pub fn status(&self, cfg: &Map<String, Value>, notes: bool) -> Value {
        let s = self.s.lock().unwrap();
        let lat = s.latest.clone();
        let fw = lat.as_ref().and_then(|l| l.get("firmware")).and_then(Value::as_str).map(str::to_string);
        let firmware_update = fw.is_some() && s.stick_fw.is_some() && releases::newer(fw.as_deref(), s.stick_fw.as_deref());
        // a newer StickyBot.app: only when the release has the Mac app's download (installable in-app only with `latest.json` too: `selfupdate::plan`)
        let tag = lat.as_ref().and_then(|l| l.get("tag")).and_then(Value::as_str).unwrap_or("").to_string();
        let ver = lat.as_ref().and_then(|l| l.get("version")).and_then(Value::as_str).map(str::to_string);
        let has_app = lat.as_ref().is_some_and(|l| l.get("app_asset").is_some_and(Value::is_string));
        let agent_update = has_app && releases::newer(ver.as_deref(), Some(&self.current));
        let title = if agent_update {
            format!("Update available: {tag}")
        } else if firmware_update {
            format!("Stick firmware {} available", fw.unwrap_or_default())
        } else {
            String::new()
        };
        let page = self.page_of(lat.as_ref());
        // can the app install it itself, and if not, why not
        let (can_update, why) = match self.plan_with(lat.as_ref(), cfg) {
            Ok(_) => (true, String::new()),
            Err(r) => (false, r.words()),
        };
        let latest = lat.map(|mut l| {
            if !notes && let Some(o) = l.as_object_mut() {
                o.remove("notes");
            }
            l
        });
        let phase = self.run.phase();
        let state = match phase.state() {
            Some(st) => st,
            None if s.checking && phase != crate::selfupdate::Phase::Waiting => "checking",
            None if agent_update => "available",
            None => "idle",
        };
        let mut st = json!({"enabled": cfg.get("update_check").and_then(Value::as_bool) != Some(false),
               "channel": channel(cfg), "checking": s.checking,
               "checked_at": if s.checked_at > 0.0 { json!(s.checked_at) } else { Value::Null },
               "error": s.error, "note": s.note,
               "current": {"agent": self.current, "firmware": s.stick_fw},
               "latest": latest, "agent_update": agent_update, "firmware_update": firmware_update,
               "available": agent_update || firmware_update, "title": title,
               "state": state, "auto": cfg.get("update_auto").and_then(Value::as_bool).unwrap_or(false),
               "install": {"can_update": can_update, "why": why}});
        if let Some(o) = st.as_object_mut() {
            o.extend(self.run.fields(&page));
        }
        st
    }

    /// `GET /api/update`: the status with notes, how this app is installed, the updaters.
    pub fn full(&self, cfg: &Map<String, Value>) -> Value {
        let mut st = self.status(cfg, true);
        let page = self.download_page(&st);
        let file = st["latest"]["app_asset"].as_str().unwrap_or("StickyBot-<version>-macos-arm64.zip").to_string();
        if let Some(o) = st["install"].as_object_mut() {
            o.insert("kind".into(), json!("rust"));
            o.insert("path".into(), json!(std::env::current_exe().ok()));
            o.insert("head".into(), Value::Null);
            o.insert("branch".into(), Value::Null);
            o.insert("download".into(), json!(page));
            o.insert(
                "instructions".into(),
                json!([format!("{page}  ({file}: unzip, quit StickyBot, replace /Applications/StickyBot.app)"),
                       "or from the repo: desktop/scripts/build-app.sh"]),
            );
        }
        // Bluetooth has its own button (POST /api/firmware/ota), USB too: no other updaters
        st["firmware_updaters"] = json!([]);
        st
    }

    fn page_of(&self, latest: Option<&Value>) -> String {
        latest
            .and_then(|l| l["url"].as_str())
            .filter(|u| u.starts_with("https://"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{}/releases/latest", self.repo))
    }

    /// Where to get the new app: the latest release's page, else the releases' latest page.
    pub fn download_page(&self, status: &Value) -> String {
        self.page_of(Some(&status["latest"]))
    }

    /// The app's version to compare releases with (tests pretend another).
    pub fn set_current(&mut self, v: &str) {
        self.current = v.to_string();
    }

    /// A check is due: on, and not yet since start-up, or the last one was 24 h ago.
    pub fn due(&self, cfg: &Map<String, Value>) -> bool {
        if cfg.get("update_check").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        let s = self.s.lock().unwrap();
        !s.ran || crate::hub::now() - s.checked_at >= CHECK_EVERY.as_secs_f64()
    }

    /// (blocking) the latest release + its firmware, downloaded.
    fn fetch(srcs: Vec<Box<dyn Source>>, repo: &str, chan: &str, root: &std::path::Path) -> Result<(Value, String), ReleaseError> {
        let (rel, src) = releases::latest(&srcs, repo, chan)?;
        let mut info = rel.summary();
        info["firmware"] = Value::Null;
        // the Mac app's download (StickyBot-<version>-macos-arm64.zip), if the release has one
        info["app_asset"] = rel.assets.keys().find(|n| is_app_asset(n)).map_or(Value::Null, |n| json!(n));
        // its signed update (`latest.json`): what "Install and restart" reads (selfupdate.rs)
        info["updater_asset"] =
            if rel.assets.contains_key(crate::selfupdate::UPDATER_ASSET) { json!(crate::selfupdate::UPDATER_ASSET) } else { Value::Null };
        if rel.assets.contains_key(releases::MANIFEST) {
            match releases::download_firmware(&rel, src, root, &|l| log::info!("{l}")) {
                Ok((_, m)) => info["firmware"] = m["version"].clone(),
                Err(e) => {
                    log::warn!("firmware of {}: {e}", rel.tag);
                    return Ok((info, format!("couldn't get its firmware: {e}")));
                }
            }
        }
        Ok((info, String::new()))
    }

    /// Checks now (one at a time) -> the full status.
    pub async fn check(&self, cfg: &Map<String, Value>) -> Value {
        let _g = self.lock.lock().await;
        self.s.lock().unwrap().checking = true;
        let chan = channel(cfg);
        let (srcs, repo, root) = ((self.sources.lock().unwrap())(), self.repo.clone(), self.firmware_root.clone());
        let c2 = chan.clone();
        let r = tokio::task::spawn_blocking(move || Self::fetch(srcs, &repo, &c2, &root)).await;
        {
            let mut s = self.s.lock().unwrap();
            match r {
                Ok(Ok((info, note))) => {
                    log::info!("update check ({chan}): latest {} (firmware {})", info["tag"], info["firmware"]);
                    s.latest = Some(info);
                    s.note = note;
                    s.error.clear();
                }
                Ok(Err(ReleaseError::NoRelease(e))) => {
                    log::info!("update check ({chan}): {e}");
                    s.latest = None;
                    s.note = e;
                    s.error.clear();
                }
                Ok(Err(e)) => {
                    log::warn!("update check failed: {e}");
                    s.error = e.to_string();
                }
                Err(e) => s.error = format!("unexpected error: {e}"),
            }
            s.checking = false;
            s.ran = true;
            s.checked_at = crate::hub::now();
        }
        self.save();
        self.full(cfg)
    }
}

/// The check loop: 20 s after start-up, then whenever due (and the stick's version on the way).
pub fn spawn(st: Arc<crate::AppState>) {
    let weak = Arc::downgrade(&st);
    drop(st);
    tokio::spawn(async move {
        if let Some(st) = weak.upgrade() {
            crate::selfupdate::after_restart(&st); // "Updated to vX", or "didn't take"
        }
        tokio::time::sleep(FIRST_CHECK).await;
        let mut asked = None::<tokio::time::Instant>;
        loop {
            let Some(st) = weak.upgrade() else { return };
            if st.link.connected() && asked.is_none_or(|t| t.elapsed() >= Duration::from_secs(3600)) {
                asked = Some(tokio::time::Instant::now());
                refresh_stick(&st).await;
            }
            let cfg = st.cfg.lock().unwrap().clone();
            if st.updates.due(&cfg) {
                st.updates.check(&cfg).await;
                emit(&st);
            }
            notify(&st).await;
            crate::selfupdate::auto_tick(&st); // `update_auto`: installs by itself when idle
            drop(st);
            tokio::time::sleep(TICK).await;
        }
    });
}

/// The connected stick's firmware version, for "Stick firmware X available".
pub async fn refresh_stick(st: &crate::AppState) {
    let c = crate::firmware::connected(&st.link).await;
    st.updates.set_stick_fw(c.get("version").and_then(Value::as_str).map(str::to_string));
}

/// One "nice" toast on the stick per release (updates.py `notify`), once the stick is there and
/// it isn't quiet hours; also when the kind is off it isn't tried again.
pub async fn notify(st: &crate::AppState) -> Option<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    let s = st.updates.status(&cfg, false);
    let tag = s["latest"]["tag"].as_str()?.to_string();
    if s["available"] != true || st.updates.s.lock().unwrap().notified.contains(&tag) {
        return None;
    }
    if !st.link.connected() || crate::events::quiet_now(&cfg) {
        return None; // later: the next tick tries again
    }
    let name = if s["agent_update"] == true {
        tag.clone()
    } else {
        format!("firmware {}", s["latest"]["firmware"].as_str().unwrap_or(""))
    };
    let ev = crate::events::notify(st, "update", &format!("Update {name} ready"), "see the dashboard", None, None, None).await;
    st.updates.s.lock().unwrap().notified.push(tag);
    st.updates.save();
    ev
}

/// The `update` event for the banner.
/// "Check for updates" (the dashboard's button, the menu bar's item): the stick's version, the
/// releases, the pages told -> the full status.
pub async fn check_now(st: &crate::AppState) -> Value {
    refresh_stick(st).await;
    let cfg = st.cfg.lock().unwrap().clone();
    let r = st.updates.check(&cfg).await;
    emit(st);
    r
}

pub fn emit(st: &crate::AppState) {
    let cfg = st.cfg.lock().unwrap().clone();
    let s = st.updates.status(&cfg, false);
    let mut ev = Map::new();
    for k in ["available", "title", "agent_update", "firmware_update", "error", "note", "state", "progress", "waiting", "install_error", "updated"] {
        ev.insert(k.into(), s[k].clone());
    }
    st.hub.emit("update", Value::Object(ev));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::releases::tests::FakeGitHub;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-upd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake(f: impl Fn() -> FakeGitHub + Send + Sync + 'static) -> Sources {
        Arc::new(move || vec![Box::new(f()) as Box<dyn Source>])
    }

    #[tokio::test]
    async fn a_check_downloads_the_firmware_and_compares_with_the_stick() {
        let home = tmp("check");
        let u = Updates::new(&home);
        u.set_sources(fake(|| {
            let mut g = FakeGitHub::default();
            g.add_release("v2.2.0", "2.3.0", false);
            g.add_release("v2.3.0-beta.1", "2.4.0", true);
            g
        }));
        let mut cfg = crate::config::defaults();
        assert!(u.due(&cfg));
        let s = u.check(&cfg).await;
        assert_eq!((s["latest"]["tag"].as_str(), s["latest"]["firmware"].as_str()), (Some("v2.2.0"), Some("2.3.0")));
        assert_eq!(s["latest"]["notes"], "notes for v2.2.0");
        assert_eq!((s["available"].as_bool(), s["install"]["can_update"].as_bool()), (Some(false), Some(false)));
        assert!(home.join("firmware/2.3.0/app.bin").exists());
        assert!(!u.due(&cfg));
        // the stick runs older firmware: an update is offered
        u.set_stick_fw(Some("2.2.1".into()));
        let st = u.status(&cfg, false);
        assert_eq!((st["firmware_update"].as_bool(), st["title"].as_str()), (Some(true), Some("Stick firmware 2.3.0 available")));
        assert!(st["latest"].get("notes").is_none());
        // beta: the prerelease
        cfg.insert("update_channel".into(), json!("beta"));
        let s = u.check(&cfg).await;
        assert_eq!(s["latest"]["firmware"], "2.4.0");
        // remembered across restarts
        let again = Updates::new(&home);
        assert_eq!(again.status(&cfg, false)["latest"]["tag"], "v2.3.0-beta.1");
        cfg.insert("update_check".into(), json!(false));
        assert!(!again.due(&cfg));
    }

    #[tokio::test]
    async fn a_newer_release_with_the_mac_app_is_an_app_update_pointing_at_its_page() {
        let home = tmp("app");
        let mut u = Updates::new(&home);
        assert_eq!(u.current, env!("CARGO_PKG_VERSION"));
        u.set_current("2.3.2");
        // a newer release without the app's zip: firmware only, no app update
        u.set_sources(fake(|| {
            let mut g = FakeGitHub::default();
            g.add_release("v2.4.0", "2.6.0", false);
            g
        }));
        let cfg = crate::config::defaults();
        let s = u.check(&cfg).await;
        assert_eq!(s["latest"]["app_asset"], Value::Null);
        assert_eq!((s["agent_update"].as_bool(), s["available"].as_bool()), (Some(false), Some(false)));
        assert_eq!(s["current"]["agent"], "2.3.2");

        // with it: an app update, the release page to download it from
        u.set_sources(fake(|| {
            let mut g = FakeGitHub::default();
            g.add_release("v2.4.0", "2.6.0", false);
            g.add_app("v2.4.0");
            g
        }));
        let s = u.check(&cfg).await;
        assert_eq!(s["latest"]["app_asset"], "StickyBot-2.4.0-macos-arm64.zip");
        assert_eq!((s["agent_update"].as_bool(), s["available"].as_bool()), (Some(true), Some(true)));
        assert_eq!(s["title"], "Update available: v2.4.0");
        assert_eq!(s["install"]["download"], "https://example.invalid/v2.4.0");
        assert!(s["install"]["instructions"][0].as_str().unwrap().contains("StickyBot-2.4.0-macos-arm64.zip"));
        assert_eq!(s["install"]["can_update"], false);
        // the firmware still comes along; the app update's title wins over the firmware's
        u.set_stick_fw(Some("2.5.0".into()));
        let st = u.status(&cfg, false);
        assert_eq!((st["firmware_update"].as_bool(), st["title"].as_str()), (Some(true), Some("Update available: v2.4.0")));

        // this app is that release (or newer): nothing for the app
        u.set_current("2.4.0");
        let st = u.status(&cfg, false);
        assert_eq!((st["agent_update"].as_bool(), st["title"].as_str()), (Some(false), Some("Stick firmware 2.6.0 available")));
        u.set_current("2.5.0-beta.1");
        assert_eq!(u.status(&cfg, false)["agent_update"], false);
        // no release page known: the releases' latest page
        assert_eq!(u.download_page(&json!({"latest": null})), format!("https://github.com/{}/releases/latest", u.repo));
    }

    #[tokio::test]
    async fn no_release_and_errors_are_reported() {
        let home = tmp("err");
        let u = Updates::new(&home);
        u.set_sources(fake(FakeGitHub::default));
        let cfg = crate::config::defaults();
        let s = u.check(&cfg).await;
        assert_eq!((s["note"].as_str(), s["error"].as_str()), (Some(releases::NO_RELEASE), Some("")));
        u.set_sources(fake(|| FakeGitHub { fail_json: Some("offline".into()), ..Default::default() }));
        let s = u.check(&cfg).await;
        assert!(s["error"].as_str().unwrap().contains("offline"));
    }
}
