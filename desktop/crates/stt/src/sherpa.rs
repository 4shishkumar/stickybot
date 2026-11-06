//! sherpa-onnx + NVIDIA Parakeet TDT 0.6b v3 (int8 ONNX export by k2-fsa).
//!
//! Model dir (from sherpa-onnx's `asr-models` release):
//! `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/{encoder,decoder,joiner}.int8.onnx + tokens.txt`.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};

use crate::{too_short, Stt};

pub struct SherpaParakeet {
    pub dir: PathBuf,
    /// "cpu" or "coreml".
    pub provider: String,
    pub threads: i32,
    rec: Option<OfflineRecognizer>,
}

impl SherpaParakeet {
    pub fn new(dir: impl Into<PathBuf>, provider: &str, threads: i32) -> Self {
        Self { dir: dir.into(), provider: provider.into(), threads, rec: None }
    }
}

impl Stt for SherpaParakeet {
    fn load(&mut self) -> Result<()> {
        let p = |f: &str| Some(self.dir.join(f).to_string_lossy().into_owned());
        let mut c = OfflineRecognizerConfig::default();
        c.model_config.transducer = OfflineTransducerModelConfig {
            encoder: p("encoder.int8.onnx"),
            decoder: p("decoder.int8.onnx"),
            joiner: p("joiner.int8.onnx"),
        };
        c.model_config.tokens = p("tokens.txt");
        c.model_config.model_type = Some("nemo_transducer".into());
        c.model_config.provider = Some(self.provider.clone());
        c.model_config.num_threads = self.threads;
        c.decoding_method = Some("greedy_search".into());
        self.rec = Some(
            OfflineRecognizer::create(&c)
                .ok_or_else(|| anyhow!("sherpa-onnx couldn't create the recognizer from {:?}", self.dir))?,
        );
        Ok(())
    }

    fn transcribe(&mut self, audio: &[f32], sample_rate: u32) -> Result<String> {
        let rec = self.rec.as_ref().ok_or_else(|| anyhow!("speech model not loaded yet"))?;
        if too_short(audio, sample_rate) {
            return Ok(String::new());
        }
        let s = rec.create_stream();
        s.accept_waveform(sample_rate as i32, audio); // sherpa resamples internally if needed
        rec.decode(&s);
        Ok(s.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default())
    }

    fn name(&self) -> String {
        format!("sherpa-onnx ({}, {} threads)", self.provider, self.threads)
    }
}
