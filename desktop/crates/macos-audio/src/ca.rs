//! The few Core Audio calls the stick mic needs, behind a trait (`CoreAudioDevices` in
//! stick_mic_device.py): the real one is [`crate::system::SystemCoreAudio`] (macOS), tests use
//! [`crate::fake::FakeCoreAudio`]. Plain Rust: builds and tests anywhere.

/// An `AudioObjectID`.
pub type Id = u32;
/// An OSStatus (0 = noErr).
pub type Status = i32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: Id,
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapInfo {
    pub id: Id,
    pub uuid: String,
    pub name: String,
}

/// What the check device's input heard while the probe tone played (`measure_input`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measure {
    /// input callbacks (0 = the device never ran)
    pub callbacks: u64,
    /// RMS of the input after the first signal (0 = silence)
    pub rms: f64,
    pub first_ms: Option<f64>,
    pub ms: f64,
}

/// Core Audio, as far as the "<stick>-bt-mic" device goes. Every call is synchronous and quick
/// (a few HAL calls), except [`CoreAudio::measure_input`] (≤ `secs`).
pub trait CoreAudio: Send {
    /// Every audio device.
    fn devices(&mut self) -> Result<Vec<DeviceInfo>, String>;
    /// A public aggregate with one sub-device `sub_uid` (also its clock), and with `tap_uuid` the
    /// process tap whose audio is its input (drift-compensated, started with the device).
    fn create_aggregate(&mut self, name: &str, uid: &str, sub_uid: &str, tap_uuid: Option<&str>) -> Result<Id, Status>;
    fn destroy_aggregate(&mut self, dev: Id) -> Status;
    /// The Core Audio process object of `pid`.
    fn process_object(&mut self, pid: i32) -> Option<Id>;
    /// The output the tap's device is clocked by and the stick's audio is played into: the default
    /// output, or the built-in one when the default is Bluetooth (see `hal::tap_output_uid`).
    fn tap_output_uid(&mut self) -> Option<String>;
    /// A public, muting (CATapMuted), stereo-mixdown tap of one process -> (tap id, its UUID).
    fn create_tap(&mut self, name: &str, process: Id) -> Result<(Id, String), String>;
    fn destroy_tap(&mut self, tap: Id) -> Status;
    /// Every process tap on this Mac (any process's public ones).
    fn taps(&mut self) -> Result<Vec<TapInfo>, String>;
    /// PIDs of the processes running input on `dev` right now; `None` when macOS can't tell.
    fn input_users(&mut self, dev: Id) -> Option<Vec<i32>>;
    /// Plays a `freq` sine of `amp` from this process into the default output (`out_uid`, which the
    /// tap mutes and captures) and measures what the device `dev_uid` hears, for up to `secs`.
    fn measure_input(&mut self, dev: Id, dev_uid: &str, out_uid: &str, secs: f64, amp: f32) -> Result<Measure, String>;
}

/// Makes the Core Audio backend on first use (so creating a device object never touches audio).
pub type Factory = std::sync::Arc<dyn Fn() -> Result<Box<dyn CoreAudio>, String> + Send + Sync>;

/// A four-char-code status as text when it is one ("'!obj' (…)"), else the number.
pub fn status_text(s: Status) -> String {
    let b = (s as u32).to_be_bytes();
    if b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
        format!("'{}' ({s})", String::from_utf8_lossy(&b))
    } else {
        s.to_string()
    }
}
