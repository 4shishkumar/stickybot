//! Sound clips for the stick's speaker (a port of `clips.py`): sound packs in
//! `~/.stickybot-rs/sounds/<pack>/` (`$STICKYBOT_RS_SOUNDS` instead, e.g. Kenney's Interface Sounds,
//! CC0). A clip is decoded (symphonia: ogg/vorbis, wav, flac, mp3, aiff), mixed to mono,
//! resampled to 16 kHz, made loud (the speaker is tiny) and sent as IMA-ADPCM packets on the
//! stick's AUDIO_IN characteristic:
//!
//! - `[0][samples u32][rate u16]` start
//! - `[1][frame]` data (the mic's frame format)
//! - `[2]` end: the stick plays it

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use stick_link::adpcm;

pub const RATE: u32 = 16_000;
pub const EXTS: &[&str] = &["ogg", "wav", "flac", "aiff", "aif", "mp3"];
pub const MAX_SECONDS: usize = 30;

pub fn sounds_dir(home: &Path) -> PathBuf {
    match std::env::var_os("STICKYBOT_RS_SOUNDS") {
        Some(d) => PathBuf::from(d),
        None => home.join("sounds"),
    }
}

pub fn is_sound(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.contains(&e.to_lowercase().as_str()))
}

/// `[{"pack", "name"}]` for every clip in the sound packs.
pub fn library(dir: &Path) -> Vec<Value> {
    let mut packs: Vec<PathBuf> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    packs.sort();
    let mut out = Vec::new();
    for pack in packs {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&pack)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_sound(p))
            .collect();
        files.sort();
        for f in files {
            out.push(json!({"pack": pack.file_name().map(|n| n.to_string_lossy()),
                            "name": f.file_stem().map(|n| n.to_string_lossy())}));
        }
    }
    out
}

/// The clip's file, only inside its pack (no path tricks).
pub fn find(dir: &Path, pack: &str, name: &str) -> Option<PathBuf> {
    let bad = |s: &str| s.is_empty() || s.contains('/') || s.contains('\\') || s == "." || s == "..";
    if bad(pack) || bad(name) {
        return None;
    }
    let d = dir.join(pack);
    let real = d.canonicalize().ok()?;
    for ext in EXTS {
        let f = d.join(format!("{name}.{ext}"));
        if f.is_file() && f.canonicalize().ok()?.parent() == Some(real.as_path()) {
            return Some(f);
        }
    }
    None
}

/// Decodes any supported file to mono f32 at its own rate.
pub fn decode(path: &Path) -> Result<(Vec<f32>, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let (id, rate) = (track.id, track.codec_params.sample_rate.unwrap_or(RATE));
    let mut dec = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("can't decode {}: {e}", path.display()))?;
    let mut mono = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let Ok(buf) = dec.decode(&packet) else { continue };
        let spec = *buf.spec();
        let ch = spec.channels.count().max(1);
        let mut s = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        s.copy_interleaved_ref(buf);
        mono.extend(s.samples().chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
        if mono.len() > rate as usize * (MAX_SECONDS + 1) {
            break;
        }
    }
    Ok((mono, rate))
}

/// Makes a clip as loud as the tiny speaker allows (clips.py `loud`): drops the bass it can't play
/// (1st-order high-pass), then compresses with a soft clipper so the average level is near full
/// scale, like a beep.
pub fn loud(x: &[f32], cutoff_hz: f32, drive: f32) -> Vec<f32> {
    if x.is_empty() {
        return Vec::new();
    }
    let a = (-2.0 * std::f32::consts::PI * cutoff_hz / RATE as f32).exp();
    let (mut px, mut py) = (0.0f32, 0.0f32);
    let y: Vec<f32> = x
        .iter()
        .map(|&v| {
            py = a * (py + v - px);
            px = v;
            py
        })
        .collect();
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let peak = if peak > 0.0 { peak } else { 1.0 };
    y.iter().map(|v| (drive * v / peak).tanh() / drive.tanh() * 0.97).collect()
}

/// A file -> 16 kHz int16, loud (clips.py `load`).
pub fn load(path: &Path, speech: bool) -> Result<Vec<i16>, String> {
    let (mono, rate) = decode(path)?;
    let mut x = stt::resample_to_16k(&mono, rate).into_owned();
    x.truncate(RATE as usize * MAX_SECONDS);
    Ok(loud(&x, 300.0, if speech { 3.0 } else { 4.0 }).iter().map(|v| (v * 32767.0) as i16).collect())
}

/// The AUDIO_IN packets for `pcm`.
pub fn packets(pcm: &[i16]) -> Vec<Vec<u8>> {
    let mut start = vec![0u8];
    start.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    start.extend_from_slice(&(RATE as u16).to_le_bytes());
    let mut out = vec![start];
    for f in adpcm::encode(pcm, 320) {
        let mut p = Vec::with_capacity(f.len() + 1);
        p.push(1);
        p.extend(f);
        out.push(p);
    }
    out.push(vec![2]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn sine(n: usize, hz: f32, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / 16000.0).sin()).collect()
    }

    /// The magnitude of one frequency (a single DFT bin over the whole signal).
    fn bin(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * hz as f64 * i as f64 / 16000.0;
            re += *v as f64 * w.cos();
            im -= *v as f64 * w.sin();
        }
        (re * re + im * im).sqrt() as f32
    }

    // --- test_clips.py ---------------------------------------------------------------------------

    #[test]
    fn loud_stays_in_range_and_gets_louder() {
        let quiet = sine(8000, 800.0, 0.1);
        let out = loud(&quiet, 400.0, 4.0);
        assert!(out.iter().all(|v| v.abs() <= 1.0));
        assert!(rms(&out) > 3.0 * rms(&quiet));
    }

    #[test]
    fn loud_removes_bass() {
        let mix: Vec<f32> = sine(16000, 60.0, 0.2).iter().zip(sine(16000, 2000.0, 0.2)).map(|(a, b)| a + b).collect();
        let out = loud(&mix, 400.0, 4.0);
        assert!(bin(&out, 60.0) < bin(&out, 2000.0) / 3.0);
    }

    #[test]
    fn loud_handles_silence_and_empty() {
        assert!(loud(&[], 400.0, 4.0).is_empty());
        assert!(loud(&[0.0; 100], 400.0, 4.0).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn packets_start_data_end() {
        let pcm: Vec<i16> = (0..5000).map(|i| ((i as f64 / 5.0).sin() * 9000.0) as i16).collect();
        let pk = packets(&pcm);
        assert_eq!(pk[0][0], 0);
        assert_eq!(u32::from_le_bytes(pk[0][1..5].try_into().unwrap()), 5000);
        assert_eq!(u16::from_le_bytes(pk[0][5..7].try_into().unwrap()), 16000);
        assert_eq!(pk.last().unwrap(), &vec![2u8]);
        let data = &pk[1..pk.len() - 1];
        assert!(data.iter().all(|p| p[0] == 1));
        assert!(data.iter().map(Vec::len).max().unwrap() <= 253); // fits one write at MTU 256
        let seqs: Vec<u16> = data.iter().map(|p| adpcm::decode_frame(&p[1..]).unwrap().0).collect();
        assert_eq!(seqs, (0..data.len() as u16).collect::<Vec<_>>());
    }

    #[test]
    fn library_and_find() {
        let dir = std::env::temp_dir().join(format!("stickybot-clips-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("kenney-test");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("ding_001.ogg"), b"x").unwrap();
        std::fs::write(pack.join("notes.txt"), b"not a sound").unwrap();
        assert_eq!(library(&dir), vec![json!({"pack": "kenney-test", "name": "ding_001"})]);
        assert_eq!(find(&dir, "kenney-test", "ding_001").unwrap().file_name().unwrap(), "ding_001.ogg");
        assert!(find(&dir, "kenney-test", "nope").is_none());
        assert!(find(&dir, "..", "etc/passwd").is_none());
    }

    #[test]
    fn a_real_wav_loads() {
        // a 44.1 kHz stereo WAV becomes 16 kHz mono, loud
        let dir = std::env::temp_dir().join(format!("stickybot-clips-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for i in 0..44100 {
            let v = ((i as f64 * 2.0 * std::f64::consts::PI * 1000.0 / 44100.0).sin() * 3000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let pcm = load(&p, false).unwrap();
        assert!((15990..=16010).contains(&pcm.len()), "{}", pcm.len());
        assert!(pcm.iter().map(|v| v.unsigned_abs()).max().unwrap() > 20000);
    }
}
