//! Speech to text for the StickyBot desktop app — Phase-0 spike.
//!
//! The stick sends 16 kHz mono IMA-ADPCM; the app decodes it to `f32` in [-1, 1]
//! (see `mac-agent/stick_agent/adpcm.py`), which is exactly what [`Stt::transcribe`] takes.
//!
//! Backends (cargo features):
//! - `sherpa`  — sherpa-onnx (official k2-fsa Rust crate), Parakeet TDT 0.6b v3 int8 ONNX.
//! - `whisper` — whisper.cpp via `whisper-rs`, Metal.
//! - `fluid`   — FluidAudio (Swift; CoreML Parakeet TDT v3 on the Neural Engine) via a C-ABI static lib.
//! - `sherpa-shared` — `sherpa` linked against shared onnxruntime, which has the CoreML EP.

use anyhow::Result;

#[cfg(any(feature = "sherpa", feature = "sherpa-shared"))]
pub mod sherpa;
#[cfg(feature = "whisper")]
pub mod whisper;
#[cfg(feature = "fluid")]
pub mod fluid;

/// What the app needs from a speech engine: load once, then transcribe short clips.
pub trait Stt: Send {
    /// Loads the model (slow: do it once, off the UI thread).
    fn load(&mut self) -> Result<()>;
    /// Transcribes mono `f32` samples in [-1, 1]. Returns trimmed text ("" for silence).
    fn transcribe(&mut self, audio: &[f32], sample_rate: u32) -> Result<String>;
    /// Human-readable engine + model name.
    fn name(&self) -> String;
}

/// The engines' native rate.
pub const RATE: u32 = 16_000;

/// Linear resampler — good enough for speech when the input isn't 16 kHz already (the stick's is).
pub fn resample_to_16k(audio: &[f32], sample_rate: u32) -> std::borrow::Cow<'_, [f32]> {
    if sample_rate == RATE || audio.is_empty() {
        return std::borrow::Cow::Borrowed(audio);
    }
    let ratio = sample_rate as f64 / RATE as f64;
    let n = (audio.len() as f64 / ratio).floor() as usize;
    let out = (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let j = x as usize;
            let f = (x - j as f64) as f32;
            let a = audio[j];
            let b = *audio.get(j + 1).unwrap_or(&a);
            a + (b - a) * f
        })
        .collect();
    std::borrow::Cow::Owned(out)
}

/// Under 0.1 s there's nothing to hear (same rule as the Python agent).
pub(crate) fn too_short(audio: &[f32], sample_rate: u32) -> bool {
    (audio.len() as u64) * 10 < sample_rate as u64
}
