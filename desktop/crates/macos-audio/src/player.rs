//! The stick's mic, played where the "<stick>-bt-mic" device hears it (wispr_mic.py `WisprMic`).
//!
//! While A holds the Wispr shortcut away from USB, the stick streams its mic (the same ADPCM
//! frames as a B talk) between `{"evt":"wispr","state":"prepare"}` and `…"stop"`. The owner
//! decodes the frames; this puts them in order ([`Sequencer`]), resamples them to the output's
//! rate and plays them from a small jitter buffer by an output stream:
//!
//! - **tap**: into the default output (the built-in one when the default is Bluetooth); our process tap mutes it and hands it to the device. Audio
//!   only goes out while the tap exists (`gate`), so the stick never plays out of the speakers.
//!   The stream is opened at "prepare" and closed once the device is removed (it's a real output
//!   device: no silence kept playing into it).
//! - **blackhole**: into BlackHole 2ch's output; that stream stays open while the feature is on and
//!   the stick is connected.
//!
//! The output itself is behind [`Outputs`] (cpal on macOS, [`crate::fake::FakeOutputs`] in tests).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::carrier::{BLACKHOLE_NAME, Carrier, NO_BLACKHOLE};
use crate::jitter::{JitterBuffer, RATE, Resampler, Sequencer};

/// How often to look for the device again while it's missing.
pub const RESCAN: Duration = Duration::from_secs(10);

/// An output device to play into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutDevice {
    pub uid: String,
    pub name: String,
    pub rate: u32,
    pub channels: usize,
}

/// Fills an interleaved buffer of `channels` per frame. Runs on the audio thread.
pub type Render = Box<dyn FnMut(&mut [f32], usize) + Send>;

/// The output side of the audio system.
pub trait Outputs: Send + Sync {
    /// What the tap mode plays into: the default output, or the built-in one when the default is
    /// Bluetooth (the same device the tap's aggregate is clocked by).
    fn tap_output(&self) -> Option<OutDevice>;
    /// The first output device whose name contains `name` (any case).
    fn find(&self, name: &str) -> Option<OutDevice>;
    /// Opens and starts an output stream on `dev`; dropping the stream closes it.
    fn open(&self, dev: &OutDevice, render: Render) -> Result<Box<dyn OutStream>, String>;
}

pub trait OutStream: Send {
    /// false once it stopped by itself (its device went away)
    fn alive(&self) -> bool;
}

/// May audio go out now? (tap mode: only while our tap exists)
pub type Gate = Arc<dyn Fn() -> bool + Send + Sync>;

/// Reports the player's status (`wispr_mic` events).
pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

pub struct WisprPlayer {
    outputs: Arc<dyn Outputs>,
    emit: Emit,
    /// `None`: nothing can carry the audio (`unavailable` says why)
    pub carrier: Option<Carrier>,
    pub unavailable: String,
    gate: Gate,
    /// blocks held back by the gate in the last stream
    gated: Arc<AtomicU64>,
    pub enabled: bool,
    dev: Option<OutDevice>,
    stream: Option<Box<dyn OutStream>>,
    jb: Arc<Mutex<JitterBuffer>>,
    resample: Option<Resampler>,
    seqr: Option<Sequencer>,
    /// between start and stop
    active: Arc<AtomicBool>,
    pub error: String,
    missing_logged: bool,
    looked_at: Option<Instant>,
    /// stats of the last stream, for the dashboard
    pub last: Option<Value>,
    started: Instant,
}

impl WisprPlayer {
    pub fn new(outputs: Arc<dyn Outputs>, emit: Emit) -> Self {
        Self {
            outputs,
            emit,
            carrier: Some(Carrier::BlackHole),
            unavailable: String::new(),
            gate: Arc::new(|| true),
            gated: Arc::new(AtomicU64::new(0)),
            enabled: false,
            dev: None,
            stream: None,
            jb: Arc::new(Mutex::new(JitterBuffer::new(48_000))),
            resample: None,
            seqr: None,
            active: Arc::new(AtomicBool::new(false)),
            error: String::new(),
            missing_logged: false,
            looked_at: None,
            last: None,
            started: Instant::now(),
        }
    }

    pub fn active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub fn is_open(&self) -> bool {
        self.stream.is_some()
    }

    /// The jitter buffer (tests look at it).
    pub fn jitter(&self) -> Arc<Mutex<JitterBuffer>> {
        self.jb.clone()
    }

    fn report(&self) {
        (self.emit)(self.status());
    }

    fn set_error(&mut self, msg: &str) {
        if msg != self.error {
            self.error = msg.to_string();
            if !msg.is_empty() {
                log::warn!("wireless mic: {msg}");
            }
            self.report();
        }
    }

    /// BlackHole 2ch is there.
    pub fn has_blackhole(&self) -> bool {
        if self.carrier == Some(Carrier::BlackHole) && self.dev.is_some() {
            return true;
        }
        self.outputs.find(BLACKHOLE_NAME).is_some()
    }

    /// The carrier was picked (`pick_carrier`): reopen on the new device.
    pub fn set_carrier(&mut self, carrier: Option<Carrier>, why: &str, gate: Option<Gate>) {
        let gate = gate.unwrap_or_else(|| Arc::new(|| true));
        if carrier == self.carrier && why == self.unavailable {
            self.gate = gate;
            return;
        }
        let changed = carrier != self.carrier;
        self.carrier = carrier;
        self.unavailable = why.to_string();
        self.gate = gate;
        if changed {
            log::info!(
                "wireless mic: carrier {}{}",
                carrier.map(Carrier::as_str).unwrap_or("none"),
                if why.is_empty() { String::new() } else { format!(" ({why})") }
            );
            self.close();
            self.dev = None;
            self.looked_at = None;
            self.missing_logged = false;
        }
        if carrier.is_none() {
            self.set_error(why);
        } else if !self.error.is_empty() && self.dev.is_none() {
            self.set_error("");
        }
        self.report();
    }

    /// The device we play into (BlackHole, or the default output for the tap), looked for again
    /// at most every [`RESCAN`] while it's missing.
    pub fn device(&mut self, force: bool) -> Option<OutDevice> {
        if self.dev.is_some() && !force {
            return self.dev.clone();
        }
        if !force && self.looked_at.is_some_and(|t| t.elapsed() < RESCAN) {
            return None;
        }
        self.looked_at = Some(Instant::now());
        let carrier = self.carrier?;
        self.dev = match carrier {
            Carrier::Tap => self.outputs.tap_output(),
            Carrier::BlackHole => self.outputs.find(BLACKHOLE_NAME),
        };
        if let Some(d) = &self.dev {
            self.missing_logged = false;
            log::info!("wireless mic: playing into {} ({} Hz, {} ch)", d.name, d.rate, d.channels);
            self.set_error("");
        } else if !self.missing_logged {
            self.missing_logged = true;
            self.set_error(if carrier == Carrier::Tap {
                "there's no output device to play into"
            } else {
                NO_BLACKHOLE
            });
        }
        self.dev.clone()
    }

    /// On, with somewhere to play: what the stick is told (`wmic` in the hello).
    pub fn ready(&mut self) -> bool {
        self.enabled && self.device(false).is_some()
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.close();
        }
        self.report();
    }

    /// Opens the output stream (silence until a stream starts). -> true if open.
    pub fn open(&mut self) -> bool {
        if self.stream.as_ref().is_some_and(|s| !s.alive()) && !self.active() {
            self.close(); // stopped by itself (its device went away): open it again
            self.dev = None;
            self.looked_at = None;
        }
        if self.stream.is_some() {
            return true;
        }
        if self.carrier == Some(Carrier::Tap) {
            // the tap's output now (the default output may have changed since the last hold: AirPods…)
            self.dev = None;
            self.looked_at = None;
        }
        let Some(dev) = self.device(false) else { return false };
        let jb = Arc::new(Mutex::new(JitterBuffer::new(dev.rate)));
        self.jb = jb.clone();
        let gate = self.gate.clone();
        let gated = self.gated.clone();
        let active = self.active.clone();
        let through = self.carrier == Some(Carrier::BlackHole);
        let mut mono: Vec<f32> = Vec::new();
        let render: Render = Box::new(move |out: &mut [f32], channels: usize| {
            let ch = channels.max(1);
            let frames = out.len() / ch;
            mono.resize(frames, 0.0);
            match jb.lock() {
                Ok(mut j) => j.pull_into(&mut mono),
                Err(_) => mono.fill(0.0),
            }
            if !through && !gate() {
                // tap mode without our tap: this is the real speakers — silence (the audio is dropped)
                if active.load(Ordering::Relaxed) && mono.iter().any(|x| *x != 0.0) {
                    gated.fetch_add(1, Ordering::Relaxed);
                }
                out.fill(0.0);
                return;
            }
            for (frame, s) in out.chunks_mut(ch).zip(mono.iter()) {
                frame.fill(*s); // mono to every channel
            }
        });
        match self.outputs.open(&dev, render) {
            Ok(s) => {
                self.stream = Some(s);
                self.set_error("");
                true
            }
            Err(e) => {
                self.stream = None;
                self.dev = None; // look for it again
                self.set_error(&format!("can't play into {}: {e}", dev.name));
                false
            }
        }
    }

    pub fn close(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        self.stream = None; // dropping it stops it
    }

    /// The stick connected: have the stream ready before the first key-down. Not in tap mode:
    /// there it's the real output device (e.g. AirPods), so it's only open around a hold.
    pub fn warm(&mut self) {
        if self.enabled && self.carrier != Some(Carrier::Tap) {
            self.open();
        }
    }

    /// The mic device is gone: in tap mode, let go of the output device until the next hold.
    pub fn release(&mut self) {
        if self.carrier == Some(Carrier::Tap) && !self.active() {
            self.close();
        }
    }

    // --- a stream from the stick ---

    pub fn start(&mut self, rate: u32) -> bool {
        if !self.enabled || !self.open() {
            self.active.store(false, Ordering::SeqCst);
            return false;
        }
        let out_rate = self.dev.as_ref().map(|d| d.rate).unwrap_or(48_000);
        if let Ok(mut j) = self.jb.lock() {
            j.clear();
            j.underruns = 0;
            j.dropped = 0;
        }
        self.gated.store(0, Ordering::SeqCst);
        self.resample = Some(Resampler::new(if rate == 0 { RATE } else { rate }, out_rate));
        self.seqr = Some(Sequencer::default());
        self.active.store(true, Ordering::SeqCst);
        self.started = Instant::now();
        self.report();
        true
    }

    /// One decoded frame: `(seq, samples)`.
    pub fn add(&mut self, seq: u16, samples: Vec<i16>) {
        if !self.active() {
            return;
        }
        let (Some(seqr), Some(rs)) = (self.seqr.as_mut(), self.resample.as_mut()) else { return };
        for chunk in seqr.add(seq, samples) {
            let x: Vec<f32> = chunk.iter().map(|s| *s as f32 / 32768.0).collect();
            let y = rs.process(&x);
            if let Ok(mut j) = self.jb.lock() {
                j.push(&y);
            }
        }
    }

    pub fn stop(&mut self) {
        if !self.active() {
            return;
        }
        self.active.store(false, Ordering::SeqCst);
        let rate = self.dev.as_ref().map(|d| d.rate).unwrap_or(48_000);
        let mut seqr = self.seqr.take().unwrap_or_default();
        if let Some(rs) = self.resample.as_mut() {
            for chunk in seqr.flush() {
                let x: Vec<f32> = chunk.iter().map(|s| *s as f32 / 32768.0).collect();
                let y = rs.process(&x);
                if let Ok(mut j) = self.jb.lock() {
                    j.push(&y);
                }
            }
        }
        let (underruns, dropped) = match self.jb.lock() {
            Ok(mut j) => {
                j.drain(); // play out the rest, then silence
                (j.underruns, j.dropped)
            }
            Err(_) => (0, 0),
        };
        let secs = (self.started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        let last = json!({"t": unix_now(), "secs": secs, "frames": seqr.frames, "lost": seqr.lost, "late": seqr.late,
                          "underruns": underruns, "dropped_ms": dropped * 1000 / rate.max(1) as u64,
                          "gated": self.gated.load(Ordering::SeqCst)});
        log::info!(
            "wispr stream: {secs:.1}s, {} frames, {} lost, {} late, {underruns} underruns, {} ms dropped",
            seqr.frames,
            seqr.lost,
            seqr.late,
            last["dropped_ms"]
        );
        self.last = Some(last);
        // keep the counters for `status()` until the next start
        self.seqr = Some(seqr);
        self.report();
    }

    pub fn status(&self) -> Value {
        json!({"enabled": self.enabled, "device": self.dev.as_ref().map(|d| d.name.clone()),
               "carrier": self.carrier, "open": self.stream.is_some(), "streaming": self.active(),
               "error": self.error, "last": self.last})
    }

    /// Blocks held back by the gate in the last stream.
    pub fn gated(&self) -> u64 {
        self.gated.load(Ordering::SeqCst)
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeOutputs;

    fn tone(secs: f64, rate: u32) -> Vec<i16> {
        (0..(rate as f64 * secs) as usize)
            .map(|i| (8000.0 * (std::f64::consts::TAU * 440.0 * i as f64 / rate as f64).sin()) as i16)
            .collect()
    }

    /// The tone in 20 ms frames, as (seq, samples) the way the decoder hands them over.
    fn frames(secs: f64) -> Vec<(u16, Vec<i16>)> {
        tone(secs, 16000).chunks(320).enumerate().map(|(i, c)| (i as u16, c.to_vec())).collect()
    }

    fn player(out: &FakeOutputs) -> (WisprPlayer, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (WisprPlayer::new(Arc::new(out.clone()), Arc::new(move |v| s.lock().unwrap().push(v))), seen)
    }

    // --- test_wispr_mic.py: the device + streams ---

    #[test]
    fn missing_device_is_reported_once_and_doesnt_crash() {
        let out = FakeOutputs::new(vec![]);
        let (mut m, seen) = player(&out);
        m.set_enabled(true);
        assert!(!m.ready());
        assert!(!m.start(16000));
        m.add(0, vec![1; 320]); // ignored
        m.stop();
        let st = m.status();
        assert!(st["device"].is_null() && st["error"].as_str().unwrap().contains("brew install --cask blackhole-2ch"));
        let errors = |s: &Arc<Mutex<Vec<Value>>>| {
            s.lock().unwrap().iter().filter(|d| d["error"].as_str().is_some_and(|e| !e.is_empty())).count()
        };
        let n = errors(&seen);
        m.device(true); // still missing: not reported again
        assert_eq!(errors(&seen), n);
    }

    #[test]
    fn device_appears_later() {
        let out = FakeOutputs::new(vec![]);
        let (mut m, _) = player(&out);
        m.set_enabled(true);
        assert!(!m.ready());
        out.set_devices(vec![FakeOutputs::blackhole()]);
        assert!(!m.ready()); // not looked at again straight away
        assert!(m.device(true).is_some() && m.ready() && m.error.is_empty());
    }

    #[test]
    fn off_never_touches_audio() {
        let out = FakeOutputs::new(vec![FakeOutputs::blackhole()]);
        let (mut m, _) = player(&out);
        m.set_enabled(false);
        assert!(!m.ready() && !m.start(16000));
        m.warm();
        assert_eq!(out.streams().len(), 0);
    }

    #[test]
    fn a_stream_plays_into_blackhole() {
        let out = FakeOutputs::new(vec![FakeOutputs::blackhole()]);
        let (mut m, _) = player(&out);
        m.set_enabled(true);
        m.warm();
        let stream = out.stream(0);
        assert_eq!((stream.device().rate, stream.device().channels), (48000, 2));
        assert!(stream.play(480).iter().all(|x| *x == 0.0)); // idle: silence
        assert!(m.start(16000) && m.status()["streaming"] == true);
        for (q, s) in frames(0.5) {
            m.add(q, s);
        }
        let played: Vec<f32> = (0..50).flat_map(|_| stream.play(480)).collect(); // 0.5 s at 48 kHz
        assert_eq!(played.len(), 24000 * 2);
        assert!(played.chunks(2).all(|f| f[0] == f[1]));
        assert!(played.iter().fold(0.0f32, |a, x| a.max(x.abs())) > 0.1);
        m.stop();
        let tail: Vec<f32> = (0..20).flat_map(|_| stream.play(480)).collect();
        assert!(tail[tail.len() - 960..].iter().all(|x| *x == 0.0)); // played out, then silence
        let last = m.status()["last"].clone();
        assert_eq!((last["frames"].as_u64(), last["lost"].as_u64()), (Some(25), Some(0)));
        assert_eq!(m.status()["streaming"], false);
        m.set_enabled(false);
        assert!(stream.closed() && m.status()["open"] == false);
    }

    #[test]
    fn lost_frames_dont_shift_the_audio() {
        let out = FakeOutputs::new(vec![OutDevice { rate: 16000, ..FakeOutputs::blackhole() }]); // no resampling
        let (mut m, _) = player(&out);
        m.set_enabled(true);
        m.start(16000);
        let fs = frames(0.2); // 10 frames
        for (i, (q, s)) in fs.into_iter().enumerate() {
            if i != 4 {
                m.add(q, s); // frame 4 lost
            }
        }
        m.stop();
        let stream = out.stream(0);
        let played: Vec<f32> = (0..12).flat_map(|_| stream.play(320)).step_by(2).collect();
        let start = played.iter().position(|x| *x != 0.0).unwrap();
        assert!(played[start + 4 * 320 + 10..start + 5 * 320 - 10].iter().all(|x| *x == 0.0)); // the lost 20 ms
        assert!(played[start + 5 * 320 + 10..start + 6 * 320].iter().any(|x| *x != 0.0)); // frame 5 in place
        assert_eq!(m.last.as_ref().unwrap()["lost"], 1);
    }

    #[test]
    fn open_failure_is_reported() {
        let out = FakeOutputs::new(vec![FakeOutputs::blackhole()]);
        out.fail_open(Some("device busy"));
        let (mut m, _) = player(&out);
        m.set_enabled(true);
        assert!(!m.start(16000));
        assert!(m.error.contains("device busy") && m.dev.is_none());
    }

    // --- test_tap_mic.py: the gate ---

    #[test]
    fn without_the_tap_nothing_reaches_the_speakers() {
        let out = FakeOutputs::new(vec![FakeOutputs::speakers()]);
        let (mut m, _) = player(&out);
        let live = Arc::new(AtomicBool::new(false));
        let l = live.clone();
        m.set_carrier(Some(Carrier::Tap), "", Some(Arc::new(move || l.load(Ordering::SeqCst))));
        m.set_enabled(true);
        m.warm();
        assert_eq!(out.streams().len(), 0); // the real output device: not held open between holds
        m.start(16000); // old firmware's "start": no device, no tap
        let stream = out.stream(0);
        assert_eq!(stream.device().uid, "Speakers_UID");
        for (q, s) in frames(0.5) {
            m.add(q, s);
        }
        assert!(stream.play(48000 / 4).iter().all(|x| *x == 0.0)); // silence, not the tone
        assert!(m.gated() > 0);
        live.store(true, Ordering::SeqCst); // now our tap exists: it goes out (into the tap)
        for (q, s) in frames(0.2).into_iter().map(|(q, s)| (q + 25, s)) {
            m.add(q, s);
        }
        assert!(stream.play(4800).iter().any(|x| x.abs() > 0.05));
        m.stop();
        live.store(false, Ordering::SeqCst);
        m.release();
        assert!(stream.closed() && !m.is_open());
    }

    #[test]
    fn a_dead_stream_is_opened_again() {
        let out = FakeOutputs::new(vec![FakeOutputs::blackhole()]);
        let (mut m, _) = player(&out);
        m.set_enabled(true);
        m.warm();
        out.stream(0).kill();
        assert!(m.start(16000));
        assert_eq!(out.streams().len(), 2);
    }
}
