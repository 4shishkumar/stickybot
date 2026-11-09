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

pub type Id = AudioObjectID;
/// An OSStatus (0 = noErr).
pub type Status = i32;

pub const SYSTEM: Id = kAudioObjectSystemObject as Id;
/// kAudioHardwarePropertyRunLoop ('rnlp'): only in the "deprecated" header group of the bindings.
const PROPERTY_RUN_LOOP: AudioObjectPropertySelector = u32::from_be_bytes(*b"rnlp");

fn addr(sel: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: sel,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// A property's raw bytes (global scope), with optional qualifier data.
pub fn get_bytes(obj: Id, sel: AudioObjectPropertySelector, qualifier: Option<&[u8]>) -> Result<Vec<u8>, Status> {
    let a = addr(sel);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: Id,
    pub name: String,
    pub uid: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapInfo {
    pub id: Id,
    pub uuid: String,
    pub name: String,
}

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

/// The Core Audio process object of `pid` (kAudioHardwarePropertyTranslatePIDToProcessObject).
pub fn process_object(pid: i32) -> Option<Id> {
    let b = get_bytes(SYSTEM, kAudioHardwarePropertyTranslatePIDToProcessObject, Some(&pid.to_ne_bytes())).ok()?;
    u32s(&b).first().copied().filter(|p| *p != 0)
}

pub fn nominal_rate(dev: Id) -> Option<f64> {
    let b = get_bytes(dev, kAudioDevicePropertyNominalSampleRate, None).ok()?;
    Some(f64::from_ne_bytes(b.get(..8)?.try_into().ok()?))
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

/// A four-char-code status as text when it is one ("!obj"), else the number.
pub fn status_text(s: Status) -> String {
    let b = (s as u32).to_be_bytes();
    if b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
        format!("'{}' ({s})", String::from_utf8_lossy(&b))
    } else {
        s.to_string()
    }
}
