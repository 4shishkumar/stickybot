//! From Bluetooth frames to a steady output stream (wispr_mic.py `Sequencer`, `JitterBuffer`,
//! `Resampler`): frames put in order by seq (a lost frame becomes 20 ms of silence), resampled to
//! the device's rate, and played from a small jitter buffer (~80 ms). Plain Rust, no audio APIs.

use std::collections::{BTreeMap, VecDeque};

/// The stick's mic.
pub const RATE: u32 = 16_000;
/// Jitter buffer: start playing once this much is queued.
pub const TARGET_MS: u32 = 80;
/// More than this queued (a burst, clock drift): drop the oldest back to TARGET_MS.
pub const MAX_MS: u32 = 200;
/// Frames that may arrive past a gap before the gap counts as lost.
pub const REORDER: usize = 2;

/// Frames by seq (u16, wraps) -> samples in order. A gap is filled with silence once `reorder`
/// later frames have arrived (or at the end); late frames and duplicates are dropped.
#[derive(Debug, Clone)]
pub struct Sequencer {
    reorder: usize,
    next: Option<u16>,
    waiting: BTreeMap<u16, Vec<i16>>,
    /// samples per frame (the last one seen), for the silence
    size: usize,
    pub lost: u64,
    pub late: u64,
    pub frames: u64,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new(REORDER)
    }
}

impl Sequencer {
    pub fn new(reorder: usize) -> Self {
        Self { reorder, next: None, waiting: BTreeMap::new(), size: 320, lost: 0, late: 0, frames: 0 }
    }

    /// -> the chunks now ready to play, in order.
    pub fn add(&mut self, seq: u16, samples: Vec<i16>) -> Vec<Vec<i16>> {
        let next = *self.next.get_or_insert(seq);
        let ahead = seq.wrapping_sub(next);
        if ahead >= 0x8000 || self.waiting.contains_key(&seq) {
            // behind (already played or given up) or a duplicate
            self.late += 1;
            return Vec::new();
        }
        self.frames += 1;
        if !samples.is_empty() {
            self.size = samples.len();
        }
        self.waiting.insert(seq, samples);
        let mut out = self.release();
        if self.waiting.len() > self.reorder {
            // a gap that won't fill any more: silence
            while !self.waiting.is_empty() && !self.waiting.contains_key(&self.next.unwrap_or(0)) {
                out.push(vec![0; self.size]);
                self.lost += 1;
                self.next = Some(self.next.unwrap_or(0).wrapping_add(1));
            }
            out.extend(self.release());
        }
        out
    }

    fn release(&mut self) -> Vec<Vec<i16>> {
        let mut out = Vec::new();
        while let Some(n) = self.next {
            match self.waiting.remove(&n) {
                Some(s) => {
                    out.push(s);
                    self.next = Some(n.wrapping_add(1));
                }
                None => break,
            }
        }
        out
    }

    /// The stream ended: everything still waiting, gaps as silence.
    pub fn flush(&mut self) -> Vec<Vec<i16>> {
        let mut out = Vec::new();
        while !self.waiting.is_empty() {
            let n = self.next.unwrap_or(0);
            match self.waiting.remove(&n) {
                Some(s) => out.push(s),
                None => {
                    out.push(vec![0; self.size]);
                    self.lost += 1;
                }
            }
            self.next = Some(n.wrapping_add(1));
        }
        out
    }
}

/// Streaming mono resampler (linear interpolation; pass-through when the rates match). Wispr's
/// recogniser doesn't need more, and it adds no delay or bursts.
#[derive(Debug, Clone)]
pub struct Resampler {
    pub rate_in: u32,
    pub rate_out: u32,
    step: f64,
    /// position of the next output sample, relative to `prev` (index 0)
    pos: f64,
    prev: Option<f32>,
}

impl Resampler {
    pub fn new(rate_in: u32, rate_out: u32) -> Self {
        let (ri, ro) = (rate_in.max(1), rate_out.max(1));
        Self { rate_in: ri, rate_out: ro, step: ri as f64 / ro as f64, pos: 0.0, prev: None }
    }

    pub fn process(&mut self, x: &[f32]) -> Vec<f32> {
        if self.rate_in == self.rate_out {
            return x.to_vec();
        }
        if x.is_empty() {
            return Vec::new();
        }
        // buf = [prev] + x (without copying x): sample i of buf
        let off = usize::from(self.prev.is_some());
        let len = x.len() + off;
        let at = |i: usize| if i < off { self.prev.unwrap_or(0.0) } else { x[i - off] };
        let mut out = Vec::with_capacity((x.len() as f64 / self.step) as usize + 2);
        while self.pos + 1.0 < len as f64 {
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            let (a, b) = (at(i), at(i + 1));
            out.push(a + (b - a) * f);
            self.pos += self.step;
        }
        self.pos -= (len - 1) as f64;
        self.prev = x.last().copied();
        out
    }
}

/// Audio at the device's rate between the Bluetooth side (push) and the audio thread (pull). Holds
/// back until `target` is queued, so the small gaps between Bluetooth deliveries don't become
/// clicks; when it runs dry it waits again. (The owner wraps it in a mutex.)
#[derive(Debug, Clone)]
pub struct JitterBuffer {
    target: usize,
    max: usize,
    q: VecDeque<f32>,
    pub playing: bool,
    /// the stream ended: play out what's left without waiting for the target
    pub draining: bool,
    pub underruns: u64,
    /// samples dropped to keep the latency down
    pub dropped: u64,
}

impl JitterBuffer {
    pub fn new(rate: u32) -> Self {
        Self::with(rate, TARGET_MS, MAX_MS)
    }

    pub fn with(rate: u32, target_ms: u32, max_ms: u32) -> Self {
        Self {
            target: (rate as u64 * target_ms as u64 / 1000) as usize,
            max: (rate as u64 * max_ms as u64 / 1000) as usize,
            q: VecDeque::new(),
            playing: false,
            draining: false,
            underruns: 0,
            dropped: 0,
        }
    }

    pub fn queued(&self) -> usize {
        self.q.len()
    }

    pub fn push(&mut self, x: &[f32]) {
        self.q.extend(x.iter().copied());
        if self.q.len() > self.max {
            let n = self.q.len() - self.target;
            self.q.drain(..n);
            self.dropped += n as u64;
        }
    }

    /// Fills `out` (silence where there's nothing to play).
    pub fn pull_into(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if !self.playing {
            if self.q.len() < self.target && !(self.draining && !self.q.is_empty()) {
                return;
            }
            self.playing = true;
        }
        let k = out.len().min(self.q.len());
        for (o, s) in out.iter_mut().zip(self.q.drain(..k)) {
            *o = s;
        }
        if k < out.len() {
            // ran dry
            self.playing = false;
            if self.draining {
                self.draining = false; // all played out
            } else {
                self.underruns += 1;
            }
        }
    }

    pub fn pull(&mut self, n: usize) -> Vec<f32> {
        let mut out = vec![0.0; n];
        self.pull_into(&mut out);
        out
    }

    pub fn drain(&mut self) {
        self.draining = true;
    }

    pub fn clear(&mut self) {
        self.q.clear();
        self.playing = false;
        self.draining = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(seq: u16, value: i16) -> (u16, Vec<i16>) {
        (seq, vec![value; 320])
    }

    fn firsts(out: &[Vec<i16>]) -> Vec<i16> {
        out.iter().map(|x| x[0]).collect()
    }

    // --- test_wispr_mic.py: Sequencer ---

    #[test]
    fn in_order_frames_come_straight_out() {
        let mut s = Sequencer::default();
        let (q, x) = chunk(0, 1);
        assert_eq!(s.add(q, x).iter().map(Vec::len).collect::<Vec<_>>(), vec![320]);
        let (q, x) = chunk(1, 1);
        assert_eq!(s.add(q, x).len(), 1);
        assert_eq!((s.lost, s.late), (0, 0));
    }

    #[test]
    fn a_frame_a_little_late_is_put_back_in_place() {
        let mut s = Sequencer::new(2);
        let mut out = s.add(0, vec![0; 320]);
        assert!(s.add(2, vec![2; 320]).is_empty()); // waiting for 1
        out.extend(s.add(1, vec![1; 320]));
        assert_eq!(firsts(&out), vec![0, 1, 2]);
        assert_eq!(s.lost, 0);
    }

    #[test]
    fn a_lost_frame_becomes_silence() {
        let mut s = Sequencer::new(2);
        let mut out = s.add(0, vec![5; 320]);
        out.extend(s.add(2, vec![5; 320]));
        out.extend(s.add(3, vec![5; 320])); // still within the reorder window
        assert_eq!(out.len(), 1);
        out.extend(s.add(4, vec![5; 320])); // 3 frames past the gap: 1 is lost
        assert_eq!(firsts(&out), vec![5, 0, 5, 5, 5]);
        assert_eq!((out[1].len(), s.lost), (320, 1));
        assert!(s.add(1, vec![1; 320]).is_empty()); // too late now: dropped
        assert_eq!(s.late, 1);
    }

    #[test]
    fn flush_fills_the_gaps_at_the_end() {
        let mut s = Sequencer::new(5);
        let mut out = s.add(0, vec![1; 320]);
        out.extend(s.add(3, vec![1; 320]));
        out.extend(s.flush());
        assert_eq!(firsts(&out), vec![1, 0, 0, 1]);
        assert_eq!(s.lost, 2);
    }

    #[test]
    fn seq_wraps_around() {
        let mut s = Sequencer::default();
        let mut out = s.add(65535, vec![1; 320]);
        out.extend(s.add(0, vec![1; 320]));
        out.extend(s.add(1, vec![1; 320]));
        assert_eq!((out.len(), s.lost, s.late), (3, 0, 0));
    }

    #[test]
    fn duplicates_are_dropped() {
        let mut s = Sequencer::new(3);
        s.add(0, vec![1; 320]);
        s.add(2, vec![1; 320]);
        assert!(s.add(2, vec![1; 320]).is_empty());
        assert_eq!(s.late, 1);
    }

    // --- jitter buffer ---

    #[test]
    fn jitter_buffer_waits_for_the_target_then_plays() {
        let mut jb = JitterBuffer::with(1000, 50, 200); // 1 kHz: 50 samples target
        jb.push(&[1.0; 30]);
        assert!(jb.pull(10).iter().all(|x| *x == 0.0)); // not enough yet: silence, nothing taken
        assert_eq!(jb.queued(), 30);
        jb.push(&[1.0; 30]);
        assert!(jb.pull(10).iter().all(|x| *x == 1.0));
        assert_eq!(jb.queued(), 50);
    }

    #[test]
    fn jitter_buffer_underrun_pads_with_silence_and_waits_again() {
        let mut jb = JitterBuffer::with(1000, 20, 200);
        jb.push(&[1.0; 25]);
        let out = jb.pull(40);
        assert!(out[..25].iter().all(|x| *x == 1.0) && out[25..].iter().all(|x| *x == 0.0));
        assert_eq!(jb.underruns, 1);
        assert!(!jb.playing);
        jb.push(&[1.0; 10]);
        assert!(jb.pull(5).iter().all(|x| *x == 0.0)); // below the target again: waits
    }

    #[test]
    fn jitter_buffer_caps_the_latency() {
        let mut jb = JitterBuffer::with(1000, 50, 100);
        let burst: Vec<f32> = (0..150).map(|i| i as f32).collect(); // catching up after a hiccup
        jb.push(&burst);
        assert_eq!((jb.queued(), jb.dropped), (50, 100));
        assert_eq!(jb.pull(1)[0], 100.0); // the oldest went
    }

    #[test]
    fn jitter_buffer_drains_the_tail_at_the_end() {
        let mut jb = JitterBuffer::with(1000, 50, 200);
        jb.push(&[1.0; 10]); // less than the target
        jb.drain();
        let out = jb.pull(20);
        assert!(out[..10].iter().all(|x| *x == 1.0) && out[10..].iter().all(|x| *x == 0.0));
        assert_eq!(jb.underruns, 0);
        assert!(!jb.draining);
    }

    // --- resampling ---

    #[test]
    fn resampler_passes_through_at_the_same_rate() {
        let x = vec![1.0f32; 100];
        assert_eq!(Resampler::new(16000, 16000).process(&x), x);
    }

    #[test]
    fn resampler_16k_to_48k_keeps_the_length_and_pitch() {
        let mut r = Resampler::new(16000, 48000);
        let x: Vec<f32> =
            (0..8000).map(|i| 0.25 * (std::f32::consts::TAU * 440.0 * i as f32 / 16000.0).sin()).collect();
        let out: Vec<f32> = x.chunks(320).flat_map(|c| r.process(c)).collect();
        assert!((out.len() as i64 - 3 * x.len() as i64).abs() < 50, "{}", out.len());
        // pitch: count rising zero crossings over 0.25 s
        let w = &out[2000..2000 + 12000];
        let ups = w.windows(2).filter(|p| p[0] < 0.0 && p[1] >= 0.0).count();
        assert!((ups as i64 - 110).abs() <= 2, "{ups} crossings (440 Hz over 0.25 s = 110)");
        // and continuous across chunk borders: no step bigger than a 440 Hz sine's at 48 kHz
        let max_step = out.windows(2).map(|p| (p[1] - p[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.25 * std::f32::consts::TAU * 440.0 / 48000.0 * 1.5, "{max_step}");
    }
}
