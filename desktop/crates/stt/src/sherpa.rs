//! sherpa-onnx + NVIDIA Parakeet TDT 0.6b v3 (int8 ONNX export by k2-fsa).
//!
//! Model dir (from sherpa-onnx's `asr-models` release):
//! `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/{encoder,decoder,joiner}.int8.onnx + tokens.txt`.
//!
//! **Word hints** (sherpa-onnx hotwords, contextual biasing): with [`SherpaParakeet::hints`] set,
//! decoding switches from greedy search to modified beam search (4 paths) and the hint words get a
//! bonus while they're being spelled. sherpa needs a BPE vocabulary to split the hints into the
//! model's pieces; the release has only `tokens.txt`, so `bpe.vocab` is written next to it (each
//! piece with its negated id as its score: the earlier a SentencePiece piece, the more common).
//! On the spike's 13 test sentences this fixes "Tory" → "Tauri" and changes nothing else, at the
//! same speed (~130 ms for 3 s).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};

use crate::{Stt, too_short};

pub struct SherpaParakeet {
    pub dir: PathBuf,
    /// "cpu" or "coreml".
    pub provider: String,
    pub threads: i32,
    /// Words the model should prefer when it's unsure ("Tauri" over "Tory"). Non-empty turns on
    /// modified beam search with hotwords.
    pub hints: Vec<String>,
    /// How strongly each hint piece is boosted (sherpa's `hotwords_score`).
    pub hint_score: f32,
    rec: Option<OfflineRecognizer>,
    /// the hints in sherpa's per-stream format (`/`-separated)
    hint_line: String,
}

impl SherpaParakeet {
    pub fn new(dir: impl Into<PathBuf>, provider: &str, threads: i32) -> Self {
        Self {
            dir: dir.into(),
            provider: provider.into(),
            threads,
            hints: Vec::new(),
            hint_score: 1.5,
            rec: None,
            hint_line: String::new(),
        }
    }
}

/// `tokens.txt` (`<piece> <id>` per line) as a SentencePiece `bpe.vocab` (`<piece>\t<score>`),
/// scored by order.
pub fn bpe_vocab(tokens: &str) -> String {
    let mut out = String::with_capacity(tokens.len() + tokens.len() / 4);
    for line in tokens.lines() {
        if let Some((piece, id)) = line.rsplit_once(' ')
            && let Ok(id) = id.trim().parse::<i64>()
        {
            out.push_str(&format!("{piece}\t{}\n", -id));
        }
    }
    out
}

/// Writes `bpe.vocab` next to `tokens.txt` (or in the temp folder if the model's is read-only).
fn ensure_bpe_vocab(dir: &Path) -> Result<PathBuf> {
    let tokens = std::fs::read_to_string(dir.join("tokens.txt")).context("tokens.txt")?;
    let want = bpe_vocab(&tokens);
    for path in [dir.join("bpe.vocab"), std::env::temp_dir().join("stickybot-parakeet-bpe.vocab")] {
        if std::fs::read_to_string(&path).is_ok_and(|have| have == want) || std::fs::write(&path, &want).is_ok() {
            return Ok(path);
        }
    }
    Err(anyhow!("couldn't write bpe.vocab"))
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
        self.hint_line = self
            .hints
            .iter()
            .map(|h| h.replace(['/', '\n', ':'], " ").trim().to_string())
            .filter(|h| !h.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        if !self.hint_line.is_empty() {
            c.model_config.modeling_unit = Some("bpe".into());
            c.model_config.bpe_vocab = Some(ensure_bpe_vocab(&self.dir)?.to_string_lossy().into_owned());
            c.decoding_method = Some("modified_beam_search".into());
            c.max_active_paths = 4;
            c.hotwords_score = self.hint_score;
        }
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
        let s = if self.hint_line.is_empty() {
            rec.create_stream()
        } else {
            rec.create_stream_with_hotwords(&self.hint_line)
        };
        s.accept_waveform(sample_rate as i32, audio); // sherpa resamples internally if needed
        rec.decode(&s);
        Ok(s.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default())
    }

    fn name(&self) -> String {
        let hints = if self.hints.is_empty() { String::new() } else { format!(", {} word hints", self.hints.len()) };
        format!("sherpa-onnx ({}, {} threads{hints})", self.provider, self.threads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_scored_by_order() {
        let v = bpe_vocab("<unk> 0\n\u{2581}the 7\nau 620\nbroken\n");
        assert_eq!(v, "<unk>\t0\n\u{2581}the\t-7\nau\t-620\n");
    }
}
