//! The probe's signal and its measurement: plain Rust, no audio APIs (unit-tested).
//! Mirrors `measure_input()` / `probe()` in stick_mic_device.py.

/// The tap check: at most this long for the tone to come through.
pub const PROBE_S: f64 = 1.5;
/// -80 dBFS: inaudible even if the tap doesn't mute it.
pub const PROBE_AMP: f32 = 1e-4;
pub const PROBE_FREQ: f32 = 440.0;
/// Stop measuring once this much of the tone has arrived.
pub const ENOUGH_MS: f64 = 60.0;

pub fn dbfs(x: f64) -> f64 {
    if x <= 0.0 { f64::NEG_INFINITY } else { 20.0 * x.log10() }
}

/// A sine for interleaved output buffers.
#[derive(Debug, Clone)]
pub struct Sine {
    amp: f32,
    step: f64,
    phase: f64,
}

impl Sine {
    pub fn new(freq: f32, amp: f32, rate: u32) -> Self {
        Self { amp, step: std::f64::consts::TAU * freq as f64 / rate as f64, phase: 0.0 }
    }

    /// Fills `out` (interleaved, `channels` per frame) with the same sample on every channel.
    pub fn fill(&mut self, out: &mut [f32], channels: usize) {
        let ch = channels.max(1);
        for frame in out.chunks_mut(ch) {
            let s = self.amp * self.phase.sin() as f32;
            frame.fill(s);
            self.phase = (self.phase + self.step) % std::f64::consts::TAU;
        }
    }
}

/// What an input stream heard: silence until the first buffer whose peak is over the threshold
/// (amp * 0.2), then the RMS of everything from that buffer on.
#[derive(Debug, Clone)]
pub struct Meter {
    pub threshold: f32,
    pub channels: usize,
    pub callbacks: u64,
    /// Frames seen before the first loud-enough buffer (for "first_ms").
    pub frames_before_first: Option<u64>,
    frames_seen: u64,
    sq: f64,
    n: u64,
    /// Frames counted after the first signal.
    pub heard_frames: u64,
    /// The loudest sample seen at all (also before "first"), for diagnostics.
    pub peak: f32,
}

impl Meter {
    pub fn new(amp: f32, channels: usize) -> Self {
        Self {
            threshold: amp * 0.2,
            channels: channels.max(1),
            callbacks: 0,
            frames_before_first: None,
            frames_seen: 0,
            sq: 0.0,
            n: 0,
            heard_frames: 0,
            peak: 0.0,
        }
    }

    pub fn push(&mut self, data: &[f32]) {
        self.callbacks += 1;
        let frames = (data.len() / self.channels) as u64;
        let peak = data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        self.peak = self.peak.max(peak);
        if self.frames_before_first.is_none() && peak > self.threshold {
            self.frames_before_first = Some(self.frames_seen);
        }
        if self.frames_before_first.is_some() {
            self.sq += data.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>();
            self.n += data.len() as u64;
            self.heard_frames += frames;
        }
        self.frames_seen += frames;
    }

    /// RMS after the first signal (0 = silence).
    pub fn rms(&self) -> f64 {
        if self.n == 0 { 0.0 } else { (self.sq / self.n as f64).sqrt() }
    }

    pub fn enough(&self, rate: u32, enough_ms: f64) -> bool {
        self.heard_frames as f64 >= rate as f64 * enough_ms / 1000.0
    }

    pub fn first_ms(&self, rate: u32) -> Option<f64> {
        self.frames_before_first.map(|f| f as f64 * 1000.0 / rate as f64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The tone came through at a plausible level: the tap works (and so mutes).
    Ok,
    /// The input ran but heard (almost) nothing: no System Audio Recording permission yet.
    Silent,
    /// The input never delivered a buffer.
    DidntRun,
}

/// Same rule as Python: the tone arrived and its RMS is at least a quarter of a sine's RMS.
pub fn verdict(m: &Meter, amp: f32) -> Verdict {
    let expected = amp as f64 / std::f64::consts::SQRT_2;
    if m.frames_before_first.is_some() && m.rms() >= expected * 0.25 {
        Verdict::Ok
    } else if m.callbacks == 0 {
        Verdict::DidntRun
    } else {
        Verdict::Silent
    }
}

/// Linear-interpolating resample of mono audio (good enough for a test phrase).
pub fn resample_linear(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let n = ((x.len() as u64 * to as u64) / from as u64) as usize;
    let ratio = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let p = i as f64 * ratio;
            let j = p as usize;
            let f = (p - j as f64) as f32;
            let a = x[j.min(x.len() - 1)];
            let b = x[(j + 1).min(x.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_minus_80_dbfs() {
        assert!((dbfs(PROBE_AMP as f64) - -80.0).abs() < 1e-3);
    }

    #[test]
    fn sine_fills_every_channel_with_the_amp() {
        let mut s = Sine::new(440.0, 0.5, 48000);
        let mut buf = vec![0.0f32; 2 * 4800];
        s.fill(&mut buf, 2);
        for f in buf.chunks(2) {
            assert_eq!(f[0], f[1]);
        }
        let peak = buf.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.49 && peak <= 0.5);
    }

    #[test]
    fn meter_hears_the_probe_tone() {
        let rate = 48000;
        let mut m = Meter::new(PROBE_AMP, 2);
        m.push(&vec![0.0; 2 * 512]); // silence first
        let mut s = Sine::new(PROBE_FREQ, PROBE_AMP, rate);
        for _ in 0..10 {
            let mut b = vec![0.0; 2 * 512];
            s.fill(&mut b, 2);
            m.push(&b);
        }
        assert_eq!(m.frames_before_first, Some(512));
        assert!(m.enough(rate, ENOUGH_MS));
        let rms = m.rms();
        assert!((rms - PROBE_AMP as f64 / 2f64.sqrt()).abs() < 1e-6, "rms {rms}");
        assert_eq!(verdict(&m, PROBE_AMP), Verdict::Ok);
    }

    #[test]
    fn meter_silence_and_noise_floor_fail() {
        let mut m = Meter::new(PROBE_AMP, 2);
        for _ in 0..10 {
            m.push(&vec![1e-6; 1024]); // way under amp*0.2
        }
        assert_eq!(verdict(&m, PROBE_AMP), Verdict::Silent);
        assert_eq!(verdict(&Meter::new(PROBE_AMP, 2), PROBE_AMP), Verdict::DidntRun);
    }

    #[test]
    fn resample_lengths() {
        let x: Vec<f32> = (0..22050).map(|i| i as f32).collect();
        assert_eq!(resample_linear(&x, 22050, 48000).len(), 48000);
        assert_eq!(resample_linear(&x, 22050, 22050).len(), 22050);
    }
}
