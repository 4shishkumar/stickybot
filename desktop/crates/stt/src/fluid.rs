//! FluidAudio (Swift, CoreML Parakeet TDT v3 on the Neural Engine) through a small C-ABI static lib
//! (`swift/FluidBridge`), built and linked by build.rs.

use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::PathBuf,
};

use anyhow::{anyhow, Result};

use crate::{resample_to_16k, too_short, Stt};

extern "C" {
    fn fluid_load(dir: *const c_char, units: i32) -> *mut c_void;
    fn fluid_transcribe(h: *mut c_void, samples: *const f32, n: usize) -> *mut c_char;
    fn fluid_free_string(s: *mut c_char);
    fn fluid_unload(h: *mut c_void);
    fn fluid_last_error() -> *const c_char;
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(fluid_last_error()).to_string_lossy().into_owned() }
}

pub struct Fluid {
    /// Folder FluidAudio downloads the CoreML models into (first load only).
    pub dir: PathBuf,
    /// Encoder on the GPU instead of the Neural Engine.
    pub gpu: bool,
    h: *mut c_void,
}

// the handle is only used through &mut self; FluidAudio's AsrManager is an actor
unsafe impl Send for Fluid {}

impl Fluid {
    pub fn new(dir: impl Into<PathBuf>, gpu: bool) -> Self {
        Self { dir: dir.into(), gpu, h: std::ptr::null_mut() }
    }
}

impl Stt for Fluid {
    fn load(&mut self) -> Result<()> {
        let dir = CString::new(self.dir.to_string_lossy().as_bytes())?;
        let h = unsafe { fluid_load(dir.as_ptr(), self.gpu as i32) };
        if h.is_null() {
            return Err(anyhow!("FluidAudio: {}", last_error()));
        }
        self.h = h;
        Ok(())
    }

    fn transcribe(&mut self, audio: &[f32], sample_rate: u32) -> Result<String> {
        if self.h.is_null() {
            return Err(anyhow!("speech model not loaded yet"));
        }
        if too_short(audio, sample_rate) {
            return Ok(String::new());
        }
        let audio = resample_to_16k(audio, sample_rate);
        let s = unsafe { fluid_transcribe(self.h, audio.as_ptr(), audio.len()) };
        if s.is_null() {
            return Err(anyhow!("FluidAudio: {}", last_error()));
        }
        let text = unsafe { CStr::from_ptr(s).to_string_lossy().trim().to_string() };
        unsafe { fluid_free_string(s) };
        Ok(text)
    }

    fn name(&self) -> String {
        format!("FluidAudio Parakeet v3 (CoreML, encoder on {})", if self.gpu { "GPU" } else { "ANE" })
    }
}

impl Drop for Fluid {
    fn drop(&mut self) {
        if !self.h.is_null() {
            unsafe { fluid_unload(self.h) };
        }
    }
}
