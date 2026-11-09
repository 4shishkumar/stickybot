//! Core Audio and the output devices in memory (conftest.py `FakeCoreAudio` / the fake
//! sounddevice), so the device, the player and the hold flow are tested without touching audio.
//! Clones share their state.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use crate::ca::{CoreAudio, DeviceInfo, Factory, Id, Measure, Status, TapInfo};
use crate::player::{OutDevice, OutStream, Outputs, Render};

pub const SPEAKERS_ID: Id = 100;
pub const BLACKHOLE_ID: Id = 120;

#[derive(Default)]
pub struct FakeState {
    /// id -> (name, uid)
    pub list: BTreeMap<Id, (String, String)>,
    /// (name, uid, sub uid, tap uuid) of every aggregate created
    pub created: Vec<(String, String, String, Option<String>)>,
    pub destroyed: Vec<Id>,
    pub fail_create: Option<Status>,
    pub fail_tap: Option<String>,
    /// device -> pids running input on it
    pub readers: HashMap<Id, Vec<i32>>,
    /// macOS 14.2+: per-process audio objects (else `input_users` can't tell)
    pub per_process: bool,
    pub default_output: Option<String>,
    /// tap id -> (uuid, name, process)
    pub tap_list: BTreeMap<Id, (String, String, Id)>,
    /// the tap carries our audio (System Audio Recording allowed)
    pub tap_hears: bool,
    /// a fixed answer for `measure_input`
    pub measure: Option<Measure>,
    pub measured: usize,
    /// "dev+", "dev-", "tap+", "tap-" in the order they happened
    pub order: Vec<String>,
    next: Id,
}

#[derive(Clone)]
pub struct FakeCoreAudio {
    s: Arc<Mutex<FakeState>>,
}

impl Default for FakeCoreAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCoreAudio {
    /// The built-in speakers (the default output) and BlackHole 2ch.
    pub fn new() -> Self {
        let mut st = FakeState { per_process: true, tap_hears: true, next: 200, ..FakeState::default() };
        st.list.insert(SPEAKERS_ID, ("MacBook Pro Speakers".into(), "Speakers_UID".into()));
        st.list.insert(BLACKHOLE_ID, ("BlackHole 2ch".into(), "BlackHole2ch_UID".into()));
        st.default_output = Some("Speakers_UID".into());
        Self { s: Arc::new(Mutex::new(st)) }
    }

    pub fn factory(&self) -> Factory {
        let me = self.clone();
        Arc::new(move || Ok(Box::new(me.clone()) as Box<dyn CoreAudio>))
    }

    pub fn set<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut self.s.lock().unwrap())
    }

    pub fn add_device(&self, id: Id, name: &str, uid: &str) {
        self.set(|s| s.list.insert(id, (name.into(), uid.into())));
    }
    pub fn remove_device(&self, id: Id) {
        self.set(|s| s.list.remove(&id));
    }
    pub fn has_device(&self, id: Id) -> bool {
        self.set(|s| s.list.contains_key(&id))
    }
    pub fn add_tap(&self, id: Id, name: &str) {
        self.set(|s| s.tap_list.insert(id, (format!("UUID-{id}"), name.into(), 0)));
    }
    /// The devices with this UID.
    pub fn ours(&self, uid: &str) -> Vec<Id> {
        self.set(|s| s.list.iter().filter(|(_, (_, u))| u == uid).map(|(id, _)| *id).collect())
    }
    pub fn names_of(&self, uid: &str) -> Vec<String> {
        self.set(|s| s.list.values().filter(|(_, u)| u == uid).map(|(n, _)| n.clone()).collect())
    }
    pub fn all_devices(&self) -> Vec<DeviceInfo> {
        self.set(|s| {
            s.list.iter().map(|(id, (n, u))| DeviceInfo { id: *id, name: n.clone(), uid: u.clone() }).collect()
        })
    }
    pub fn created(&self) -> Vec<(String, String, String, Option<String>)> {
        self.set(|s| s.created.clone())
    }
    pub fn taps_now(&self) -> Vec<TapInfo> {
        self.set(|s| {
            s.tap_list.iter().map(|(id, (u, n, _))| TapInfo { id: *id, uuid: u.clone(), name: n.clone() }).collect()
        })
    }
    pub fn tap_process(&self, tap: Id) -> Option<Id> {
        self.set(|s| s.tap_list.get(&tap).map(|t| t.2))
    }
    /// The process object the fake hands out for `pid`.
    pub fn process_of(&self, pid: i32) -> Id {
        1000 + pid as Id
    }
    pub fn measured(&self) -> usize {
        self.set(|s| s.measured)
    }
    pub fn order(&self) -> Vec<String> {
        self.set(|s| s.order.clone())
    }
}

impl CoreAudio for FakeCoreAudio {
    fn devices(&mut self) -> Result<Vec<DeviceInfo>, String> {
        Ok(self.all_devices())
    }

    fn create_aggregate(&mut self, name: &str, uid: &str, sub_uid: &str, tap_uuid: Option<&str>) -> Result<Id, Status> {
        self.set(|s| {
            if let Some(e) = s.fail_create {
                return Err(e);
            }
            s.next += 1;
            let id = s.next;
            s.list.insert(id, (name.into(), uid.into()));
            s.created.push((name.into(), uid.into(), sub_uid.into(), tap_uuid.map(str::to_string)));
            s.order.push("dev+".into());
            Ok(id)
        })
    }

    fn destroy_aggregate(&mut self, dev: Id) -> Status {
        self.set(|s| {
            if s.list.remove(&dev).is_none() {
                return -1;
            }
            s.destroyed.push(dev);
            s.order.push("dev-".into());
            0
        })
    }

    fn process_object(&mut self, pid: i32) -> Option<Id> {
        Some(self.process_of(pid))
    }

    fn tap_output_uid(&mut self) -> Option<String> {
        self.set(|s| s.default_output.clone())
    }

    fn create_tap(&mut self, name: &str, process: Id) -> Result<(Id, String), String> {
        self.set(|s| {
            if let Some(e) = &s.fail_tap {
                return Err(e.clone());
            }
            s.next += 1;
            let id = s.next;
            let uuid = format!("TAP-UUID-{id}");
            s.tap_list.insert(id, (uuid.clone(), name.into(), process));
            s.order.push("tap+".into());
            Ok((id, uuid))
        })
    }

    fn destroy_tap(&mut self, tap: Id) -> Status {
        self.set(|s| {
            if s.tap_list.remove(&tap).is_none() {
                return -1;
            }
            s.order.push("tap-".into());
            0
        })
    }

    fn taps(&mut self) -> Result<Vec<TapInfo>, String> {
        Ok(self.taps_now())
    }

    fn input_users(&mut self, dev: Id) -> Option<Vec<i32>> {
        self.set(|s| if s.per_process { Some(s.readers.get(&dev).cloned().unwrap_or_default()) } else { None })
    }

    fn measure_input(&mut self, _dev: Id, _uid: &str, _out: &str, _secs: f64, amp: f32) -> Result<Measure, String> {
        self.set(|s| {
            s.measured += 1;
            if let Some(m) = &s.measure {
                return Ok(m.clone());
            }
            Ok(if s.tap_hears {
                Measure { callbacks: 9, rms: amp as f64 / std::f64::consts::SQRT_2, first_ms: Some(21.0), ms: 120.0 }
            } else {
                Measure { callbacks: 9, rms: 0.0, first_ms: None, ms: 1500.0 }
            })
        })
    }
}

// --- the output side ---

struct StreamState {
    dev: OutDevice,
    render: Render,
    closed: bool,
    dead: bool,
}

/// An open fake output stream: [`FakeStreamHandle::play`] runs the audio callback.
#[derive(Clone)]
pub struct FakeStreamHandle {
    s: Arc<Mutex<StreamState>>,
}

impl FakeStreamHandle {
    /// `frames` frames of output (interleaved, the device's channels).
    pub fn play(&self, frames: usize) -> Vec<f32> {
        let mut s = self.s.lock().unwrap();
        let ch = s.dev.channels.max(1);
        let mut buf = vec![0.0; frames * ch];
        if !s.closed {
            (s.render)(&mut buf, ch);
        }
        buf
    }
    pub fn closed(&self) -> bool {
        self.s.lock().unwrap().closed
    }
    pub fn device(&self) -> OutDevice {
        self.s.lock().unwrap().dev.clone()
    }
    /// The device went away under it.
    pub fn kill(&self) {
        self.s.lock().unwrap().dead = true;
    }
}

struct FakeStream {
    s: Arc<Mutex<StreamState>>,
}

impl OutStream for FakeStream {
    fn alive(&self) -> bool {
        !self.s.lock().unwrap().dead
    }
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        if let Ok(mut s) = self.s.lock() {
            s.closed = true;
        }
    }
}

#[derive(Default)]
struct OutState {
    devices: Vec<OutDevice>,
    streams: Vec<FakeStreamHandle>,
    fail_open: Option<String>,
}

/// Output devices in memory; the first one is the default output.
#[derive(Clone, Default)]
pub struct FakeOutputs {
    s: Arc<Mutex<OutState>>,
}

impl FakeOutputs {
    pub fn new(devices: Vec<OutDevice>) -> Self {
        let me = Self::default();
        me.set_devices(devices);
        me
    }
    pub fn speakers() -> OutDevice {
        OutDevice { uid: "Speakers_UID".into(), name: "MacBook Pro Speakers".into(), rate: 48000, channels: 2 }
    }
    pub fn blackhole() -> OutDevice {
        OutDevice { uid: "BlackHole2ch_UID".into(), name: "BlackHole 2ch".into(), rate: 48000, channels: 2 }
    }
    pub fn set_devices(&self, d: Vec<OutDevice>) {
        self.s.lock().unwrap().devices = d;
    }
    pub fn fail_open(&self, why: Option<&str>) {
        self.s.lock().unwrap().fail_open = why.map(str::to_string);
    }
    pub fn streams(&self) -> Vec<FakeStreamHandle> {
        self.s.lock().unwrap().streams.clone()
    }
    pub fn stream(&self, i: usize) -> FakeStreamHandle {
        self.streams()[i].clone()
    }
    pub fn last_stream(&self) -> Option<FakeStreamHandle> {
        self.streams().last().cloned()
    }
    /// Streams still open.
    pub fn open_count(&self) -> usize {
        self.streams().iter().filter(|s| !s.closed()).count()
    }
}

impl Outputs for FakeOutputs {
    fn tap_output(&self) -> Option<OutDevice> {
        self.s.lock().unwrap().devices.first().cloned()
    }
    fn find(&self, name: &str) -> Option<OutDevice> {
        let n = name.to_lowercase();
        self.s.lock().unwrap().devices.iter().find(|d| d.name.to_lowercase().contains(&n)).cloned()
    }
    fn open(&self, dev: &OutDevice, render: Render) -> Result<Box<dyn OutStream>, String> {
        let mut s = self.s.lock().unwrap();
        if let Some(e) = &s.fail_open {
            return Err(e.clone());
        }
        let st = Arc::new(Mutex::new(StreamState { dev: dev.clone(), render, closed: false, dead: false }));
        s.streams.push(FakeStreamHandle { s: st.clone() });
        Ok(Box::new(FakeStream { s: st }))
    }
}
