//! Benchmarks one STT engine on the spike's test WAVs and writes JSON for `score.py`.
//!
//! stt-bench --engine sherpa  --model DIR  [--provider cpu|coreml] [--threads N] --wavs DIR --out FILE [--runs 5]
//! stt-bench --engine whisper --model FILE [--threads N]                         --wavs DIR --out FILE [--runs 5]
//!
//! Timings: `load_s` = Stt::load(); `first_s` = the first real transcription after load;
//! `warm_s` = `--runs` repeats per utterance after that. Peak memory comes from `/usr/bin/time -l`
//! (wrap the command with `stt-testset/with_mem.py`).

use std::{collections::HashMap, fs, path::{Path, PathBuf}, time::Instant};

use anyhow::{bail, Context, Result};
use serde_json::json;
use stt::Stt;

fn read_wav(p: &Path) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(p).with_context(|| format!("{p:?}"))?;
    let spec = r.spec();
    if spec.channels != 1 || spec.bits_per_sample != 16 {
        bail!("{p:?}: want mono 16-bit");
    }
    let s = r.samples::<i16>().map(|s| s.map(|v| v as f32 / 32768.0)).collect::<Result<_, _>>()?;
    Ok((s, spec.sample_rate))
}

fn dir_bytes(p: &Path) -> u64 {
    if p.is_file() {
        return p.metadata().map(|m| m.len()).unwrap_or(0);
    }
    fs::read_dir(p).map(|d| d.flatten().map(|e| dir_bytes(&e.path())).sum()).unwrap_or(0)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut a: HashMap<String, String> = HashMap::new();
    for w in args[1..].chunks(2) {
        a.insert(w[0].trim_start_matches("--").into(), w.get(1).cloned().unwrap_or_default());
    }
    let get = |k: &str| a.get(k).cloned().with_context(|| format!("--{k} is required"));
    let engine = get("engine")?;
    let model = PathBuf::from(get("model")?);
    let wavs = PathBuf::from(get("wavs")?);
    let out = PathBuf::from(get("out")?);
    let runs: usize = a.get("runs").map(|s| s.parse()).transpose()?.unwrap_or(5);
    let threads: i32 = a.get("threads").map(|s| s.parse()).transpose()?.unwrap_or(4);
    let label = a.get("label").cloned();

    let mut stt: Box<dyn Stt> = match engine.as_str() {
        #[cfg(any(feature = "sherpa", feature = "sherpa-shared"))]
        "sherpa" => Box::new(stt::sherpa::SherpaParakeet::new(
            &model,
            a.get("provider").map(String::as_str).unwrap_or("cpu"),
            threads,
        )),
        #[cfg(feature = "whisper")]
        "whisper" => Box::new(stt::whisper::Whisper::new(&model, threads)),
        #[cfg(feature = "fluid")]
        "fluid" => Box::new(stt::fluid::Fluid::new(&model, a.get("units").map(String::as_str) == Some("gpu"))),
        e => bail!("unknown engine {e} (or not compiled in)"),
    };
    let _ = threads; // unused when only `fluid` is compiled in

    let t = Instant::now();
    stt.load()?;
    let load_s = t.elapsed().as_secs_f64();

    let mut ids: Vec<String> = fs::read_dir(&wavs)?
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".wav").map(String::from))
        .collect();
    ids.sort();
    let audio: Vec<(Vec<f32>, u32)> = ids.iter().map(|i| read_wav(&wavs.join(format!("{i}.wav")))).collect::<Result<_>>()?;

    let t = Instant::now();
    stt.transcribe(&audio[0].0, audio[0].1)?;
    let first_s = t.elapsed().as_secs_f64();

    let mut results = vec![];
    for (id, (samples, rate)) in ids.iter().zip(&audio) {
        let mut times = vec![];
        let mut text = String::new();
        for _ in 0..runs {
            let t = Instant::now();
            text = stt.transcribe(samples, *rate)?;
            times.push(t.elapsed().as_secs_f64());
        }
        eprintln!("{id}: {:.0} ms  {text}", times.iter().cloned().fold(f64::MAX, f64::min) * 1e3);
        results.push(json!({"id": id, "audio_s": samples.len() as f64 / *rate as f64, "warm_s": times, "text": text}));
    }
    let model_name = model.file_name().unwrap_or_default().to_string_lossy().into_owned();
    fs::write(
        &out,
        serde_json::to_string_pretty(&json!({
            "engine": label.unwrap_or_else(|| stt.name()),
            "model": model_name,
            "model_bytes": dir_bytes(&model),
            "load_s": load_s,
            "first_s": first_s,
            "results": results,
        }))?,
    )?;
    Ok(())
}
