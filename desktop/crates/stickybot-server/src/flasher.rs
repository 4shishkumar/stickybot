//! Flashing the stick over USB with esptool (a port of `flasher.py` + firmware_api.py's `flash`).
//!
//! The parts of the best bundle (`firmware.rs`) are written at their own offsets, so the stick's
//! settings storage (NVS at 0x9000-0xDFFF: pairings, combo, name, Wi-Fi, sounds) is never erased.
//! Steps: (backup) -> reset into the ROM bootloader with the 1200-baud "touch" on the stick's USB
//! port -> wait for the bootloader's port -> write + verify -> hard reset. esptool runs as a
//! subprocess (an [`Esptool`] runner, so tests never run it); its output is streamed line by line,
//! progress (`NN %`) as `{"progress","stage"}` events.
//!
//! esptool itself: `$STICKYBOT_RS_ESPTOOL`, else `esptool` / `esptool.py` on PATH or in Homebrew
//! (`brew install esptool`), else `python3 -m esptool` (`pip install esptool`).

use crate::AppState;
use crate::firmware::Job;
use crate::usb::Usb;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// the stick's settings: never write here (default_8MB.csv)
pub const NVS: (u64, u64) = (0x9000, 0xE000);
pub const SECTOR: u64 = 0x1000;
pub const NO_FIRMWARE: &str = "no firmware yet — no release downloaded (Settings → Updates → Check now) and no local build (make -C m5-stick-s3 release)";

/// Runs esptool with `args`, giving each output piece (split at `\r` / `\n`) to `line`; returns
/// the exit code.
pub trait Esptool: Send + Sync {
    fn run(&self, args: &[String], line: &mut dyn FnMut(&str)) -> Result<i32, String>;
}

/// The real esptool, found as the module doc says.
pub struct SystemEsptool;

fn find_esptool() -> Option<(PathBuf, Vec<String>)> {
    if let Some(p) = std::env::var_os("STICKYBOT_RS_ESPTOOL").filter(|p| !p.is_empty()) {
        return Some((PathBuf::from(p), vec![]));
    }
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
        .collect();
    for d in &dirs {
        for name in ["esptool", "esptool.py"] {
            if d.join(name).is_file() {
                return Some((d.join(name), vec![]));
            }
        }
    }
    for d in &dirs {
        let py = d.join("python3");
        if py.is_file()
            && Command::new(&py)
                .args(["-c", "import esptool"])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        {
            return Some((py, vec!["-m".into(), "esptool".into()]));
        }
    }
    None
}

impl Esptool for SystemEsptool {
    fn run(&self, args: &[String], line: &mut dyn FnMut(&str)) -> Result<i32, String> {
        let (prog, pre) =
            find_esptool().ok_or("esptool not found — install it (brew install esptool, or pip install esptool)")?;
        let (mut reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&prog);
        cmd.args(&pre).args(args).stdin(Stdio::null());
        cmd.stdout(writer.try_clone().map_err(|e| e.to_string())?).stderr(writer);
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", prog.display()))?;
        drop(cmd); // closes our copies of the pipe's write end: EOF when esptool exits
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            // esptool redraws progress with \r: treat \r and \n as line ends
            while let Some(i) = buf.iter().position(|b| *b == b'\r' || *b == b'\n') {
                let piece: Vec<u8> = buf.drain(..=i).collect();
                line(String::from_utf8_lossy(&piece).trim());
            }
        }
        if !buf.is_empty() {
            line(String::from_utf8_lossy(&buf).trim());
        }
        child.wait().map(|s| s.code().unwrap_or(-1)).map_err(|e| e.to_string())
    }
}

/// `NN %` / `NN.N%` in an esptool line.
pub fn progress(line: &str) -> Option<u32> {
    let i = line.find('%')?;
    let num: String = line[..i].trim_end().chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let num: String = num.chars().rev().collect();
    let v: f64 = num.parse().ok()?;
    (v <= 100.0 || num.len() <= 3).then_some((v as u32).min(100))
}

/// Refuses any part whose erase range (whole 4 KB sectors) overlaps NVS.
pub fn check_parts(parts: &[(String, PathBuf)]) -> Result<(), String> {
    for (offset, path) in parts {
        let start =
            u64::from_str_radix(offset.trim_start_matches("0x"), 16).map_err(|_| format!("bad offset {offset}"))?;
        let len = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
        let (first, last) = (start / SECTOR * SECTOR, (start + len).div_ceil(SECTOR) * SECTOR);
        if first < NVS.1 && last > NVS.0 {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return Err(format!("{name} at {offset} would overwrite the stick's settings (NVS) — refusing"));
        }
    }
    Ok(())
}

/// A bundle's `files` as `[(offset, path)]`.
pub fn parts_of(fw: &Value) -> Vec<(String, PathBuf)> {
    fw["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| Some((f[0].as_str()?.to_string(), PathBuf::from(f[1].as_str()?))))
        .collect()
}

/// How long the steps wait (tests shorten them).
#[derive(Debug, Clone)]
pub struct FlashTimings {
    pub settle: Duration,
    pub poll: Duration,
    pub bootloader: Duration,
}

impl Default for FlashTimings {
    fn default() -> Self {
        Self {
            settle: Duration::from_millis(600),
            poll: Duration::from_millis(300),
            bootloader: Duration::from_secs(12),
        }
    }
}

/// After the touch the port disappears and the bootloader's comes back (maybe under a new name).
fn wait_for_bootloader(
    usb: &dyn Usb,
    old: &str,
    serial: &str,
    before: &[crate::usb::PortInfo],
    t: &FlashTimings,
) -> Result<String, String> {
    let deadline = Instant::now() + t.bootloader;
    std::thread::sleep(t.settle);
    while Instant::now() < deadline {
        let ports = usb.ports();
        let boot: Vec<_> = ports.iter().filter(|p| p.in_bootloader()).collect();
        if let Some(port) = boot.iter().find(|p| if serial.is_empty() { p.port == old } else { p.serial == serial }) {
            return Ok(port.port.clone());
        }
        // TinyUSB and ROM USB-Serial-JTAG need not share a serial. Only a unique
        // newly appeared bootloader is eligible; never pick one present before the touch.
        let new: Vec<_> =
            boot.iter().filter(|p| !before.iter().any(|b| b.in_bootloader() && b.port == p.port)).collect();
        let old_app_gone = !ports.iter().any(|p| p.port == old && !p.in_bootloader());
        if old_app_gone && boot.len() == 1 && new.len() == 1 {
            return Ok(new[0].port.clone());
        }
        std::thread::sleep(t.poll);
    }
    Err("the stick didn't come back in download mode — hold its side button until the green LED flashes (manual download mode) and try again".into())
}

fn run_esptool(tool: &dyn Esptool, args: Vec<String>, emit: &dyn Fn(Value), stage: &str) -> Result<(), String> {
    emit(json!({"line": format!("$ esptool {}", args.join(" "))}));
    let mut last = None;
    let code = tool.run(&args, &mut |l| {
        if l.is_empty() {
            return;
        }
        if let Some(p) = progress(l) {
            if last != Some(p) {
                last = Some(p);
                emit(json!({"progress": p, "stage": stage}));
            }
            return; // don't flood the log with progress redraws
        }
        emit(json!({"line": l}));
    })?;
    if code != 0 {
        return Err(format!("esptool failed (exit {code})"));
    }
    Ok(())
}

/// What a flash needs.
pub struct Flash<'a> {
    pub usb: &'a dyn Usb,
    pub esptool: &'a dyn Esptool,
    pub port: String,
    pub backup: bool,
    pub bundles: Vec<Value>,
    pub expected_device: Option<String>,
    pub recovery_board: Option<String>,
    pub backups: PathBuf,
    pub timings: FlashTimings,
}

/// Flashes the bundle to the stick on `port`, keeping its settings (flasher.py `flash`). Blocking.
pub fn flash(f: &Flash, emit: &dyn Fn(Value)) -> Result<(), String> {
    let ports_before = f.usb.ports();
    let info = ports_before.iter().find(|p| p.port == f.port).ok_or_else(|| not_a_stick(&f.port))?;
    // One explicit-port probe per operation; no API/start/discovery reopens it.
    let state = if info.in_bootloader() { None } else { f.usb.proto(&f.port, "@state", Duration::from_millis(1500)) };
    let board = if let Some(state) = &state {
        if let Some(expected) = &f.expected_device
            && state["deviceId"].as_str() != Some(expected.as_str())
        {
            return Err("USB port does not belong to the selected device".into());
        }
        let board = state["board"].as_str().unwrap_or("");
        if let Some(chosen) = &f.recovery_board
            && crate::firmware::canonical_board(board) != crate::firmware::canonical_board(chosen)
        {
            return Err("Selected recovery model does not match the running firmware".into());
        }
        board
    } else {
        f.recovery_board.as_deref().ok_or("Device model cannot be verified over USB. Choose its port and model under USB recovery, put it in download mode and try again.")?
    };
    let fw = crate::firmware::compatible_bundle(&f.bundles, board)?;
    let parts = parts_of(fw);
    if parts.is_empty() {
        return Err(NO_FIRMWARE.into());
    }
    check_parts(&parts)?;
    crate::firmware::check_board(fw, board)?;
    crate::firmware::bundled_app(Some(fw))?;
    // Check every part's hash, not only the app, before resetting the selected device.
    for part in fw["parts"].as_array().into_iter().flatten() {
        let path = parts
            .iter()
            .find(|(offset, _)| offset == part["offset"].as_str().unwrap_or(""))
            .map(|(_, path)| path)
            .ok_or("Missing firmware part")?;
        if part["file"] == "bootloader.bin" {
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            crate::firmware::check_image_board(&data, board)?;
        }
        if let Some(expected) = part["sha256"].as_str()
            && crate::releases::sha256_file(path).as_deref() != Some(expected)
        {
            return Err("Firmware part does not match its manifest; refusing update".into());
        }
    }
    let flash_cfg = &fw["flash"];
    let classic = crate::firmware::canonical_board(board) == Some("m5-stickc-plus2");
    let chip = if classic { "esp32" } else { "esp32s3" };
    let reset = if classic { "default-reset" } else { "usb-reset" };
    let cfg = |k: &str, d: &str| flash_cfg.get(k).and_then(Value::as_str).unwrap_or(d).to_string();

    // 1. into the bootloader
    emit(json!({"progress": 0, "stage": "reset"}));
    let boot = if classic {
        // Plus2's CH9102 bridge keeps the same port; esptool drives its reset pins.
        f.port.clone()
    } else if info.in_bootloader() {
        emit(json!({"line": format!("{} is already in download mode", f.port)}));
        f.port.clone()
    } else {
        emit(json!({"line": format!("resetting {} into download mode (1200-baud touch)", f.port)}));
        f.usb.touch_1200(&f.port)?;
        let b = wait_for_bootloader(f.usb, &f.port, &info.serial, &ports_before, &f.timings)?;
        emit(json!({"line": format!("bootloader on {b}")}));
        b
    };
    emit(json!({"progress": 100, "stage": "reset"}));
    if state.is_none() {
        // The user chose the board; a ROM command verifies the corresponding silicon
        // before any flash write. No board identity is inferred from a generic USB PID.
        let args = vec![
            "--chip".into(),
            chip.into(),
            "--port".into(),
            boot.clone(),
            "--before".into(),
            if classic { "default-reset".into() } else { "no-reset".into() },
            "--after".into(),
            "no-reset".into(),
            "--no-stub".into(),
            "chip-id".into(),
        ];
        run_esptool(f.esptool, args, emit, "identify")?;
    }
    let base: Vec<String> =
        vec!["--chip".into(), cfg("chip", chip), "--port".into(), boot, "--baud".into(), "921600".into()];

    // 2. optional backup of the whole flash
    if f.backup {
        let name = if info.serial.is_empty() { "stick".to_string() } else { info.serial.replace('/', "_") };
        let out = f.backups.join(format!("{name}-{}.bin", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        std::fs::create_dir_all(&f.backups).map_err(|e| format!("{}: {e}", f.backups.display()))?;
        emit(json!({"line": format!("backing up the stick's flash to {}", out.display())}));
        let mut a = base.clone();
        a.extend(["--before", reset, "--after", "no-reset", "read-flash", "0x0", "0x800000"].map(String::from));
        a.push(out.to_string_lossy().into_owned());
        run_esptool(f.esptool, a, emit, "backup")?;
    }

    // 3. write + verify (esptool checks the written data's hash), then reboot. "usb-reset" (like
    // PlatformIO): after the touch the chip is held in download mode, and only esptool's USB reset
    // lets the final hard reset start the new firmware.
    let mut a = base;
    a.extend(["--before", reset, "--after", "hard-reset", "write-flash"].map(String::from));
    a.extend(["--flash-mode".into(), cfg("mode", "dio"), "--flash-freq".into(), cfg("freq", "80m")]);
    a.extend(["--flash-size".into(), cfg("size", "8MB")]);
    for (off, path) in &parts {
        a.push(off.clone());
        a.push(path.to_string_lossy().into_owned());
    }
    run_esptool(f.esptool, a, emit, "flash")?;
    emit(json!({"progress": 100, "stage": "verify"}));
    emit(json!({"progress": 100, "stage": "reboot"}));
    emit(json!({"line": "done — the stick restarts with the new firmware (its settings and pairings are kept)"}));
    Ok(())
}

fn not_a_stick(port: &str) -> String {
    format!("{port} isn't an Espressif USB device (not a stick) — refusing")
}

/// `POST /api/firmware/flash` `{"port","backup"}`: starts a job, or says why not.
pub fn start(
    st: &Arc<AppState>,
    port: &str,
    backup: bool,
    recovery_board: Option<String>,
) -> Result<String, (u16, String)> {
    let usb = st.usb.backend();
    if !usb.ports().iter().any(|p| p.port == port) {
        return Err((400, not_a_stick(port)));
    }
    if st.jobs.busy() {
        return Err((409, "a flash is already running".into()));
    }
    if recovery_board.as_deref().is_some_and(|b| crate::firmware::canonical_board(b).is_none()) {
        return Err((400, "Choose a supported recovery model".into()));
    }
    let expected_device = st.host_state.lock().unwrap()["deviceId"].as_str().map(str::to_owned);
    if recovery_board.is_none()
        && expected_device.is_none()
        && (st.device_id != "primary" || st.devices.all().len() > 1)
    {
        return Err((409, "Selected device's USB identity is unknown; use explicit USB recovery".into()));
    }
    let bundles = st.firmware();
    if bundles.is_empty() {
        return Err((409, NO_FIRMWARE.to_string()));
    }
    // The shared USB controller has one flashing mode; reserve it before spawning so
    // simultaneous requests from different device sessions cannot clear each other's guard.
    if st
        .usb
        .flashing
        .compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
        .is_err()
    {
        return Err((409, "another USB flash is already running".into()));
    }
    let job = Job::new();
    st.jobs.add(job.clone());
    let (st2, j, port) = (st.clone(), job.clone(), port.to_string());
    let tool = st.esptool();
    let timings = st.flash_timings.lock().unwrap().clone();
    tokio::spawn(async move {
        let (j2, p2, backups) = (j.clone(), port.clone(), st2.paths.home.join("backups"));
        let r = tokio::task::spawn_blocking(move || {
            let f = Flash {
                usb: &*usb,
                esptool: &*tool,
                port: p2,
                backup,
                bundles,
                expected_device,
                recovery_board,
                backups,
                timings,
            };
            flash(&f, &|e| j2.emit(e))
        })
        .await
        .unwrap_or_else(|e| Err(format!("unexpected error: {e}")));
        st2.usb.flashing.store(false, std::sync::atomic::Ordering::Relaxed);
        st2.usb.forget();
        match &r {
            Ok(()) => log::info!("flash {port}: ok"),
            Err(e) => log::info!("flash {port}: {e}"),
        }
        let (ok, error) = match r {
            Ok(()) => (true, Value::Null),
            Err(e) => (false, json!(e)),
        };
        j.finish(json!({"done": true, "ok": ok, "error": error}));
    });
    Ok(job.id.clone())
}

pub fn backups_dir(home: &Path) -> PathBuf {
    home.join("backups")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::usb::FakeUsb;
    use std::sync::Mutex;

    /// esptool in memory: records the arguments, prints progress, exits with `code`.
    #[derive(Default)]
    pub struct FakeEsptool {
        pub runs: Mutex<Vec<Vec<String>>>,
        pub code: i32,
        pub reject_identify: bool,
    }

    impl Esptool for FakeEsptool {
        fn run(&self, args: &[String], line: &mut dyn FnMut(&str)) -> Result<i32, String> {
            self.runs.lock().unwrap().push(args.to_vec());
            line("esptool v5");
            if args.last().is_some_and(|a| a == "chip-id") {
                return Ok(if self.reject_identify { 2 } else { self.code });
            }
            for p in [
                "Writing at 0x00010000... (10 %)",
                "Writing at 0x00020000... (10 %)",
                "Writing at 0x00090000... (100 %)",
            ] {
                line(p);
            }
            line("Hash of data verified.");
            Ok(self.code)
        }
    }

    fn bundle(name: &str, app_offset: &str) -> Value {
        let d = std::env::temp_dir().join(format!("stickybot-flash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut boot = vec![0u8; 100];
        boot[0] = 0xe9;
        boot[12] = 9;
        std::fs::write(d.join("bootloader.bin"), boot).unwrap();
        let mut app = vec![0u8; 5000];
        app[0] = 0xe9;
        app[12] = 9;
        std::fs::write(d.join("app.bin"), app).unwrap();
        let m = json!({"version": "2.3.0", "board": "m5-sticks3", "flash": {"mode": "qio", "chip": "esp32s3"}, "parts": [
            {"file": "bootloader.bin", "offset": "0x0", "size": 100},
            {"file": "app.bin", "offset": app_offset, "size": 5000}]});
        std::fs::write(d.join("manifest.json"), m.to_string()).unwrap();
        crate::firmware::load_bundle(&d, "local").unwrap()
    }

    fn quick() -> FlashTimings {
        FlashTimings { settle: Duration::ZERO, poll: Duration::from_millis(1), bootloader: Duration::from_millis(50) }
    }

    #[test]
    fn progress_is_parsed() {
        assert_eq!(progress("Writing at 0x00010000... (37 %)"), Some(37));
        assert_eq!(progress("Writing at 0x0001 [====>   ]  42.5% 123/456 bytes"), Some(42));
        assert_eq!(progress("Hash of data verified."), None);
    }

    #[test]
    fn nvs_is_never_written() {
        let fw = bundle("nvs", "0x9000");
        let e = check_parts(&parts_of(&fw)).unwrap_err();
        assert!(e.contains("NVS"), "{e}");
        // the bootloader at 0x0 (100 bytes) and the app at 0x10000 are fine
        assert!(check_parts(&parts_of(&bundle("ok", "0x10000"))).is_ok());
    }

    #[test]
    fn touch_bootloader_backup_write() {
        let usb = FakeUsb::stick("/dev/cu.usbmodem1", "2.2.0");
        *usb.boot_port.lock().unwrap() = Some("/dev/cu.usbmodem2".into());
        let tool = FakeEsptool::default();
        let backups = std::env::temp_dir().join(format!("stickybot-flash-bk-{}", std::process::id()));
        let f = Flash {
            usb: &*usb,
            esptool: &tool,
            port: "/dev/cu.usbmodem1".into(),
            backup: true,
            bundles: vec![bundle("all", "0x10000")],
            expected_device: None,
            recovery_board: None,
            backups: backups.clone(),
            timings: quick(),
        };
        let events = Mutex::new(Vec::new());
        flash(&f, &|e| events.lock().unwrap().push(e)).unwrap();
        assert_eq!(*usb.touches.lock().unwrap(), vec!["/dev/cu.usbmodem1".to_string()]);
        let runs = tool.runs.lock().unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs[0].join(" ").contains(
            "--port /dev/cu.usbmodem2 --baud 921600 --before usb-reset --after no-reset read-flash 0x0 0x800000"
        ));
        assert!(runs[0].last().unwrap().starts_with(&backups.join("SN1-").to_string_lossy().into_owned()));
        let w = runs[1].join(" ");
        assert!(
            w.contains("--after hard-reset write-flash --flash-mode qio --flash-freq 80m --flash-size 8MB 0x0 "),
            "{w}"
        );
        assert!(w.contains(" 0x10000 ") && w.ends_with("app.bin"), "{w}");
        let ev = events.lock().unwrap();
        // progress once per value, no flood
        let flash_pct: Vec<u64> =
            ev.iter().filter(|e| e["stage"] == "flash").filter_map(|e| e["progress"].as_u64()).collect();
        assert_eq!(flash_pct, [10, 100]);
        assert!(ev.iter().any(|e| e["line"] == "Hash of data verified."));
        assert!(ev.last().unwrap()["line"].as_str().unwrap().starts_with("done"));
    }

    #[test]
    fn failures_say_why() {
        let usb = FakeUsb::stick("/dev/cu.usbmodem1", "2.2.0");
        let tool = FakeEsptool { code: 2, ..Default::default() };
        let mut f = Flash {
            usb: &*usb,
            esptool: &tool,
            port: "/dev/cu.nope".into(),
            backup: false,
            bundles: vec![bundle("fail", "0x10000")],
            expected_device: None,
            recovery_board: None,
            backups: std::env::temp_dir(),
            timings: quick(),
        };
        assert!(flash(&f, &|_| {}).unwrap_err().contains("isn't an Espressif"));
        f.port = "/dev/cu.usbmodem1".into();
        assert_eq!(flash(&f, &|_| {}).unwrap_err(), "esptool failed (exit 2)");
        // the bootloader never shows up
        let usb2 = FakeUsb::stick("/dev/cu.usbmodem1", "2.2.0");
        *usb2.boot_port.lock().unwrap() = None;
        f.usb = &*usb2;
        assert!(flash(&f, &|_| {}).unwrap_err().contains("download mode"));
    }
    #[test]
    fn wrong_board_and_unknown_device_never_reset_or_write() {
        let usb = FakeUsb::stick("/dev/cu.fake", "2.2.0");
        let tool = FakeEsptool::default();
        let f = Flash {
            usb: &*usb,
            esptool: &tool,
            port: "/dev/cu.fake".into(),
            backup: false,
            bundles: vec![bundle("wrong-board", "0x10000")],
            expected_device: None,
            recovery_board: None,
            backups: std::env::temp_dir(),
            timings: quick(),
        };
        usb.states.lock().unwrap().get_mut("/dev/cu.fake").unwrap()["board"] = json!("m5-stickc-plus2");
        assert!(flash(&f, &|_| {}).unwrap_err().contains("compatible"));
        usb.states.lock().unwrap().clear();
        assert!(flash(&f, &|_| {}).unwrap_err().contains("verified"));
        assert!(usb.touches.lock().unwrap().is_empty());
        assert!(tool.runs.lock().unwrap().is_empty());
    }

    #[test]
    fn bootloader_search_never_switches_to_another_stick() {
        let usb = FakeUsb::stick("/dev/cu.other", "2.2.0");
        usb.touch_1200("/dev/cu.other").unwrap();
        assert!(wait_for_bootloader(&*usb, "/dev/cu.selected", "SN2", &usb.ports(), &quick()).is_err());
        assert!(wait_for_bootloader(&*usb, "/dev/cu.other", "SN2", &usb.ports(), &quick()).is_err());
        assert_eq!(wait_for_bootloader(&*usb, "/dev/cu.old", "SN1", &usb.ports(), &quick()).unwrap(), "/dev/cu.other");
    }
    #[test]
    fn changed_rom_serial_uses_only_a_unique_new_bootloader() {
        let usb = FakeUsb::stick("/dev/cu.app", "2.2.0");
        *usb.boot_port.lock().unwrap() = Some("/dev/cu.rom".into());
        *usb.boot_serial.lock().unwrap() = Some("ROM-DIFFERENT".into());
        let before = usb.ports();
        usb.touch_1200("/dev/cu.app").unwrap();
        assert_eq!(wait_for_bootloader(&*usb, "/dev/cu.app", "SN1", &before, &quick()).unwrap(), "/dev/cu.rom");
        let rom = usb.ports()[0].clone();
        // A bootloader that already existed is never the serial-mismatch fallback.
        assert!(wait_for_bootloader(&*usb, "/dev/cu.app", "SN1", std::slice::from_ref(&rom), &quick()).is_err());
        let mut second = rom;
        second.port = "/dev/cu.second-rom".into();
        usb.ports.lock().unwrap().push(second);
        assert!(wait_for_bootloader(&*usb, "/dev/cu.app", "SN1", &before, &quick()).is_err());
        assert!(usb.lines.lock().unwrap().is_empty(), "enumeration never opens serial ports");
    }

    #[test]
    fn recovery_verifies_the_selected_rom_chip_before_writing_without_state() {
        for crashed_app in [false, true] {
            let usb = FakeUsb::stick("/dev/cu.recovery", "2.2.0");
            usb.states.lock().unwrap().clear();
            if !crashed_app {
                usb.touch_1200("/dev/cu.recovery").unwrap();
            }
            let tool = FakeEsptool::default();
            let mut f = Flash {
                usb: &*usb,
                esptool: &tool,
                port: "/dev/cu.recovery".into(),
                backup: false,
                bundles: vec![bundle(if crashed_app { "crashed" } else { "rom" }, "0x10000")],
                expected_device: Some("known-stick".into()),
                recovery_board: None,
                backups: std::env::temp_dir(),
                timings: quick(),
            };
            assert!(flash(&f, &|_| {}).unwrap_err().contains("recovery"));
            assert!(tool.runs.lock().unwrap().is_empty());
            f.recovery_board = Some("m5-sticks3".into());
            usb.lines.lock().unwrap().clear();
            flash(&f, &|_| {}).unwrap();
            let runs = tool.runs.lock().unwrap();
            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0].last().unwrap(), "chip-id");
            assert!(runs[0].join(" ").contains("--chip esp32s3 --port /dev/cu.recovery"));
            assert!(runs[0].join(" ").contains("--no-stub"));
            assert!(runs[1].iter().any(|a| a == "write-flash"));
            assert_eq!(usb.lines.lock().unwrap().len(), usize::from(crashed_app));
            drop(runs);
            let wrong = FakeEsptool { reject_identify: true, ..Default::default() };
            f.esptool = &wrong;
            assert!(flash(&f, &|_| {}).is_err());
            assert_eq!(wrong.runs.lock().unwrap().len(), 1);
            assert_eq!(wrong.runs.lock().unwrap()[0].last().unwrap(), "chip-id");
        }
    }

    #[test]
    fn plus2_uses_its_chip_offsets_and_bridge_reset() {
        let usb = FakeUsb::stick("/dev/cu.fakeplus", "2.2.0");
        usb.states.lock().unwrap().get_mut("/dev/cu.fakeplus").unwrap()["board"] = json!("m5-stickc-plus2");
        let mut fw = bundle("plus2", "0x10000");
        fw["board"] = json!("m5-stickc-plus2");
        fw["flash"]["chip"] = json!("esp32");
        fw["parts"][0]["offset"] = json!("0x1000");
        fw["files"][0][0] = json!("0x1000");
        let boot_path = fw["files"][0][1].as_str().unwrap();
        let mut boot = std::fs::read(boot_path).unwrap();
        boot[12] = 0;
        std::fs::write(boot_path, boot).unwrap();
        let path = fw["files"][1][1].as_str().unwrap();
        let mut app = std::fs::read(path).unwrap();
        app[12] = 0;
        std::fs::write(path, app).unwrap();
        let tool = FakeEsptool::default();
        let f = Flash {
            usb: &*usb,
            esptool: &tool,
            port: "/dev/cu.fakeplus".into(),
            backup: false,
            bundles: vec![fw],
            expected_device: None,
            recovery_board: None,
            backups: std::env::temp_dir(),
            timings: quick(),
        };
        flash(&f, &|_| {}).unwrap();
        assert!(usb.touches.lock().unwrap().is_empty());
        let args = tool.runs.lock().unwrap()[0].join(" ");
        assert!(args.contains("--chip esp32 --port /dev/cu.fakeplus"));
        assert!(args.contains("--before default-reset"));
        assert!(args.contains("8MB 0x1000"));
    }
}
