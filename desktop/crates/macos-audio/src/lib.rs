//! StickyBot's "wireless mic" virtual microphone, from Rust (Phase-0 spike).
//!
//! A Core Audio *process tap* of our own process (stereo mixdown, muted, public) inside a public
//! *aggregate device* clocked by the default output (the built-in one when the default is
//! Bluetooth): what this process plays there is muted and becomes the aggregate's input, which
//! Wispr Flow records.
//! Port of `mac-agent/stick_agent/stick_mic_device.py` (tap carrier). macOS 14.2+, and the
//! process needs the "System Audio Recording" permission (until then a tap is silent and does not
//! mute — always run the -80 dBFS probe first; see `examples/tapmic.rs`).

pub mod ca;
pub mod carrier;
pub mod device;
pub mod fake;
pub mod jitter;
pub mod level;
pub mod naming;
pub mod player;
pub mod plist;

#[cfg(target_os = "macos")]
pub mod hal;
#[cfg(target_os = "macos")]
pub mod mic;
#[cfg(target_os = "macos")]
pub mod system;
#[cfg(target_os = "macos")]
pub mod tap;

use std::sync::Arc;

/// The audio system the wireless mic uses: Core Audio (devices, taps), the output streams, and
/// whether this Mac has process taps. [`Audio::system`] is the real one; tests use [`Audio::fake`].
#[derive(Clone)]
pub struct Audio {
    pub core_audio: ca::Factory,
    pub outputs: Arc<dyn player::Outputs>,
    pub tap_supported: bool,
}

impl Audio {
    /// The real thing. Nothing touches Core Audio until it's used.
    #[cfg(target_os = "macos")]
    pub fn system() -> Self {
        Self {
            core_audio: Arc::new(|| Ok(Box::new(system::SystemCoreAudio::new()?) as Box<dyn ca::CoreAudio>)),
            outputs: Arc::new(system::CpalOutputs),
            tap_supported: tap::available().is_ok(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn system() -> Self {
        Self {
            core_audio: Arc::new(|| Err("Core Audio is macOS-only".to_string())),
            outputs: Arc::new(fake::FakeOutputs::new(vec![])),
            tap_supported: false,
        }
    }

    pub fn fake(ca: &fake::FakeCoreAudio, outputs: &fake::FakeOutputs, tap_supported: bool) -> Self {
        Self { core_audio: ca.factory(), outputs: Arc::new(outputs.clone()), tap_supported }
    }
}
