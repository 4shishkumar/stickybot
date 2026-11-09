//! The virtual mic: our own process tap inside a public aggregate whose only sub-device (and
//! clock) is the default output. Whatever this process plays to the default output is muted on
//! the speakers and becomes the aggregate's input (once System Audio Recording is allowed).
//!
//! Aggregates and public taps outlive the process that made them, so every object we create is
//! recorded in a process-wide registry: [`destroy_all_live`] (for a Ctrl-C / watchdog handler)
//! and `Drop` both destroy exactly what is still recorded, each object once.

use std::sync::Mutex;
use std::time::Instant;

use crate::hal::{self, Id, Status};
use crate::naming::Names;
use crate::plist::aggregate_description;
use crate::tap::{self, Tap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Live {
    Device(Id),
    Tap(Id),
}

static LIVE: Mutex<Vec<Live>> = Mutex::new(Vec::new());

fn live() -> std::sync::MutexGuard<'static, Vec<Live>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn record(x: Live) {
    live().push(x);
}

/// Takes `x` out of the registry; true if it was there (so the caller is the one to destroy it).
pub(crate) fn claim(x: Live) -> bool {
    let mut l = live();
    match l.iter().position(|y| *y == x) {
        Some(i) => {
            l.remove(i);
            true
        }
        None => false,
    }
}

fn destroy(x: Live) -> Status {
    match x {
        Live::Device(d) => hal::destroy_aggregate(d),
        Live::Tap(t) => tap::destroy_process_tap(t),
    }
}

/// Destroys every device and tap this process created and hasn't destroyed yet (devices first:
/// a device lists its tap). Safe from any thread, e.g. a Ctrl-C handler. -> how many.
pub fn destroy_all_live() -> usize {
    let mut all = std::mem::take(&mut *live());
    all.sort_by_key(|x| matches!(x, Live::Tap(_))); // devices, then taps
    for x in &all {
        destroy(*x);
    }
    all.len()
}

/// A process tap of this process + the aggregate device that reads it.
#[derive(Debug)]
pub struct TapMic {
    pub names: Names,
    pub tap: Tap,
    pub dev: Id,
    /// The sub-device (clock): the default output when it was made.
    pub out_uid: String,
    pub create_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct DestroyReport {
    pub device_status: Option<Status>,
    pub tap_status: Option<Status>,
    pub ms: f64,
}

impl TapMic {
    /// Creates the tap (named `<uid>.tap`) of this process and the public aggregate
    /// (`names.name`, `names.uid`) around it, clocked by the default output.
    pub fn create(names: &Names) -> Result<Self, String> {
        tap::available()?;
        let t0 = Instant::now();
        let pid = std::process::id() as i32;
        let process = hal::process_object(pid).ok_or("macOS has no audio process object for this process")?;
        let out_uid = hal::tap_output_uid().ok_or("there's no default output device")?;
        let tap = tap::create_process_tap(&names.tap_name(), process)?;
        record(Live::Tap(tap.id));
        let desc = aggregate_description(&names.name, &names.uid, &out_uid, Some(&tap.uuid));
        let dev = match hal::create_aggregate(&desc) {
            Ok(d) => d,
            Err(e) => {
                if claim(Live::Tap(tap.id)) {
                    tap::destroy_process_tap(tap.id);
                }
                return Err(format!("can't create the {} device (CoreAudio error {})", names.name, hal::status_text(e)));
            }
        };
        record(Live::Device(dev));
        Ok(Self {
            names: names.clone(),
            tap,
            dev,
            out_uid,
            create_ms: t0.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Our tap and device still exist (not destroyed by us or a signal handler): the only time
    /// it's safe to let audio out of this process (it's muted then — if the probe passed).
    pub fn live(&self) -> bool {
        let l = live();
        l.contains(&Live::Device(self.dev)) && l.contains(&Live::Tap(self.tap.id))
    }

    /// Destroys the device, then the tap. Idempotent.
    pub fn destroy(&mut self) -> DestroyReport {
        let t0 = Instant::now();
        let mut r = DestroyReport::default();
        if claim(Live::Device(self.dev)) {
            r.device_status = Some(destroy(Live::Device(self.dev)));
        }
        if claim(Live::Tap(self.tap.id)) {
            r.tap_status = Some(destroy(Live::Tap(self.tap.id)));
        }
        r.ms = t0.elapsed().as_secs_f64() * 1000.0;
        r
    }
}

impl Drop for TapMic {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Removes devices / taps a crashed earlier run left behind: aggregates whose UID is ours (the
/// device or its check device) and taps with our tap names. -> (devices, taps) removed.
pub fn cleanup_stale(names: &Names) -> (usize, usize) {
    let uids = names.owned_device_uids();
    let tap_names = names.owned_tap_names();
    let mut nd = 0;
    for d in hal::devices() {
        if uids.contains(&d.uid) && hal::destroy_aggregate(d.id) == 0 {
            nd += 1;
        }
    }
    let mut nt = 0;
    for t in hal::taps() {
        if tap_names.contains(&t.name) && tap::destroy_process_tap(t.id) == 0 {
            nt += 1;
        }
    }
    (nd, nt)
}
