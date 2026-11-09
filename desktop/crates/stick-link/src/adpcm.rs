//! The stick's audio frames (PROTOCOL.md "Mac link"; a port of `mac-agent/stick_agent/adpcm.py`).
//!
//! Frame: `[seq u16][predictor i16][step index u8][4-bit IMA-ADPCM samples, low nibble first]`,
//! little-endian, 16 kHz mono, ≤20 ms. Each frame carries the decoder state at its start, so frames
//! decode on their own and a lost frame just becomes 20 ms of silence.

use std::collections::BTreeMap;

pub const RATE: u32 = 16_000;
/// The header: seq, predictor, step index.
pub const HEADER: usize = 5;

const STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107,
    118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
    1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
    6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];
const ADJ: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

fn step_once(code: u8, pred: &mut i32, idx: &mut i32) -> i16 {
    let step = STEP[*idx as usize];
    let mut d = step >> 3;
    if code & 4 != 0 {
        d += step;
    }
    if code & 2 != 0 {
        d += step >> 1;
    }
    if code & 1 != 0 {
        d += step >> 2;
    }
    *pred = (if code & 8 != 0 { *pred - d } else { *pred + d }).clamp(-32768, 32767);
    *idx = (*idx + ADJ[code as usize]).clamp(0, 88);
    *pred as i16
}

/// One frame -> (seq, samples). `None` if it's shorter than the header.
pub fn decode_frame(frame: &[u8]) -> Option<(u16, Vec<i16>)> {
    if frame.len() < HEADER {
        return None;
    }
    let seq = u16::from_le_bytes([frame[0], frame[1]]);
    let mut pred = i16::from_le_bytes([frame[2], frame[3]]) as i32;
    let mut idx = (frame[4] as i32).clamp(0, 88);
    let mut out = Vec::with_capacity((frame.len() - HEADER) * 2);
    for &b in &frame[HEADER..] {
        out.push(step_once(b & 15, &mut pred, &mut idx));
        out.push(step_once(b >> 4, &mut pred, &mut idx));
    }
    Some((seq, out))
}

/// int16 samples -> frames in the stick's format (each decodes on its own). For tests and for
/// feeding recorded speech through the fake stick.
pub fn encode(pcm: &[i16], frame_samples: usize) -> Vec<Vec<u8>> {
    let (mut pred, mut idx) = (0i32, 0i32);
    let mut frames = Vec::new();
    for (seq, chunk) in pcm.chunks(frame_samples.max(2)).enumerate() {
        let mut out = Vec::with_capacity(HEADER + chunk.len().div_ceil(2));
        out.extend_from_slice(&(seq as u16).to_le_bytes());
        out.extend_from_slice(&(pred as i16).to_le_bytes());
        out.push(idx as u8);
        let mut byte = 0u8;
        for (i, &s) in chunk.iter().enumerate() {
            let mut step = STEP[idx as usize];
            let mut diff = s as i32 - pred;
            let mut code = 0u8;
            if diff < 0 {
                code = 8;
                diff = -diff;
            }
            if diff >= step {
                code |= 4;
                diff -= step;
            }
            step >>= 1;
            if diff >= step {
                code |= 2;
                diff -= step;
            }
            step >>= 1;
            if diff >= step {
                code |= 1;
            }
            step_once(code, &mut pred, &mut idx); // the decoder's view, so encoder and decoder agree
            if i & 1 == 1 {
                out.push(byte | (code << 4));
            } else {
                byte = code;
            }
        }
        if chunk.len() & 1 == 1 {
            out.push(byte);
        }
        frames.push(out);
    }
    frames
}

/// One talk's frames, turned into `f32` audio (adpcm.py `Recording`).
#[derive(Debug, Default, Clone)]
pub struct Recording {
    frames: BTreeMap<u16, Vec<i16>>,
}

impl Recording {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes a frame; header-only (or shorter) frames are ignored.
    pub fn add(&mut self, frame: &[u8]) {
        if frame.len() > HEADER
            && let Some((seq, samples)) = decode_frame(frame)
        {
            self.frames.insert(seq, samples);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// In seq order, a lost frame as silence the size of the largest frame; samples in [-1, 1).
    pub fn audio(&self) -> Vec<f32> {
        let (Some(&lo), Some(&hi)) = (self.frames.keys().next(), self.frames.keys().next_back()) else {
            return Vec::new();
        };
        let size = self.frames.values().map(Vec::len).max().unwrap_or(0);
        let mut out = Vec::with_capacity((hi - lo + 1) as usize * size);
        for seq in lo..=hi {
            match self.frames.get(&seq) {
                Some(s) => out.extend(s.iter().map(|&v| v as f32 / 32768.0)),
                None => out.extend(std::iter::repeat_n(0.0, size)),
            }
        }
        out
    }

    /// Frames missing between the first and the last.
    pub fn lost(&self) -> usize {
        match (self.frames.keys().next(), self.frames.keys().next_back()) {
            (Some(&lo), Some(&hi)) => (hi - lo) as usize + 1 - self.frames.len(),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f64, hz: f64, amp: f64) -> Vec<i16> {
        let n = (16000.0 * seconds) as usize;
        (0..n).map(|i| (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / 16000.0).sin()) as i16).collect()
    }

    fn snr_db(r: &[i16], o: &[i16]) -> f64 {
        let sig: f64 = r.iter().map(|&x| (x as f64).powi(2)).sum();
        let noise: f64 = r.iter().zip(o).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum();
        10.0 * (sig / noise.max(1e-9)).log10()
    }

    fn decode_all(frames: &[Vec<u8>]) -> Vec<i16> {
        frames.iter().flat_map(|f| decode_frame(f).unwrap().1).collect()
    }

    // --- test_adpcm.py ---------------------------------------------------------------------------

    #[test]
    fn frame_format() {
        let frames = encode(&tone(0.05, 440.0, 12000.0), 320);
        assert_eq!(frames.len(), 3); // 800 samples -> 320 + 320 + 160
        let seq = u16::from_le_bytes([frames[1][0], frames[1][1]]);
        assert_eq!(seq, 1);
        assert!(frames[1][4] <= 88);
        assert_eq!(frames[0].len(), 5 + 320 / 2);
        assert_eq!(frames[2].len(), 5 + 160 / 2);
    }

    #[test]
    fn round_trip_quality() {
        let pcm = tone(1.0, 440.0, 12000.0);
        let out = decode_all(&encode(&pcm, 320));
        assert_eq!(out.len(), pcm.len());
        assert!(snr_db(&pcm, &out) > 20.0);
    }

    #[test]
    fn odd_sample_count() {
        let pcm: Vec<i16> = (0..101).map(|i| (i * 37) as i16).collect();
        let frames = encode(&pcm, 320);
        assert_eq!(frames.iter().map(|f| decode_frame(f).unwrap().1.len()).sum::<usize>(), 102);
    }

    #[test]
    fn each_frame_decodes_on_its_own() {
        let frames = encode(&tone(0.2, 440.0, 12000.0), 320);
        let whole = decode_all(&frames);
        let alone = decode_frame(&frames[3]).unwrap().1;
        assert_eq!(alone, whole[3 * 320..4 * 320]);
    }

    #[test]
    fn recording_orders_frames_and_fills_lost_ones_with_silence() {
        let frames = encode(&tone(0.2, 440.0, 12000.0), 320); // 10 frames
        let mut rec = Recording::new();
        for f in frames.iter().rev() {
            if u16::from_le_bytes([f[0], f[1]]) != 4 {
                rec.add(f);
            }
        }
        let audio = rec.audio();
        assert_eq!(rec.lost(), 1);
        assert_eq!(audio.len(), 10 * 320);
        assert!(audio[4 * 320..5 * 320].iter().all(|&x| x == 0.0));
        assert!(audio[5 * 320..].iter().fold(0.0f32, |m, x| m.max(x.abs())) > 0.1);
        assert!(audio.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn recording_ignores_runt_frames_and_empty_is_empty() {
        let mut rec = Recording::new();
        rec.add(&[0, 0, 0, 0, 0]);
        assert!(rec.audio().is_empty());
        assert_eq!(rec.lost(), 0);
        assert_eq!(decode_frame(&[1, 2]), None);
    }

    #[test]
    fn matches_the_python_codec() {
        // adpcm.decode_frame(bytes([3,0, 0x10,0x00, 5, 0x7f, 0x88, 0x01])) in the Python agent
        let (seq, s) = decode_frame(&[3, 0, 0x10, 0x00, 5, 0x7f, 0x88, 0x01]).unwrap();
        assert_eq!(seq, 3);
        assert_eq!(s, vec![-6, 40, 34, 28, 44, 49]);
        // adpcm.encode([int(8000 * math.sin(i / 3)) for i in range(700)], 320)
        let pcm: Vec<i16> = (0..700).map(|i| (8000.0 * (i as f64 / 3.0).sin()) as i16).collect();
        let f = encode(&pcm, 320);
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(f.len(), 3);
        assert_eq!(hex(&f[1][..12]), "010031f23c342411a8ebbbbb");
        assert_eq!(hex(&f[2][f[2].len() - 4..]), "89314543");
    }
}
