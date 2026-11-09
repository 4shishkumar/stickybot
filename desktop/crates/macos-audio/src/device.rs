//! The "<stick>-bt-mic" device (stick_mic_device.py `StickMicDevice`): creates / removes the
//! public aggregate (and, in tap mode, our process tap), finds out whether someone (Wispr) reads
//! it, and runs the inaudible tap check. Synchronous; the owner runs it on one worker thread.
//! Plain Rust over the [`CoreAudio`] trait (tests use [`crate::fake::FakeCoreAudio`]).
//!
//! Cleanup only ever touches objects with *our* UID (and its `.check` / `.tap` names). The Rust
//! app's UID is [`crate::naming::RUST_UID`], never the Python agent's.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ca::{CoreAudio, DeviceInfo, Factory, Id, status_text};
use crate::carrier::{BLACKHOLE_NAME, Carrier, CheckState, NO_BLACKHOLE, PERMISSION_HINT};
use crate::naming::{CHECK_SUFFIX, Names, TAP_SUFFIX};

/// The tap check: at most this long for the tone to come through.
pub const PROBE_S: f64 = 1.5;
/// -80 dBFS: inaudible even if the tap doesn't mute it.
pub const PROBE_AMP: f32 = 1e-4;

/// `ensure_present()`'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// just now (Wispr needs the lead time to notice it)
    Created,
    /// it already was
    Present,
    /// it can't be (`error` says why)
    No,
}

impl Made {
    pub fn as_str(self) -> &'static str {
        match self {
            Made::Created => "created",
            Made::Present => "present",
            Made::No => "failed",
        }
    }
    pub fn ok(self) -> bool {
        self != Made::No
    }
}

/// The last tap check (`probe()`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Check {
    pub state: Option<CheckState>,
    pub detail: String,
    /// unix time
    pub at: Option<f64>,
    pub create_ms: Option<f64>,
    pub first_ms: Option<f64>,
    pub level: Option<f64>,
    pub ms: Option<f64>,
}

impl Check {
    pub fn to_json(&self) -> Value {
        json!({"state": self.state, "detail": self.detail, "at": self.at, "create_ms": self.create_ms,
               "first_ms": self.first_ms, "level": self.level, "ms": self.ms})
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn ms_since(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

pub struct StickMicDevice {
    factory: Factory,
    ca: Option<Box<dyn CoreAudio>>,
    pub name: String,
    pub uid: String,
    /// BlackHole's device name
    pub carrier: String,
    /// the carrier to make it with (the owner sets it, see `pick_carrier`)
    pub mode: Carrier,
    pub own_pid: i32,
    /// its Core Audio id while it exists
    pub dev: Option<Id>,
    /// its sub-device (BlackHole, or the output device in tap mode)
    pub carrier_dev: Option<Id>,
    /// our process tap while it exists (tap mode)
    pub tap: Option<Id>,
    /// the carrier the device was made with
    pub dev_mode: Option<Carrier>,
    /// processes already reading the sub-device itself when we made the device (`None`: unknown)
    pub baseline: Option<HashSet<i32>>,
    pub error: String,
    pub create_ms: Option<f64>,
    pub destroy_ms: Option<f64>,
    pub check: Check,
    /// `tap_live()` for the audio thread (never locks anything): set once the tapped device
    /// exists, cleared *before* it is destroyed
    live: Arc<AtomicBool>,
    /// how long the audio thread gets to see `live` go false before the tap goes (its muting with it)
    pub silence_before_destroy: Duration,
}

impl StickMicDevice {
    pub fn new(factory: Factory, names: &Names, own_pid: i32) -> Self {
        Self {
            factory,
            ca: None,
            name: names.name.clone(),
            uid: names.uid.clone(),
            carrier: BLACKHOLE_NAME.to_string(),
            mode: Carrier::BlackHole,
            own_pid,
            dev: None,
            carrier_dev: None,
            tap: None,
            dev_mode: None,
            baseline: None,
            error: String::new(),
            create_ms: None,
            destroy_ms: None,
            check: Check::default(),
            live: Arc::new(AtomicBool::new(false)),
            silence_before_destroy: Duration::from_millis(30),
        }
    }

    /// A new backend (tests): forgets the old one.
    pub fn set_factory(&mut self, factory: Factory) {
        self.factory = factory;
        self.ca = None;
    }

    pub fn tap_name(&self) -> String {
        format!("{}{TAP_SUFFIX}", self.uid)
    }

    fn ca(&mut self) -> Result<&mut Box<dyn CoreAudio>, String> {
        if self.ca.is_none() {
            self.ca = Some((self.factory)()?);
        }
        Ok(self.ca.as_mut().expect("just made"))
    }

    fn set_error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if !msg.is_empty() && msg != self.error {
            log::warn!("stick mic: {msg}");
        }
        self.error = msg;
    }

    pub fn present(&self) -> bool {
        self.dev.is_some()
    }

    /// Our tap exists (so this process's sound is muted and goes to the device): the only time the
    /// player lets the stick's audio out in tap mode.
    pub fn tap_live(&self) -> bool {
        self.mode == Carrier::Tap && self.tap.is_some() && self.dev.is_some()
    }

    /// The same, for the audio thread.
    pub fn live_flag(&self) -> Arc<AtomicBool> {
        self.live.clone()
    }

    /// The device exists: `Created` (just now), `Present` (it already was) or `No`.
    pub fn ensure_present(&mut self) -> Made {
        let devices = match self.ca().and_then(|ca| ca.devices()) {
            Ok(d) => d,
            Err(e) => {
                self.dev = None;
                self.set_error(format!("can't create the {} device: {e}", self.name));
                return Made::No;
            }
        };
        let ours: Vec<&DeviceInfo> = devices.iter().filter(|d| d.uid == self.uid).collect();
        // (a BlackHole one left by an earlier run is fine to reuse; a tapped one needs our own tap)
        let same = match self.mode {
            Carrier::BlackHole => matches!(self.dev_mode, None | Some(Carrier::BlackHole)),
            Carrier::Tap => self.dev_mode == Some(Carrier::Tap) && self.tap.is_some(),
        };
        if !ours.is_empty() && same && ours.iter().all(|d| d.name == self.name) {
            self.dev = Some(ours[0].id);
            return Made::Present;
        }
        if !ours.is_empty() {
            // renamed in the settings, another carrier, or left by an earlier run: make it again
            let ids: Vec<Id> = ours.iter().map(|d| d.id).collect();
            self.remove_all("removed the old", Some(ids));
        }
        if self.mode == Carrier::Tap {
            return self.create_tapped(&devices);
        }
        let carrier = devices
            .iter()
            .find(|d| d.name == self.carrier)
            .or_else(|| devices.iter().find(|d| d.name.to_lowercase().contains(&self.carrier.to_lowercase())));
        let Some(carrier) = carrier.cloned() else {
            self.dev = None;
            self.set_error(NO_BLACKHOLE);
            return Made::No;
        };
        self.carrier_dev = Some(carrier.id);
        self.baseline = self.readers(carrier.id);
        let t = Instant::now();
        let (name, uid) = (self.name.clone(), self.uid.clone());
        match self.ca().map(|ca| ca.create_aggregate(&name, &uid, &carrier.uid, None)) {
            Ok(Ok(dev)) => self.made(dev, t, &self.carrier.clone()),
            Ok(Err(err)) => {
                self.dev = None;
                self.set_error(format!("can't create the {} device (CoreAudio error {})", self.name, status_text(err)));
                Made::No
            }
            Err(e) => {
                self.dev = None;
                self.set_error(format!("can't create the {} device: {e}", self.name));
                Made::No
            }
        }
    }

    fn made(&mut self, dev: Id, t: Instant, what: &str) -> Made {
        self.create_ms = Some(ms_since(t));
        self.dev = Some(dev);
        self.dev_mode = Some(self.mode);
        self.error.clear();
        self.live.store(self.tap_live(), Ordering::SeqCst);
        log::info!("stick mic: {} created in {:.1} ms (id {dev}, on {what})", self.name, self.create_ms.unwrap_or(0.0));
        Made::Created
    }

    /// -> (tap id, uuid, output uid).
    fn new_tap(&mut self, name: &str) -> Result<(Id, String, String), String> {
        let pid = self.own_pid;
        let ca = self.ca()?;
        let process = ca.process_object(pid).ok_or("macOS has no audio process object for this app")?;
        let out_uid = ca.tap_output_uid().ok_or("there's no default output device")?;
        let (tap, uuid) = ca.create_tap(name, process)?;
        Ok((tap, uuid, out_uid))
    }

    fn create_tapped(&mut self, devices: &[DeviceInfo]) -> Made {
        let t = Instant::now();
        let (tap, uuid, out_uid) = match self.new_tap(&self.tap_name()) {
            Ok(x) => x,
            Err(e) => {
                self.dev = None;
                self.set_error(format!("can't create the {} device: {e}", self.name));
                return Made::No;
            }
        };
        let out = devices.iter().find(|d| d.uid == out_uid).map(|d| d.id);
        self.carrier_dev = out;
        self.baseline = match out {
            Some(o) => self.readers(o),
            None => Some(HashSet::new()),
        };
        let (name, uid) = (self.name.clone(), self.uid.clone());
        let res = self.ca().map(|ca| ca.create_aggregate(&name, &uid, &out_uid, Some(&uuid)));
        match res {
            Ok(Ok(dev)) => {
                self.tap = Some(tap);
                self.made(dev, t, &format!("the Apple audio tap {tap}, clocked by {out_uid}"))
            }
            other => {
                if let Ok(ca) = self.ca() {
                    ca.destroy_tap(tap);
                }
                self.dev = None;
                let why = match other {
                    Ok(Err(err)) => format!("CoreAudio error {}", status_text(err)),
                    Err(e) => e,
                    Ok(Ok(_)) => unreachable!(),
                };
                self.set_error(format!("can't create the {} device ({why})", self.name));
                Made::No
            }
        }
    }

    /// Destroys every device with our UID (ours and any left over), and our tap. -> how many devices.
    pub fn remove(&mut self) -> usize {
        self.remove_all("removed", None)
    }

    /// `remove()` for start / quit: a crashed app leaves its device and tap behind (and an
    /// interrupted check its test device). Only our UID / tap names.
    pub fn cleanup_stale(&mut self) -> usize {
        let n = self.remove_all("removed", None);
        let check_uid = format!("{}{CHECK_SUFFIX}", self.uid);
        let tap_names = [self.tap_name(), format!("{check_uid}{TAP_SUFFIX}")];
        if let Ok(ca) = self.ca() {
            if let Ok(devs) = ca.devices() {
                for d in devs.iter().filter(|d| d.uid == check_uid) {
                    ca.destroy_aggregate(d.id);
                }
            }
            match ca.taps() {
                Ok(taps) => {
                    for t in taps.iter().filter(|t| tap_names.contains(&t.name)) {
                        let err = ca.destroy_tap(t.id);
                        log::info!(
                            "stick mic: removed a left-over audio tap {} ({}){}",
                            t.id,
                            t.name,
                            if err != 0 { format!(": CoreAudio error {err}") } else { String::new() }
                        );
                    }
                }
                Err(e) => log::debug!("stick mic: can't list audio taps: {e}"),
            }
        }
        n
    }

    fn remove_all(&mut self, what: &str, ours: Option<Vec<Id>>) -> usize {
        // silence first: the audio thread stops letting the stick out before the tap (and its
        // muting) goes away
        if self.live.swap(false, Ordering::SeqCst) {
            std::thread::sleep(self.silence_before_destroy);
        }
        let uid = self.uid.clone();
        let ours = match ours {
            Some(o) => o,
            None => match self.ca().and_then(|ca| ca.devices()) {
                Ok(d) => d.into_iter().filter(|d| d.uid == uid).map(|d| d.id).collect(),
                Err(e) => {
                    self.set_error(format!("can't list audio devices: {e}"));
                    return 0;
                }
            },
        };
        let mut n = 0;
        for d in ours {
            let t = Instant::now();
            let Ok(ca) = self.ca() else { break };
            let err = ca.destroy_aggregate(d);
            let ms = ms_since(t);
            if err != 0 {
                self.set_error(format!("can't remove the {} device (CoreAudio error {})", self.name, status_text(err)));
                continue;
            }
            n += 1;
            self.destroy_ms = Some(ms);
            let left = if Some(d) == self.dev { "" } else { " (left over)" };
            log::info!("stick mic: {what} {}{left} in {ms:.1} ms (id {d})", self.name);
        }
        self.dev = None;
        if let Some(tap) = self.tap.take() {
            // after the device: it lists the tap
            if let Ok(ca) = self.ca() {
                let err = ca.destroy_tap(tap);
                if err != 0 {
                    log::warn!("stick mic: can't remove the audio tap {tap} (CoreAudio error {err})");
                }
            }
        }
        n
    }

    fn readers(&mut self, dev: Id) -> Option<HashSet<i32>> {
        let own = self.own_pid;
        let pids = self.ca().ok()?.input_users(dev)?;
        Some(pids.into_iter().filter(|p| *p != own).collect())
    }

    /// Someone (Wispr) reads the device right now: `Some(true/false)`, or `None` when we can't
    /// tell. macOS may report an aggregate's readers on its sub-device, so a process that was
    /// already reading that one before the device appeared makes it ambiguous.
    pub fn recording(&mut self) -> Option<bool> {
        let Some(dev) = self.dev else { return Some(false) };
        match &self.baseline {
            None => return None,
            Some(b) if !b.is_empty() => return None,
            _ => {}
        }
        let mut readers = HashSet::new();
        for d in [Some(dev), self.carrier_dev].into_iter().flatten() {
            readers.extend(self.readers(d)?);
        }
        Some(!readers.is_empty())
    }

    /// Proves the tap path works before any stick audio goes through it: a test-named tap +
    /// aggregate ("…-check", never the real name / UID), a -80 dBFS tone from this process into
    /// it, and its input must hear the tone. Without the System Audio Recording permission (or
    /// while macOS still asks) a tap hears silence — and doesn't mute either. The first call is
    /// what makes macOS ask.
    pub fn probe(&mut self, secs: f64, amp: f32) -> Check {
        let t0 = Instant::now();
        let mut res = Check { state: Some(CheckState::Error), at: Some(unix_now()), ..Check::default() };
        let check_uid = format!("{}{CHECK_SUFFIX}", self.uid);
        let check_name = format!("{}-check", self.name);
        let mut tap = None;
        let mut dev = None;
        let r = (|| -> Result<(), String> {
            let (t, uuid, out_uid) = self.new_tap(&format!("{check_uid}{TAP_SUFFIX}"))?;
            tap = Some(t);
            let ca = self.ca()?;
            let d = ca
                .create_aggregate(&check_name, &check_uid, &out_uid, Some(&uuid))
                .map_err(|e| format!("can't create the check device (CoreAudio error {})", status_text(e)))?;
            dev = Some(d);
            res.create_ms = Some(ms_since(t0));
            let m = ca.measure_input(d, &check_uid, &out_uid, secs, amp)?;
            res.first_ms = m.first_ms;
            res.level = Some(m.rms);
            let expected = amp as f64 / std::f64::consts::SQRT_2;
            if m.first_ms.is_some() && m.rms >= expected * 0.25 {
                res.state = Some(CheckState::Ok);
                res.detail = "the Apple audio tap works".into();
            } else if m.callbacks == 0 {
                res.detail = "the check device didn't run".into();
            } else {
                res.state = Some(CheckState::Failed);
                res.detail = format!("the tap heard nothing — {PERMISSION_HINT}");
            }
            Ok(())
        })();
        if let Err(e) = r {
            res.detail = e;
        }
        if let Ok(ca) = self.ca() {
            if let Some(d) = dev {
                let err = ca.destroy_aggregate(d);
                if err != 0 {
                    log::warn!("stick mic: can't remove the check device: {err}");
                }
            }
            if let Some(t) = tap {
                let err = ca.destroy_tap(t);
                if err != 0 {
                    log::warn!("stick mic: can't remove the check tap: {err}");
                }
            }
        }
        res.ms = Some(ms_since(t0));
        log::info!(
            "stick mic: tap check {:?} in {:.0} ms (created in {:?} ms, tone after {:?} ms, level {:?}): {}",
            res.state,
            res.ms.unwrap_or(0.0),
            res.create_ms,
            res.first_ms,
            res.level,
            res.detail
        );
        self.check = res.clone();
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{BLACKHOLE_ID, FakeCoreAudio, SPEAKERS_ID};
    use crate::naming::RUST_UID;

    fn dev(fake: &FakeCoreAudio) -> StickMicDevice {
        StickMicDevice::new(fake.factory(), &Names::new("stickybot-bt-mic", RUST_UID), 1)
    }

    fn tapped(fake: &FakeCoreAudio) -> StickMicDevice {
        let mut d = dev(fake);
        d.mode = Carrier::Tap;
        d
    }

    // --- test_stick_mic_device.py: the device (BlackHole) ---

    #[test]
    fn creates_a_public_aggregate_on_blackhole() {
        let fake = FakeCoreAudio::new();
        let mut d = dev(&fake);
        assert_eq!(d.ensure_present(), Made::Created);
        assert_eq!(fake.created(), vec![("stickybot-bt-mic".into(), RUST_UID.into(), "BlackHole2ch_UID".into(), None)]);
        assert!(d.present() && d.create_ms.is_some());
        assert_eq!(d.ensure_present(), Made::Present);
        assert_eq!(fake.created().len(), 1); // not twice
    }

    #[test]
    fn remove_destroys_it() {
        let fake = FakeCoreAudio::new();
        let mut d = dev(&fake);
        d.ensure_present();
        assert_eq!(d.remove(), 1);
        assert!(!d.present() && fake.ours(RUST_UID).is_empty() && d.destroy_ms.is_some());
        assert_eq!(d.remove(), 0); // nothing left: fine
    }

    #[test]
    fn stale_ones_are_removed_and_only_ours() {
        let fake = FakeCoreAudio::new();
        fake.add_device(250, "stickybot-bt-mic", RUST_UID); // left by an app that was killed
        fake.add_device(251, "stickybot-bt-mic", RUST_UID);
        fake.add_device(252, "Someone else's aggregate", "com.other.agg");
        fake.add_device(253, "stickybot-bt-mic", crate::naming::UID); // the Python agent's: never touched
        let mut d = dev(&fake);
        assert_eq!(d.cleanup_stale(), 2);
        assert!(fake.ours(RUST_UID).is_empty());
        assert!(fake.has_device(252) && fake.has_device(253));
    }

    #[test]
    fn an_existing_one_is_reused_after_a_restart() {
        let fake = FakeCoreAudio::new();
        fake.add_device(250, "stickybot-bt-mic", RUST_UID);
        let mut d = dev(&fake);
        assert_eq!(d.ensure_present(), Made::Present);
        assert_eq!(d.dev, Some(250));
        assert!(fake.created().is_empty());
    }

    #[test]
    fn renamed_it_is_made_again() {
        let fake = FakeCoreAudio::new();
        let mut d = dev(&fake);
        d.ensure_present();
        d.name = "stick-mic-bt".into();
        assert_eq!(d.ensure_present(), Made::Created);
        assert_eq!(fake.names_of(RUST_UID), vec!["stick-mic-bt".to_string()]);
    }

    #[test]
    fn without_blackhole_it_says_how_to_install_it() {
        let fake = FakeCoreAudio::new();
        fake.remove_device(BLACKHOLE_ID);
        let mut d = dev(&fake);
        assert_eq!(d.ensure_present(), Made::No);
        assert!(d.error.contains("brew install --cask blackhole-2ch"));
        assert!(fake.created().is_empty());
    }

    #[test]
    fn a_coreaudio_error_is_reported() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.fail_create = Some(-50));
        let mut d = dev(&fake);
        assert_eq!(d.ensure_present(), Made::No);
        assert!(d.error.contains("-50") && !d.present());
    }

    #[test]
    fn coreaudio_missing_doesnt_crash() {
        let broken: Factory = Arc::new(|| Err("no CoreAudio here".to_string()));
        let mut d = StickMicDevice::new(broken, &Names::new("x", RUST_UID), 1);
        assert_eq!(d.ensure_present(), Made::No);
        assert!(d.error.contains("no CoreAudio"));
        assert_eq!(d.remove(), 0);
        assert_eq!(d.probe(0.1, PROBE_AMP).state, Some(CheckState::Error));
    }

    #[test]
    fn recording_sees_wispr_on_the_sub_device() {
        let fake = FakeCoreAudio::new();
        let mut d = dev(&fake);
        d.ensure_present();
        assert_eq!(d.recording(), Some(false));
        fake.set(|s| s.readers.insert(BLACKHOLE_ID, vec![1])); // our own output stream: not a reader
        assert_eq!(d.recording(), Some(false));
        fake.set(|s| s.readers.insert(BLACKHOLE_ID, vec![1, 4242])); // the aggregate's reader on BlackHole
        assert_eq!(d.recording(), Some(true));
    }

    #[test]
    fn recording_is_unknown_when_blackhole_already_had_a_reader() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.readers.insert(BLACKHOLE_ID, vec![777])); // e.g. Wispr still set to BlackHole itself
        let mut d = dev(&fake);
        d.ensure_present();
        assert_eq!(d.recording(), None);
        fake.set(|s| s.per_process = false); // macOS before 14.2
        d.remove();
        d.ensure_present();
        assert_eq!(d.recording(), None);
    }

    // --- test_tap_mic.py: the tapped aggregate ---

    #[test]
    fn tap_mode_makes_a_public_aggregate_with_our_tap() {
        let fake = FakeCoreAudio::new();
        let mut d = tapped(&fake);
        assert_eq!(d.ensure_present(), Made::Created);
        let created = fake.created();
        assert_eq!(created.len(), 1);
        let (name, uid, sub, tap_uuid) = &created[0];
        assert_eq!((name.as_str(), uid.as_str(), sub.as_str()), ("stickybot-bt-mic", RUST_UID, "Speakers_UID"));
        let taps = fake.taps_now();
        assert_eq!(taps.len(), 1);
        assert_eq!(Some(&taps[0].uuid), tap_uuid.as_ref());
        assert_eq!(taps[0].name, format!("{RUST_UID}.tap"));
        assert_eq!(fake.tap_process(taps[0].id), Some(fake.process_of(1)));
        assert!(d.tap_live() && d.live_flag().load(Ordering::SeqCst));
        assert_eq!(d.carrier_dev, Some(SPEAKERS_ID));
        assert_eq!(d.ensure_present(), Made::Present);
    }

    #[test]
    fn removing_it_destroys_the_tap_too() {
        let fake = FakeCoreAudio::new();
        let mut d = tapped(&fake);
        d.ensure_present();
        assert_eq!(d.remove(), 1);
        assert!(fake.taps_now().is_empty() && fake.ours(RUST_UID).is_empty());
        assert!(!d.tap_live() && !d.live_flag().load(Ordering::SeqCst));
        // the device went before the tap (it lists the tap)
        assert_eq!(fake.order().iter().map(|s| s.as_str()).collect::<Vec<_>>(), vec!["tap+", "dev+", "dev-", "tap-"]);
    }

    #[test]
    fn a_leftover_aggregate_without_our_tap_is_made_again() {
        let fake = FakeCoreAudio::new();
        fake.add_device(250, "stickybot-bt-mic", RUST_UID);
        let mut d = tapped(&fake);
        assert_eq!(d.ensure_present(), Made::Created);
        assert!(!fake.has_device(250) && d.tap.is_some());
    }

    #[test]
    fn switching_carrier_makes_it_again() {
        let fake = FakeCoreAudio::new();
        let mut d = dev(&fake);
        d.ensure_present();
        d.mode = Carrier::Tap;
        assert_eq!(d.ensure_present(), Made::Created);
        assert_eq!(fake.created().last().unwrap().2, "Speakers_UID");
        d.mode = Carrier::BlackHole;
        assert_eq!(d.ensure_present(), Made::Created);
        assert!(fake.taps_now().is_empty() && d.tap.is_none());
    }

    #[test]
    fn tap_failure_makes_no_device() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.fail_tap = Some("can't create the audio tap (CoreAudio error 'what')".into()));
        let mut d = tapped(&fake);
        assert_eq!(d.ensure_present(), Made::No);
        assert!(d.error.contains("audio tap") && fake.created().is_empty());
    }

    #[test]
    fn aggregate_failure_removes_the_tap() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.fail_create = Some(-50));
        let mut d = tapped(&fake);
        assert_eq!(d.ensure_present(), Made::No);
        assert!(fake.taps_now().is_empty() && d.tap.is_none() && !d.live_flag().load(Ordering::SeqCst));
    }

    #[test]
    fn no_default_output() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.default_output = None);
        let mut d = tapped(&fake);
        assert_eq!(d.ensure_present(), Made::No);
        assert!(d.error.contains("no default output"));
        assert!(fake.taps_now().is_empty());
    }

    #[test]
    fn stale_taps_are_removed_by_name() {
        let fake = FakeCoreAudio::new();
        fake.add_tap(60, &format!("{RUST_UID}.tap"));
        fake.add_tap(61, &format!("{RUST_UID}.check.tap"));
        fake.add_tap(62, "com.stickkey.wireless-mic.tap"); // the Python agent's
        fake.add_tap(63, "someone.else.tap");
        fake.add_device(70, "stickybot-bt-mic-check", &format!("{RUST_UID}.check"));
        let mut d = tapped(&fake);
        d.cleanup_stale();
        let left: Vec<String> = fake.taps_now().into_iter().map(|t| t.name).collect();
        assert_eq!(left, vec!["com.stickkey.wireless-mic.tap".to_string(), "someone.else.tap".into()]);
        assert!(!fake.has_device(70));
    }

    #[test]
    fn check_hears_the_tone() {
        let fake = FakeCoreAudio::new();
        let mut d = tapped(&fake);
        let c = d.probe(PROBE_S, PROBE_AMP);
        assert_eq!(c.state, Some(CheckState::Ok), "{}", c.detail);
        assert!(c.create_ms.is_some() && c.first_ms.is_some());
        // a test-named device, never the real one, and nothing left behind
        assert_eq!(fake.created()[0].0, "stickybot-bt-mic-check");
        assert_eq!(fake.created()[0].1, format!("{RUST_UID}.check"));
        assert!(fake.all_devices().iter().all(|x| !x.uid.starts_with(RUST_UID)) && fake.taps_now().is_empty());
        assert_eq!(fake.measured(), 1);
        assert!(!d.present() && d.check.state == Some(CheckState::Ok));
    }

    #[test]
    fn check_hears_nothing_without_the_permission() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.tap_hears = false);
        let c = tapped(&fake).probe(PROBE_S, PROBE_AMP);
        assert_eq!(c.state, Some(CheckState::Failed));
        assert!(c.detail.contains("System Audio Recording"));
        assert!(fake.taps_now().is_empty() && fake.all_devices().iter().all(|x| !x.uid.starts_with(RUST_UID)));
    }

    #[test]
    fn check_error_is_reported() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.fail_tap = Some("no taps here".into()));
        let c = tapped(&fake).probe(PROBE_S, PROBE_AMP);
        assert_eq!(c.state, Some(CheckState::Error));
        assert!(c.detail.contains("no taps here"));
    }

    #[test]
    fn a_quiet_signal_doesnt_count() {
        let fake = FakeCoreAudio::new();
        fake.set(|s| s.measure = Some(crate::ca::Measure { callbacks: 9, rms: 1e-6, first_ms: Some(50.0), ms: 100.0 }));
        assert_eq!(tapped(&fake).probe(PROBE_S, PROBE_AMP).state, Some(CheckState::Failed));
        fake.set(|s| s.measure = Some(crate::ca::Measure { callbacks: 0, rms: 0.0, first_ms: None, ms: 100.0 }));
        let c = tapped(&fake).probe(PROBE_S, PROBE_AMP);
        assert_eq!((c.state, c.detail.as_str()), (Some(CheckState::Error), "the check device didn't run"));
    }
}
