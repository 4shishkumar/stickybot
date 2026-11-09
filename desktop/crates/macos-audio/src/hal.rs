//! Thin, safe wrappers over the Core Audio HAL property API (via objc2-core-audio's bindings).
//! The Rust side of `CoreAudioDevices` in stick_mic_device.py: listing devices and taps,
//! the default output, PID -> process object, creating / destroying aggregates.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_audio::{
    AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertySelector,
    AudioObjectSetPropertyData, kAudioDevicePropertyDeviceUID, kAudioDevicePropertyNominalSampleRate,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices, kAudioHardwarePropertyTapList,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain, kAudioObjectPropertyName,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioTapPropertyUID,
};
use objc2_core_foundation::{CFRetained, CFString};

use crate::plist::Plist;

pub use crate::ca::{Id, Status};
const _: () = assert!(size_of::<AudioObjectID>() == size_of::<Id>());

pub const SYSTEM: Id = kAudioObjectSystemObject as Id;
/// kAudioHardwarePropertyRunLoop ('rnlp'): only in the "deprecated" header group of the bindings.
const PROPERTY_RUN_LOOP: AudioObjectPropertySelector = u32::from_be_bytes(*b"rnlp");

fn addr(sel: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    addr_in(sel, kAudioObjectPropertyScopeGlobal)
}

fn addr_in(sel: AudioObjectPropertySelector, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: sel, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

const fn fcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// A property's raw bytes (global scope), with optional qualifier data.
pub fn get_bytes(obj: Id, sel: AudioObjectPropertySelector, qualifier: Option<&[u8]>) -> Result<Vec<u8>, Status> {
    get_bytes_in(obj, sel, kAudioObjectPropertyScopeGlobal, qualifier)
}

/// A property's raw bytes in `scope`, with optional qualifier data.
pub fn get_bytes_in(
    obj: Id,
    sel: AudioObjectPropertySelector,
    scope: u32,
    qualifier: Option<&[u8]>,
) -> Result<Vec<u8>, Status> {
    let a = addr_in(sel, scope);
    let (qn, qp) = match qualifier {
        Some(q) => (q.len() as u32, q.as_ptr() as *const c_void),
        None => (0, std::ptr::null()),
    };
    let mut n: u32 = 0;
    // SAFETY: valid address / out pointers for the duration of the call.
    let err = unsafe { AudioObjectGetPropertyDataSize(obj, NonNull::from(&a), qn, qp, NonNull::from(&mut n)) };
    if err != 0 {
        return Err(err);
    }
    // u64-backed so pointer / f64 / u32 reads are aligned.
    let mut buf = vec![0u64; (n as usize).div_ceil(8).max(1)];
    let err = unsafe {
        AudioObjectGetPropertyData(
            obj,
            NonNull::from(&a),
            qn,
            qp,
            NonNull::from(&mut n),
            NonNull::new(buf.as_mut_ptr() as *mut c_void).unwrap(),
        )
    };
    if err != 0 {
        return Err(err);
    }
    let bytes: Vec<u8> = buf.iter().flat_map(|w| w.to_ne_bytes()).take(n as usize).collect();
    Ok(bytes)
}

fn u32s(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect()
}

pub fn get_u32(obj: Id, sel: AudioObjectPropertySelector) -> Option<u32> {
    get_bytes(obj, sel, None).ok().and_then(|b| u32s(&b).first().copied())
}

/// A CFString property (returned +1 retained by the HAL; released here).
pub fn get_string(obj: Id, sel: AudioObjectPropertySelector) -> Option<String> {
    let b = get_bytes(obj, sel, None).ok()?;
    if b.len() < size_of::<usize>() {
        return None;
    }
    let p = usize::from_ne_bytes(b[..size_of::<usize>()].try_into().unwrap()) as *mut CFString;
    let p = NonNull::new(p)?;
    // SAFETY: the HAL hands out a retained CFStringRef ("create rule").
    let s = unsafe { CFRetained::from_raw(p) };
    Some(s.to_string())
}

pub use crate::ca::DeviceInfo;

pub fn device_ids() -> Vec<Id> {
    get_bytes(SYSTEM, kAudioHardwarePropertyDevices, None).map(|b| u32s(&b)).unwrap_or_default()
}

/// Every audio device: (id, name, uid).
pub fn devices() -> Vec<DeviceInfo> {
    device_ids()
        .into_iter()
        .map(|id| DeviceInfo {
            id,
            name: get_string(id, kAudioObjectPropertyName).unwrap_or_default(),
            uid: get_string(id, kAudioDevicePropertyDeviceUID).unwrap_or_default(),
        })
        .collect()
}

pub use crate::ca::TapInfo;

/// Every process tap on this Mac (any process's public ones).
pub fn taps() -> Vec<TapInfo> {
    get_bytes(SYSTEM, kAudioHardwarePropertyTapList, None)
        .map(|b| u32s(&b))
        .unwrap_or_default()
        .into_iter()
        .map(|id| TapInfo {
            id,
            uuid: get_string(id, kAudioTapPropertyUID).unwrap_or_default(),
            name: get_string(id, kAudioObjectPropertyName).unwrap_or_default(),
        })
        .collect()
}

/// The default output device's UID.
pub fn default_output_uid() -> Option<String> {
    let dev = get_u32(SYSTEM, kAudioHardwarePropertyDefaultOutputDevice).filter(|d| *d != 0)?;
    get_string(dev, kAudioDevicePropertyDeviceUID).filter(|s| !s.is_empty())
}

/// kAudioDevicePropertyTransportType ('blue', 'blea', 'bltn', 'usb ', …).
pub fn transport_type(dev: Id) -> Option<u32> {
    get_u32(dev, fcc(b"tran"))
}

/// The device has output streams.
pub fn has_output(dev: Id) -> bool {
    get_bytes_in(dev, fcc(b"stm#"), fcc(b"outp"), None).is_ok_and(|b| b.len() >= 4)
}

/// The output the tap's device is clocked by, and that the stick's audio is played into (muted by
/// the tap): the default output, unless it's Bluetooth — then the built-in output.
///
/// Why: an aggregate whose clock is a Bluetooth output (AirPods, earbuds) never runs its input on
/// macOS 27 (0 callbacks), and audio played into Bluetooth reaches the tap only after ~1.1 s (the
/// headphones' start-up), which would cut the start of every dictation. Clocked by and played into
/// the built-in speakers it works within ~21 ms; the tap mutes it there just the same (the probe
/// checks exactly this path before anything is played).
pub fn tap_output_uid() -> Option<String> {
    let default = get_u32(SYSTEM, kAudioHardwarePropertyDefaultOutputDevice).filter(|d| *d != 0)?;
    let bluetooth = matches!(transport_type(default), Some(t) if t == fcc(b"blue") || t == fcc(b"blea"));
    let dev = if bluetooth {
        device_ids().into_iter().find(|d| transport_type(*d) == Some(fcc(b"bltn")) && has_output(*d)).unwrap_or(default)
    } else {
        default
    };
    get_string(dev, kAudioDevicePropertyDeviceUID).filter(|s| !s.is_empty())
}

/// The Core Audio process object of `pid` (kAudioHardwarePropertyTranslatePIDToProcessObject).
pub fn process_object(pid: i32) -> Option<Id> {
    let b = get_bytes(SYSTEM, kAudioHardwarePropertyTranslatePIDToProcessObject, Some(&pid.to_ne_bytes())).ok()?;
    u32s(&b).first().copied().filter(|p| *p != 0)
}

pub fn nominal_rate(dev: Id) -> Option<f64> {
    let b = get_bytes(dev, kAudioDevicePropertyNominalSampleRate, None).ok()?;
    Some(f64::from_ne_bytes(b.get(..8)?.try_into().ok()?))
}

/// PIDs of the processes running input on `dev` right now (per-process audio objects, macOS
/// 14.2+), or `None` when macOS can't tell. (stick_mic_device.py `input_users`)
pub fn input_users(dev: Id) -> Option<Vec<i32>> {
    let procs = get_bytes(SYSTEM, fcc(b"prs#"), None).ok()?; // kAudioHardwarePropertyProcessObjectList
    let mut pids = Vec::new();
    for p in u32s(&procs) {
        if get_u32(p, fcc(b"piri")).unwrap_or(0) == 0 {
            continue; // kAudioProcessPropertyIsRunningInput
        }
        // kAudioProcessPropertyDevices, input scope
        let devs = get_bytes_in(p, fcc(b"pdv#"), fcc(b"inpt"), None).map(|b| u32s(&b)).unwrap_or_default();
        if devs.contains(&dev)
            && let Some(pid) = get_u32(p, fcc(b"ppid"))
        {
            pids.push(pid as i32); // kAudioProcessPropertyPID
        }
    }
    Some(pids)
}

/// Let the HAL run its notifications on its own thread (kAudioHardwarePropertyRunLoop = NULL), so
/// creating a device never waits for a run loop on the calling thread. (Python does the same.)
pub fn use_hal_thread_for_notifications() -> Status {
    let a = addr(PROPERTY_RUN_LOOP);
    let none: *const c_void = std::ptr::null();
    unsafe {
        AudioObjectSetPropertyData(
            SYSTEM,
            NonNull::from(&a),
            0,
            std::ptr::null(),
            size_of::<*const c_void>() as u32,
            NonNull::from(&none).cast(),
        )
    }
}

/// `AudioHardwareCreateAggregateDevice` with a description from [`crate::plist::aggregate_description`].
pub fn create_aggregate(desc: &Plist) -> Result<Id, Status> {
    let d = desc.to_cf_dict().ok_or(-50)?; // paramErr
    let mut dev: Id = 0;
    let err = unsafe { AudioHardwareCreateAggregateDevice(d.as_ref(), NonNull::from(&mut dev)) };
    if err != 0 || dev == 0 { Err(if err != 0 { err } else { -1 }) } else { Ok(dev) }
}

pub fn destroy_aggregate(dev: Id) -> Status {
    unsafe { AudioHardwareDestroyAggregateDevice(dev) }
}

pub use crate::ca::status_text;
