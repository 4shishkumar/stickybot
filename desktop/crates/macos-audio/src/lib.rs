//! StickyBot's "wireless mic" virtual microphone, from Rust (Phase-0 spike).
//!
//! A Core Audio *process tap* of our own process (stereo mixdown, muted, public) inside a public
//! *aggregate device* clocked by the default output: what this process plays to the default output
//! is muted on the speakers and becomes the aggregate's input, which Wispr Flow records.
//! Port of `mac-agent/stick_agent/stick_mic_device.py` (tap carrier). macOS 14.2+, and the
//! process needs the "System Audio Recording" permission (until then a tap is silent and does not
//! mute — always run the -80 dBFS probe first; see `examples/tapmic.rs`).

pub mod level;
pub mod naming;
pub mod plist;

#[cfg(target_os = "macos")]
pub mod hal;
#[cfg(target_os = "macos")]
pub mod mic;
#[cfg(target_os = "macos")]
pub mod tap;
