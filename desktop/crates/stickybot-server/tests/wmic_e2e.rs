//! The wireless Wispr mic, end to end with real Core Audio: macOS `say` speaks a sentence, it's
//! encoded with the stick's codec (IMA-ADPCM, 20 ms frames), the fake stick holds A like firmware
//! 1.7.1+ (`prepare`, frames in real time, the key on `ready`, the 250 ms tail, `stop`), and the
//! app plays it into a real process tap + aggregate device. This test reads that device's input
//! (like Wispr would) and compares it with what was said. Inaudible: the tap mutes this process,
//! and nothing is played before the -80 dBFS check proved the tap works.
//!
//! Opt-in (it creates a real, temporary aggregate + tap): `STICKYBOT_RS_AUDIO_E2E=1 cargo test -p
//! stickybot-server --test wmic_e2e -- --nocapture`. Needs the System Audio Recording permission for
//! the app that runs it (the terminal); without it the check fails and the test stops there.
//!
//! The device uses the Rust UID (`com.stickkey.rust-wireless-mic`) and a test name
//! (`stickybot-rs-e2e-mic`, not "<stick>-bt-mic", so Wispr's ranking doesn't switch to it). It
//! checks that nothing is left afterwards: the device and tap counts before and after match.
#![cfg(target_os = "macos")]

use cpal::traits::{DeviceTrait, StreamTrait};
use macos_audio::hal;
use macos_audio::naming::RUST_UID;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stick_link::fake::FakeStick;
use stick_link::{Backend, LinkState, adpcm};
use stickybot_server::{AppState, Options, wmic};

const NAME: &str = "stickybot-rs-e2e-mic";
const TEXT: &str = "This is the StickyBot wireless mic test, from the Rust app.";

fn ours() -> (Vec<String>, Vec<String>) {
    let devs =
        hal::devices().into_iter().filter(|d| d.uid.starts_with(RUST_UID)).map(|d| format!("{} {}", d.id, d.uid));
    let taps = hal::taps().into_iter().filter(|t| t.name.starts_with(RUST_UID)).map(|t| format!("{} {}", t.id, t.name));
    (devs.collect(), taps.collect())
}

fn counts() -> (usize, usize) {
    (hal::devices().len(), hal::taps().len())
}

/// Whatever happens (a failed assert), the device and tap this process made go.
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let n = macos_audio::mic::destroy_all_live();
        if n > 0 {
            eprintln!("guard: destroyed {n} device/tap object(s)");
        }
    }
}

/// `say` -> 16 kHz mono 16-bit samples.
fn say(text: &str, dir: &Path) -> Vec<i16> {
    let wav = dir.join("say.wav");
    let ok = std::process::Command::new("/usr/bin/say")
        .args(["-v", "Samantha", "-o"])
        .arg(&wav)
        .args(["--data-format=LEI16@16000", text])
        .status()
        .expect("macOS say");
    assert!(ok.success());
    let mut r = hound::WavReader::open(&wav).unwrap();
    assert_eq!((r.spec().sample_rate, r.spec().channels), (16000, 1));
    r.samples::<i16>().map(Result::unwrap).collect()
}

/// RMS per `block` samples.
fn envelope(x: &[f32], block: usize) -> Vec<f64> {
    x.chunks(block).map(|c| (c.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / c.len() as f64).sqrt()).collect()
}

fn dbfs(x: f64) -> f64 {
    if x <= 0.0 { f64::NEG_INFINITY } else { 20.0 * x.log10() }
}

/// Pearson correlation of two equally long series.
fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    let (mx, my) = (x[..n].iter().sum::<f64>() / n as f64, y[..n].iter().sum::<f64>() / n as f64);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (dx, dy) = (x[i] - mx, y[i] - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx > 0.0 && syy > 0.0 { sxy / (sxx * syy).sqrt() } else { 0.0 }
}

/// The best correlation of the heard envelope `b` against the said one `a`, `b` shifted by up to
/// `max_lag` blocks either way -> (r, lag in blocks; negative = the recording started after the
/// stream did).
fn best_correlation(a: &[f64], b: &[f64], max_lag: usize) -> (f64, i64) {
    let mut best = (f64::MIN, 0);
    for lag in -(max_lag as i64)..=max_lag as i64 {
        let (x, y) = if lag >= 0 {
            let l = lag as usize;
            if l >= b.len() {
                continue;
            }
            (a, &b[l..])
        } else {
            let l = (-lag) as usize;
            if l >= a.len() {
                continue;
            }
            (&a[l..], b)
        };
        if x.len().min(y.len()) < 50 {
            continue;
        }
        let r = pearson(x, y);
        if r > best.0 {
            best = (r, lag);
        }
    }
    best
}

/// Records the device's input (cpal, found by UID) from when it appears until `stop` is set.
/// -> (mono samples, rate).
fn record(stop: Arc<AtomicBool>) -> Result<(Vec<f32>, u32), String> {
    let t0 = Instant::now();
    while !hal::devices().iter().any(|d| d.uid == RUST_UID) {
        if t0.elapsed() > Duration::from_secs(3) {
            return Err("the device never appeared".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let host = cpal::default_host();
    let input = macos_audio::system::find_input(&host, RUST_UID)?;
    let cfg = macos_audio::system::input_config(&input, RUST_UID)?;
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let got = Arc::new(Mutex::new(Vec::<f32>::new()));
    let g = got.clone();
    let stream = input
        .build_input_stream::<f32, _, _>(
            cfg.config(),
            move |data, _| {
                if let Ok(mut v) = g.lock() {
                    v.extend(data.chunks(ch.max(1)).map(|f| f[0]));
                }
            },
            |e| eprintln!("input: {e}"),
            Some(Duration::from_secs(3)),
        )
        .map_err(|e| format!("can't open the device's input: {e}"))?;
    stream.play().map_err(|e| e.to_string())?;
    eprintln!(
        "recording the device's input ({ch} ch @ {rate} Hz), {:.0} ms after it appeared",
        t0.elapsed().as_secs_f64() * 1000.0
    );
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    let v = got.lock().unwrap().clone();
    Ok((v, rate))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sticks_audio_reaches_the_device() {
    if std::env::var("STICKYBOT_RS_AUDIO_E2E").as_deref() != Ok("1") {
        eprintln!("skipped: STICKYBOT_RS_AUDIO_E2E=1 runs it (a real, temporary Core Audio device + tap; inaudible)");
        return;
    }
    if macos_audio::tap::available().is_err() {
        eprintln!("skipped: no process taps on this Mac");
        return;
    }
    let (d0, t0) = ours();
    if !d0.is_empty() || !t0.is_empty() {
        eprintln!("skipped: the Rust UID's device / tap exists ({d0:?} {t0:?}): is the Rust app running a hold?");
        return;
    }
    let before = counts();
    let _guard = Guard;
    let home = std::env::temp_dir().join(format!("stickybot-rs-wmic-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let fake = FakeStick::new();
    let st = AppState::new(&Options { port: 8670, home: home.clone(), fake_stick: true }, Backend::Fake(fake.clone()))
        .unwrap();
    st.link.spawn();
    wmic::spawn(st.clone());
    let set = |v: Value| st.update_config(v.as_object().unwrap().clone()).unwrap();
    set(json!({"stick": {"address": "FAKE", "name": "StickyBot E2E"}, "wispr_mic_name": NAME}));
    let mut w = st.link.watch_state();
    tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected)).await.unwrap().unwrap();

    // 1. on, carried by the tap: the inaudible check runs first
    set(json!({"wireless_mic": true, "wireless_mic_carrier": "tap", "wispr_switch_ms": 250, "wispr_tail_ms": 800}));
    let t = Instant::now();
    while st.wmic.device().check.state.is_none() && t.elapsed() < Duration::from_secs(25) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let check = st.wmic.device().check;
    eprintln!(
        "tap check: {:?} in {:?} ms (create {:?} ms, tone after {:?} ms): {}",
        check.state, check.ms, check.create_ms, check.first_ms, check.detail
    );
    if check.state != Some(macos_audio::carrier::CheckState::Ok) {
        eprintln!("the tap isn't allowed for this terminal: nothing was played (System Audio Recording)");
    } else {
        for _ in 0..200 {
            if fake.wmic() == Some(true) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(fake.wmic(), Some(true), "the stick wasn't told to stream");

        // 2. the hold: 0.6 s before the speech (the user starts talking after the press)
        let mut pcm = vec![0i16; 9600];
        pcm.extend(say(TEXT, &home));
        let frames = adpcm::encode(&pcm, 320);
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let rec = tokio::task::spawn_blocking(move || record(s2));
        let hold = fake.wispr_hold(1, &frames, Duration::from_millis(20)).await;
        eprintln!(
            "hold: key down {:?} after prepare ({:?}), {:.2} s streamed",
            hold.pressed_after,
            hold.ready,
            hold.streamed.as_secs_f64()
        );
        tokio::time::sleep(Duration::from_millis(500)).await; // Wispr reads the tail
        stop.store(true, Ordering::SeqCst);
        let (got, rate) = rec.await.unwrap().expect("recording the device");

        // 3. what the device heard vs what was said
        let src: Vec<f32> = pcm.iter().map(|s| *s as f32 / 32768.0).collect();
        let (es, eg) = (envelope(&src, 160), envelope(&got, rate as usize / 100)); // 10 ms blocks
        let voiced = |e: &[f64]| e.iter().filter(|x| dbfs(**x) > -45.0).count();
        let (r, lag) = best_correlation(&es, &eg, 300);
        let heard_rms = (got.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / got.len().max(1) as f64).sqrt();
        let peak = got.iter().fold(0.0f32, |a, x| a.max(x.abs()));
        eprintln!(
            "device input: {:.2} s, rms {:.1} dBFS, peak {:.1} dBFS; voiced 10 ms blocks {} (said {}); envelope \
             correlation {r:.3} at a lag of {} ms",
            got.len() as f64 / rate as f64,
            dbfs(heard_rms),
            dbfs(peak as f64),
            voiced(&eg),
            voiced(&es),
            lag * 10
        );
        assert_eq!(hold.ready, Some(json!({"src": "agent", "wispr": "ready", "id": 1})));
        assert!(hold.pressed_after.unwrap() >= Duration::from_millis(250), "the lead time");
        assert!(dbfs(peak as f64) > -20.0, "the speech came through at a speech level");
        assert!(voiced(&eg) as f64 >= 0.9 * voiced(&es) as f64, "the whole sentence came through");
        assert!(r > 0.8, "the device heard what was said (envelope correlation {r:.3})");

        // 4. the device stays for the tail, then goes
        let t = Instant::now();
        while st.wmic.device().present && t.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let s = wmic::status(&st);
        eprintln!("last stream: {}\nlast hold: {}", s["last"], s["device_mic"]["last"]);
        assert!(!st.wmic.device().present, "removed after the tail");
        assert!(s["device_mic"]["last"]["kept_ms"].as_u64().unwrap() >= 800, "kept for the tail");
        assert_eq!(s["last"]["lost"], 0);
        assert_eq!(s["last"]["gated"], 0, "nothing was held back: the tap was there for all of it");
    }

    // 5. quit: nothing of ours left, the same counts as before
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || st2.shutdown()).await.unwrap();
    let t = Instant::now();
    let mut left = ours();
    while (!left.0.is_empty() || !left.1.is_empty()) && t.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
        left = ours();
    }
    let after = counts();
    eprintln!("devices before/after: {}/{}, process taps before/after: {}/{}", before.0, after.0, before.1, after.1);
    assert_eq!(left, (vec![], vec![]), "nothing of ours left");
    assert_eq!(before, after);
    let _ = std::fs::remove_dir_all(&home);
}
