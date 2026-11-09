//! whisper.cpp (via `whisper-rs`, Metal) — the fallback engine.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use crate::{resample_to_16k, too_short, Stt};

pub struct Whisper {
    pub model: PathBuf,
    pub threads: i32,
    // the state borrows nothing from the context in whisper-rs 0.16 (it holds an Arc), so both can live here
    ctx: Option<WhisperContext>,
    state: Option<WhisperState>,
}

impl Whisper {
    pub fn new(model: impl Into<PathBuf>, threads: i32) -> Self {
        Self { model: model.into(), threads, ctx: None, state: None }
    }
}

impl Stt for Whisper {
    fn load(&mut self) -> Result<()> {
        let mut cp = WhisperContextParameters::default();
        cp.use_gpu(true);
        let ctx = WhisperContext::new_with_params(&self.model, cp)
            .map_err(|e| anyhow!("whisper: loading {:?}: {e}", self.model))?;
        self.state = Some(ctx.create_state().map_err(|e| anyhow!("whisper: state: {e}"))?);
        self.ctx = Some(ctx);
        Ok(())
    }

    fn transcribe(&mut self, audio: &[f32], sample_rate: u32) -> Result<String> {
        let state = self.state.as_mut().ok_or_else(|| anyhow!("speech model not loaded yet"))?;
        if too_short(audio, sample_rate) {
            return Ok(String::new());
        }
        let audio = resample_to_16k(audio, sample_rate);
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_language(Some("en"));
        p.set_n_threads(self.threads);
        p.set_no_context(true);
        p.set_single_segment(true);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        state.full(p, &audio).map_err(|e| anyhow!("whisper: {e}"))?;
        let mut text = String::new();
        for seg in state.as_iter() {
            text.push_str(&seg.to_str_lossy().map_err(|e| anyhow!("whisper: {e}"))?);
        }
        Ok(text.trim().to_string())
    }

    fn name(&self) -> String {
        format!("whisper.cpp ({})", self.model.file_name().unwrap_or_default().to_string_lossy())
    }
}
