//! Core Audio process taps (macOS 14.2+): `CATapDescription` + `AudioHardwareCreateProcessTap`.
//! Same tap as `CoreAudioDevices.create_tap()` in stick_mic_device.py: a public, muting, stereo
//! mixdown of one process (a private tap gives a public aggregate no input channels).

use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyClass;
use objc2_core_audio::{AudioHardwareCreateProcessTap, AudioHardwareDestroyProcessTap, CATapDescription, CATapMuteBehavior};
use objc2_foundation::{NSArray, NSNumber, NSProcessInfo, NSString};

use crate::hal::{Id, Status};

pub const TAP_MIN_MACOS: (isize, isize) = (14, 2);

/// (major, minor, patch) of the running macOS.
pub fn macos_version() -> (isize, isize, isize) {
    let v = NSProcessInfo::processInfo().operatingSystemVersion();
    (v.majorVersion, v.minorVersion, v.patchVersion)
}

/// Ok when this Mac has process taps (14.2+ and the CATapDescription class is there). Checked
/// before any message is sent: objc2 panics on a missing class.
pub fn available() -> Result<(), String> {
    let (maj, min, _) = macos_version();
    if (maj, min) < TAP_MIN_MACOS {
        return Err(format!("process taps need macOS {}.{} or newer (this is {maj}.{min})", TAP_MIN_MACOS.0, TAP_MIN_MACOS.1));
    }
    if AnyClass::get(c"CATapDescription").is_none() {
        return Err("CATapDescription isn't available on this Mac".into());
    }
    Ok(())
}

/// A created tap and what its description said (read back through the getters).
#[derive(Debug, Clone)]
pub struct Tap {
    pub id: Id,
    /// The description's UUID: what the aggregate's tap list refers to.
    pub uuid: String,
    pub name: String,
    /// `isMuted` read back (the getter of `setMuteBehavior:`): 1 = CATapMuted.
    pub mute_behavior: isize,
    pub private: bool,
    pub mono: bool,
    pub mixdown: bool,
}

/// Creates a public, muted (CATapMuted), stereo-mixdown tap of the Core Audio process `process`.
pub fn create_process_tap(name: &str, process: Id) -> Result<Tap, String> {
    available()?;
    autoreleasepool(|_| {
        let procs = NSArray::from_retained_slice(&[NSNumber::new_u32(process)]);
        // SAFETY: plain Objective-C calls on a freshly allocated CATapDescription; the class exists
        // (checked by available()).
        let desc = unsafe { CATapDescription::initStereoMixdownOfProcesses(CATapDescription::alloc(), &procs) };
        unsafe {
            desc.setMuteBehavior(CATapMuteBehavior::Muted);
            desc.setPrivate(false);
            desc.setName(&NSString::from_str(name));
        }
        let uuid = unsafe { desc.UUID() }.UUIDString().to_string();
        let (mute_behavior, private, mono, mixdown) =
            unsafe { (desc.isMuted().0, desc.isPrivate(), desc.isMono(), desc.isMixdown()) };
        let mut id: Id = 0;
        let err = unsafe { AudioHardwareCreateProcessTap(Some(&desc), &mut id) };
        if err != 0 || id == 0 {
            return Err(format!("can't create the audio tap (CoreAudio error {})", crate::hal::status_text(err)));
        }
        Ok(Tap { id, uuid, name: name.to_string(), mute_behavior, private, mono, mixdown })
    })
}

pub fn destroy_process_tap(id: Id) -> Status {
    unsafe { AudioHardwareDestroyProcessTap(id) }
}
