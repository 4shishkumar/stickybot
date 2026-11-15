//! Firmware for the stick (flasher.py's bundles + ota.py's `update` + firmware_api.py's jobs):
//! the Bluetooth path only (USB flashing with esptool isn't ported).
//!
//! Bundles (`manifest.json` + the flash parts, as `make -C m5-stick-s3 release` and the release
//! assets make them) are looked for in `~/.stickybot-rs/firmware/<version>/` and `…/local/`, and in
//! `$STICKYBOT_RS_FIRMWARE` (one bundle folder). The best is the highest version; for the same
//! version a local build wins. Only its app image (`app.bin`, offset 0x10000) goes over Bluetooth.

use crate::AppState;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use stick_link::ota::{self, CancelHandle, Image, OtaError, Sender, Timings};
use stick_link::{Backend, Link};
use tokio::sync::Notify;

pub const APP_OFFSET: &str = "0x10000";
/// After `done`: how long to wait for the stick to come back with the new version.
pub const REBOOT: Duration = Duration::from_secs(90);

/// A bundle folder's manifest, with `files` as `[(offset, path)]`, `source` and `dir`; `None` if
/// it's missing or incomplete (a part absent or of the wrong size). flasher.py `load_bundle`.
pub fn load_bundle(folder: &Path, source: &str) -> Option<Value> {
    let mut m: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("manifest.json")).ok()?).ok()?;
    let mut files = Vec::new();
    for p in m.get("parts")?.as_array()? {
        let name = p.get("file")?.as_str()?;
        if Path::new(name).components().count() != 1 || name == "." || name == ".." {
            return None;
        }
        let path = folder.join(name);
        let len = std::fs::metadata(&path).ok().filter(|m| m.is_file())?.len();
        if p.get("size").and_then(Value::as_u64).is_some_and(|s| s != len) {
            return None;
        }
        files.push(json!([p.get("offset")?.as_str()?, path]));
    }
    if files.is_empty() || m.get("version").and_then(Value::as_str).is_none_or(str::is_empty) {
        return None;
    }
    m["files"] = Value::Array(files);
    m["source"] = json!(source);
    m["dir"] = json!(folder);
    Some(m)
}

/// Board names used by older S3 releases are aliases, never a wildcard.
pub fn canonical_board(board: &str) -> Option<&'static str> {
    match board {
        "m5stick-s3" | "m5-sticks3" => Some("m5-sticks3"),
        "m5-stickc-plus2" => Some("m5-stickc-plus2"),
        _ => None,
    }
}

fn board_chip(board: &str) -> Result<(&'static str, u16), String> {
    match canonical_board(board) {
        Some("m5-sticks3") => Ok(("esp32s3", 9)),
        Some("m5-stickc-plus2") => Ok(("esp32", 0)),
        _ => Err("Device model is unknown; a compatible firmware update is unavailable".into()),
    }
}

/// Reject mismatched board/chip/layout before any transport can write an image.
pub fn check_board(fw: &Value, board: &str) -> Result<(), String> {
    let (chip, _) = board_chip(board)?;
    if canonical_board(fw["board"].as_str().unwrap_or("")) != canonical_board(board) {
        return Err(format!("No compatible firmware bundle for {board}; this update is unavailable"));
    }
    if fw["flash"]["chip"].as_str().is_some_and(|value| value != chip) {
        return Err("Firmware board and chip do not match; refusing update".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut app = false;
    for part in fw["parts"].as_array().into_iter().flatten() {
        let name = part["file"].as_str().unwrap_or("");
        let expected = match name {
            "bootloader.bin" => {
                if chip == "esp32" {
                    0x1000
                } else {
                    0
                }
            }
            "partitions.bin" => 0x8000,
            "boot_app0.bin" => 0xe000,
            "app.bin" => {
                app = true;
                0x10000
            }
            _ => return Err("Unknown firmware part; refusing an unverified flash layout".into()),
        };
        let offset = u64::from_str_radix(part["offset"].as_str().unwrap_or("").trim_start_matches("0x"), 16).ok();
        let limit = match name {
            "bootloader.bin" => 0x8000 - expected,
            "partitions.bin" => 0x1000,
            "boot_app0.bin" => 0x2000,
            _ => 0x330000,
        };
        if offset != Some(expected) || !seen.insert(name) || part["size"].as_u64().is_none_or(|n| n == 0 || n > limit) {
            return Err("Firmware layout does not match this board; refusing update".into());
        }
    }
    if !app {
        return Err("Firmware bundle has no app image".into());
    }
    Ok(())
}

pub fn compatible_bundle<'a>(bundles: &'a [Value], board: &str) -> Result<&'a Value, String> {
    board_chip(board)?;
    bundles
        .iter()
        .find(|fw| check_board(fw, board).is_ok())
        .ok_or_else(|| format!("No compatible firmware bundle for {board}; this update is unavailable"))
}

/// ESP image header includes the silicon id at byte 12, independent of filenames.
pub fn check_image_board(data: &[u8], board: &str) -> Result<(), String> {
    let (_, chip) = board_chip(board)?;
    if data.len() < 24 || data[0] != 0xe9 || u16::from_le_bytes([data[12], data[13]]) != chip {
        return Err("Firmware image targets a different chip or has an invalid header; refusing update".into());
    }
    Ok(())
}

fn version_key(v: &str) -> Vec<u64> {
    ota::version_tuple(v)
}

/// Every usable bundle, best first.
pub fn bundles(root: &Path, extra: Option<&Path>) -> Vec<Value> {
    let mut found: Vec<(Value, u8)> = Vec::new();
    for d in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = d.file_name().to_string_lossy().to_string();
        let (source, rank) = if name == "local" {
            ("local", 2)
        } else if !version_key(&name).is_empty() {
            ("release", 1)
        } else {
            continue;
        };
        found.extend(load_bundle(&d.path(), source).map(|b| (b, rank)));
        // A version can contain separate bundles for multiple supported boards.
        for board in ["m5-sticks3", "m5-stickc-plus2"] {
            found.extend(load_bundle(&d.path().join(board), source).map(|b| (b, rank)));
        }
    }
    if let Some(e) = extra {
        found.extend(load_bundle(e, "local").map(|b| (b, 3)));
    }
    found.sort_by(|(a, ra), (b, rb)| {
        let va = version_key(a["version"].as_str().unwrap_or(""));
        let vb = version_key(b["version"].as_str().unwrap_or(""));
        (vb, rb).cmp(&(va, ra))
    });
    found.into_iter().map(|(b, _)| b).collect()
}

/// The app image of the best bundle (ota.py `bundled_app`), checked against its manifest.
pub fn bundled_app(fw: Option<&Value>) -> Result<Image, String> {
    let fw = fw.ok_or("no firmware yet — put a bundle in ~/.stickybot-rs/firmware/local (make -C m5-stick-s3 release, then copy it) or set STICKYBOT_RS_FIRMWARE")?;
    let board = fw["board"].as_str().unwrap_or("");
    check_board(fw, board)?;
    let part = fw["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["offset"] == APP_OFFSET || p["file"] == "app.bin")
        .ok_or("the bundled firmware has no app image")?;
    let path = fw["files"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f[0] == part["offset"])
        .and_then(|f| f[1].as_str())
        .ok_or("the bundled firmware has no app image")?;
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let sha: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    if part["sha256"].as_str().is_some_and(|s| !s.is_empty() && s != sha) {
        return Err("the bundled app image doesn't match its manifest (sha256) — refusing".into());
    }
    if part["size"].as_u64().is_some_and(|s| s != data.len() as u64) {
        return Err("the bundled app image doesn't match its manifest (size) — refusing".into());
    }
    if data.first() != Some(&0xE9) {
        return Err("the bundled app.bin isn't an ESP32 app image — refusing".into());
    }
    check_image_board(&data, board)?;
    // signing: not implemented yet (ota.py verify_signature); the sha256 + the bonded link is all
    Ok(Image::new(data, fw["version"].as_str().unwrap_or("?")))
}

/// A bundle as `GET /api/firmware` shows it.
pub fn describe(fw: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for k in ["version", "board", "built", "sha256", "size", "source", "release"] {
        out.insert(k.into(), fw.get(k).cloned().unwrap_or(Value::Null));
    }
    Value::Object(out)
}

// --- jobs ----------------------------------------------------------------------------------------------

/// One update, its events for the page (`{"line"}`, `{"progress","stage"}`, `{"done","ok","error"}`).
pub struct Job {
    pub id: String,
    events: Mutex<Vec<Value>>,
    done: Mutex<bool>,
    pub changed: Notify,
    cancel: Mutex<Option<CancelHandle>>,
}

impl Job {
    pub(crate) fn new() -> Arc<Self> {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).expect("system randomness");
        Arc::new(Self {
            id: b.iter().map(|x| format!("{x:02x}")).collect(),
            events: Mutex::new(Vec::new()),
            done: Mutex::new(false),
            changed: Notify::new(),
            cancel: Mutex::new(None),
        })
    }

    pub fn emit(&self, e: Value) {
        self.events.lock().unwrap().push(e);
        self.changed.notify_waiters();
    }

    pub(crate) fn finish(&self, e: Value) {
        self.events.lock().unwrap().push(e);
        *self.done.lock().unwrap() = true;
        self.changed.notify_waiters();
    }

    pub fn is_done(&self) -> bool {
        *self.done.lock().unwrap()
    }

    /// Events from `i` on.
    pub fn events_from(&self, i: usize) -> Vec<Value> {
        self.events.lock().unwrap().get(i..).map(<[Value]>::to_vec).unwrap_or_default()
    }

    /// What Cancel stops (a Bluetooth transfer), or nothing.
    pub(crate) fn set_cancel(&self, h: Option<CancelHandle>) {
        *self.cancel.lock().unwrap() = h;
    }

    /// Stops a running Bluetooth update; false if there's nothing to stop.
    pub fn cancel(&self) -> bool {
        if self.is_done() {
            return false;
        }
        match self.cancel.lock().unwrap().as_ref() {
            Some(c) => {
                c.cancel();
                true
            }
            None => false,
        }
    }
}

#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl Jobs {
    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap().get(id).cloned()
    }
    pub fn busy(&self) -> bool {
        self.jobs.lock().unwrap().values().any(|j| !j.is_done())
    }
    pub(crate) fn add(&self, j: Arc<Job>) {
        self.jobs.lock().unwrap().insert(j.id.clone(), j);
    }
}

// --- the update ----------------------------------------------------------------------------------------

/// How long things take (tests shorten them).
#[derive(Debug, Clone)]
pub struct UpdateTimings {
    pub transfer: Timings,
    pub reboot: Duration,
    pub poll: Duration,
}

impl Default for UpdateTimings {
    fn default() -> Self {
        Self { transfer: Timings::default(), reboot: REBOOT, poll: Duration::from_secs(2) }
    }
}

/// The checks before anything is sent (ota.py `update`, first half): the link, the image, the
/// stick's support, its pending state and the versions.
pub async fn preflight(link: &Link<Backend>, img: &Image, force: bool) -> Result<Value, String> {
    if !link.connected() {
        return Err("the stick isn't connected by Bluetooth".into());
    }
    let st = link.request("@state").await.ok_or("the stick didn't answer (@state)")?;
    if !ota::supported(&st) {
        let fw = st.get("fw").and_then(Value::as_str).unwrap_or("?");
        return Err(format!(
            "the stick's firmware {fw} can't update over Bluetooth — update it once over USB (1.8.0 or newer)"
        ));
    }
    check_image_board(&img.data, st["board"].as_str().unwrap_or(""))?;
    if st.get("otaPending").and_then(Value::as_bool) == Some(true) {
        return Err(ota::PENDING.into());
    }
    if let Some(why) = ota::check_versions(&img.version, st.get("fw").and_then(Value::as_str), force) {
        return Err(why);
    }
    Ok(st)
}

/// The whole update: checks, the transfer, then waits for the stick to come back with the new
/// version (ota.py `update`). Returns a summary.
pub async fn update(
    link: Link<Backend>,
    img: Image,
    force: bool,
    job: Arc<Job>,
    t: UpdateTimings,
) -> Result<Value, String> {
    let st = preflight(&link, &img, force).await?;
    let from = st.get("fw").and_then(Value::as_str).unwrap_or("").to_string();
    let name = st.get("name").and_then(Value::as_str).unwrap_or("the stick").to_string();
    job.emit(json!({"line": format!("updating {name} from {from} to {} over Bluetooth", img.version)}));
    let j = job.clone();
    let mut sender = Sender::new(link.clone(), Arc::new(move |e| j.emit(e)));
    sender.timings = t.transfer.clone();
    *job.cancel.lock().unwrap() = Some(sender.cancel_handle());
    let stats = sender.run(&img, force).await.map_err(|e| match e {
        OtaError::Failed(m) | OtaError::Cancelled(m) => m,
    })?;
    *job.cancel.lock().unwrap() = None;
    job.emit(json!({"progress": 0, "stage": "reboot"}));
    job.emit(json!({"line": "the stick is restarting with the new firmware"}));
    let mut result = stats.to_json();
    result["from"] = json!(from);
    result["to"] = json!(img.version);
    result["version"] = Value::Null;
    result["confirmed"] = json!(false);
    let want = ota::version_tuple(&img.version);
    let start = tokio::time::Instant::now();
    let deadline = start + t.reboot;
    tokio::time::sleep(t.poll).await;
    while tokio::time::Instant::now() < deadline {
        if link.connected()
            && let Some(now) = link.request("@state").await
        {
            let fw = now.get("fw").and_then(Value::as_str).unwrap_or("");
            if ota::version_tuple(fw) == want {
                result["version"] = json!(fw);
                if now.get("otaPending").and_then(Value::as_bool) != Some(true) {
                    result["confirmed"] = json!(true);
                    break;
                }
            } else if !fw.is_empty() && fw == from && tokio::time::Instant::now() > start + t.reboot / 2 {
                result["version"] = json!(fw);
                break;
            }
        }
        tokio::time::sleep(t.poll).await;
    }
    job.emit(json!({"progress": 100, "stage": "reboot"}));
    let version = result["version"].as_str().map(str::to_string);
    if result["confirmed"] == true {
        job.emit(json!({"line": format!("done — the stick runs {} (confirmed; its settings and pairings are kept)", version.unwrap_or_default())}));
    } else if version.as_deref() == Some(img.version.as_str()) {
        job.emit(json!({"line": format!("the stick runs {}; it confirms the new firmware after ~25 s of running", img.version)}));
    } else if let Some(v) = version {
        return Err(format!(
            "the stick came back with {v} — the new firmware didn't start and it rolled back (nothing lost)"
        ));
    } else {
        job.emit(json!({"line": "the stick hasn't reconnected yet — it should be back in a few seconds"}));
    }
    Ok(result)
}

/// `POST /api/firmware/ota`: starts a job, or says why not (status code, error).
pub fn start_ota(st: &Arc<AppState>, force: bool, t: UpdateTimings) -> Result<String, (u16, String)> {
    if st.jobs.busy() {
        return Err((409, "a flash is already running".into()));
    }
    if !st.link.connected() {
        return Err((409, "the stick isn't connected by Bluetooth".into()));
    }
    if st.faces.busy() {
        // one transfer at a time over the OTA characteristic
        return Err((409, "a face is being installed; try again when it's done".into()));
    }
    let bundles = st.firmware();
    if bundles.is_empty() {
        return Err((400, "No compatible firmware bundle is available".into()));
    }
    let job = Job::new();
    st.jobs.add(job.clone());
    let (link, j) = (st.link.clone(), job.clone());
    tokio::spawn(async move {
        let r = async {
            let state = link.request("@state").await.ok_or("The stick did not identify its model")?;
            let fw = compatible_bundle(&bundles, state["board"].as_str().unwrap_or(""))?;
            let img = bundled_app(Some(fw))?;
            update(link, img, force, j.clone(), t).await
        }
        .await;
        match &r {
            Ok(_) => log::info!("bluetooth update: ok"),
            Err(e) => log::info!("bluetooth update: {e}"),
        }
        let (ok, error, result) = match r {
            Ok(v) => (true, Value::Null, v),
            Err(e) => (false, json!(e), Value::Null),
        };
        j.finish(json!({"done": true, "ok": ok, "error": error, "result": result}));
    });
    Ok(job.id.clone())
}

/// `GET /api/firmware`'s `connected`: the Bluetooth-linked stick's name, version and OTA support.
pub async fn connected(link: &Link<Backend>) -> Value {
    if !link.connected() {
        return Value::Null;
    }
    match link.request("@state").await {
        Some(st) => {
            json!({"name": st.get("name"), "version": st.get("fw"), "ota": ota::supported(&st), "board": st.get("board"), "deviceId": st.get("deviceId")})
        }
        None => Value::Null,
    }
}

pub fn firmware_root(home: &Path) -> PathBuf {
    home.join("firmware")
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn make_bundle(dir: &Path, version: &str, app: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("app.bin"), app).unwrap();
        std::fs::write(dir.join("bootloader.bin"), b"boot").unwrap();
        let sha: String = Sha256::digest(app).iter().map(|b| format!("{b:02x}")).collect();
        let m = json!({"version": version, "board": "m5stick-s3", "parts": [
            {"file": "bootloader.bin", "offset": "0x0", "size": 4},
            {"file": "app.bin", "offset": "0x10000", "size": app.len(), "sha256": sha}]});
        std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stickybot-fw-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn app(n: usize) -> Vec<u8> {
        let mut d = vec![7u8; n.max(24)];
        d[0] = 0xE9;
        d[12] = 9;
        d[13] = 0;
        d
    }

    #[test]
    fn bundled_app_uses_only_the_app_part() {
        let root = tmp("app");
        make_bundle(&root.join("1.8.1"), "1.8.1", &app(3000));
        let b = bundles(&root, None);
        let a = bundled_app(b.first()).unwrap();
        assert_eq!((a.size(), a.version.as_str()), (3000, "1.8.1"));
        assert_eq!(describe(&b[0])["source"], "release");
    }

    #[test]
    fn the_best_bundle_wins() {
        let root = tmp("best");
        make_bundle(&root.join("1.8.1"), "1.8.1", &app(10));
        make_bundle(&root.join("1.9.0"), "1.9.0", &app(10));
        make_bundle(&root.join("local"), "1.9.0", &app(10));
        std::fs::create_dir_all(root.join("junk")).unwrap();
        let b = bundles(&root, None);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0]["version"].as_str(), b[0]["source"].as_str()), (Some("1.9.0"), Some("local")));
        assert_eq!(b[2]["version"], "1.8.1");
    }

    #[test]
    fn bundled_app_refuses_a_tampered_image() {
        let root = tmp("tamper");
        make_bundle(&root.join("1.8.1"), "1.8.1", &app(3000));
        let mut bad = app(3000);
        bad[5] = 1;
        std::fs::write(root.join("1.8.1/app.bin"), bad).unwrap();
        let e = bundled_app(bundles(&root, None).first()).unwrap_err();
        assert!(e.contains("sha256"), "{e}");
    }

    #[test]
    fn a_missing_part_or_bundle() {
        let root = tmp("missing");
        make_bundle(&root.join("1.8.1"), "1.8.1", &app(10));
        std::fs::remove_file(root.join("1.8.1/bootloader.bin")).unwrap();
        assert!(bundles(&root, None).is_empty());
        assert!(bundled_app(None).unwrap_err().contains("no firmware"));
    }
    #[test]
    fn bundles_are_selected_by_board_not_only_version() {
        let root = tmp("boards");
        make_bundle(&root.join("9.0.0"), "9.0.0", &app(40));
        make_bundle(&root.join("1.0.0"), "1.0.0", &app(40));
        let mut m: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("1.0.0/manifest.json")).unwrap()).unwrap();
        m["board"] = json!("m5-stickc-plus2");
        m["parts"][0]["offset"] = json!("0x1000");
        std::fs::write(root.join("1.0.0/manifest.json"), m.to_string()).unwrap();
        let all = bundles(&root, None);
        assert_eq!(compatible_bundle(&all, "m5-stickc-plus2").unwrap()["version"], "1.0.0");
        assert_eq!(compatible_bundle(&all, "m5-sticks3").unwrap()["version"], "9.0.0");
        assert!(compatible_bundle(&all, "unknown").unwrap_err().contains("unknown"));
        assert!(compatible_bundle(&all[..1], "m5-stickc-plus2").unwrap_err().contains("unavailable"));
        // An image mislabeled as Plus2 is rejected even with a valid checksum.
        assert!(bundled_app(Some(&all[1])).unwrap_err().contains("different chip"));
    }

    #[test]
    fn board_chip_and_layout_are_checked_independently() {
        let root = tmp("layout");
        make_bundle(&root, "1.0.0", &app(40));
        let mut fw = load_bundle(&root, "local").unwrap();
        assert!(check_board(&fw, "m5-sticks3").is_ok());
        fw["flash"] = json!({"chip": "esp32"});
        assert!(check_board(&fw, "m5-sticks3").unwrap_err().contains("chip"));
        fw["flash"]["chip"] = json!("esp32s3");
        fw["parts"][1]["offset"] = json!("0x9000");
        assert!(check_board(&fw, "m5-sticks3").unwrap_err().contains("layout"));
        assert!(check_image_board(&[0xe9], "m5-sticks3").is_err());
        assert!(check_image_board(&app(40), "m5-stickc-plus2").is_err());
        let mut plus = app(40);
        plus[12] = 0;
        assert!(check_image_board(&plus, "m5-stickc-plus2").is_ok());
    }
}
