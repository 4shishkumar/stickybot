//! tapmic — the Rust "wireless mic" spike: creates the tap + aggregate ("spike-rust-mic"),
//! proves the tap works with an inaudible -80 dBFS probe, optionally plays speech / a tone into
//! it for Wispr Flow, and always destroys what it made.
//!
//!   tapmic                  the probe only (default; inaudible): create the check device, play
//!                           -80 dBFS into the default output, measure the device's input, destroy
//!   tapmic --speech         probe, then create "spike-rust-mic" and play a spoken phrase (from
//!                           `say`) into it after a countdown — for Wispr Flow (AUDIBLE if the tap
//!                           didn't mute, so only after the probe passed, and only while the tap exists)
//!   tapmic --tone           the same with a 440 Hz tone (-20 dBFS, 2 s)
//!   tapmic --list           list audio devices and process taps, change nothing
//!   tapmic --cleanup        remove spike devices/taps a crashed run left behind
//!   tapmic --show SECS      probe, then keep "spike-rust-mic" present for SECS, playing nothing (to
//!                           pick / rank it in Wispr Flow, or to test Ctrl-C cleanup)
//!
//! Options: --delay SECS (countdown before playing, default 8), --text "..." (what `say` says),
//! --timeout SECS (watchdog for the whole run, default 20 + the loud part).
//!
//! Safety: the loud audio only goes out after the probe heard the -80 dBFS tone through *this*
//! device's tap (no permission = silent tap that doesn't mute = the probe fails and nothing else
//! plays); the output callback is gated on the tap existing; Ctrl-C, the watchdog, panics and
//! errors all destroy the devices and taps we made.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("tapmic: macOS only");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(app::run());
}

#[cfg(target_os = "macos")]
mod app {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use macos_audio::level::{self, ENOUGH_MS, Meter, PROBE_AMP, PROBE_FREQ, PROBE_S, Sine, Verdict};
    use macos_audio::mic::{self, TapMic};
    use macos_audio::naming::Names;
    use macos_audio::{hal, tap};

    const PERMISSION_HINT: &str = "allow System Audio Recording for the app that runs tapmic (Terminal / iTerm / \
        your IDE when run from a shell; the .app itself once bundled): System Settings → Privacy & Security → \
        Screen & System Audio Recording → System Audio Recording Only";
    const DEFAULT_TEXT: &str =
        "This is the StickyBot wireless mic test. If Wispr Flow typed this sentence, the Rust audio tap works.";

    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    /// Watchdog deadline, in ms since START.
    static DEADLINE_MS: AtomicU64 = AtomicU64::new(u64::MAX);
    /// Set by Ctrl-C / the watchdog: every output callback writes silence from then on.
    static SILENCE: AtomicBool = AtomicBool::new(false);

    /// Silence our output first (a callback in flight gets ~30 ms), then destroy the tap: never
    /// the other way round, or the last buffers would reach the speakers unmuted.
    fn silence_then_destroy_all() -> usize {
        SILENCE.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        mic::destroy_all_live()
    }

    fn since_start_ms() -> u64 {
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    fn set_timeout(secs: u64) {
        DEADLINE_MS.store(since_start_ms() + secs * 1000, Ordering::SeqCst);
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Probe,
        Speech,
        Tone,
        List,
        Cleanup,
        Show(u64),
    }

    struct Opts {
        mode: Mode,
        delay: u64,
        text: String,
        timeout: Option<u64>,
        as_stick: bool, // the Python app's own mic name + UID, so Wispr's ranking picks it
    }

    fn parse() -> Result<Opts, String> {
        let mut o = Opts { mode: Mode::Probe, delay: 8, text: DEFAULT_TEXT.into(), timeout: None, as_stick: false };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
            match a.as_str() {
                "--probe" => o.mode = Mode::Probe,
                "--speech" => o.mode = Mode::Speech,
                "--as-stick" => o.as_stick = true,
                "--tone" => o.mode = Mode::Tone,
                "--list" => o.mode = Mode::List,
                "--cleanup" => o.mode = Mode::Cleanup,
                "--show" => o.mode = Mode::Show(val("--show")?.parse().map_err(|e| format!("--show: {e}"))?),
                "--delay" => o.delay = val("--delay")?.parse().map_err(|e| format!("--delay: {e}"))?,
                "--text" => o.text = val("--text")?,
                "--timeout" => o.timeout = Some(val("--timeout")?.parse().map_err(|e| format!("--timeout: {e}"))?),
                "-h" | "--help" => return Err(String::new()),
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(o)
    }

    pub fn run() -> i32 {
        START.get_or_init(Instant::now);
        let opts = match parse() {
            Ok(o) => o,
            Err(e) => {
                if !e.is_empty() {
                    eprintln!("tapmic: {e}");
                }
                eprintln!("usage: tapmic [--probe | --speech | --tone | --show S | --list | --cleanup] [--delay S] [--text T] [--timeout S]");
                return 64;
            }
        };
        let (maj, min, patch) = tap::macos_version();
        println!("macOS {maj}.{min}.{patch}, pid {}, exe {}", std::process::id(), exe());
        if opts.mode == Mode::List {
            list();
            return 0;
        }
        if let Err(e) = tap::available() {
            println!("NOT AVAILABLE: {e}");
            return 3;
        }
        let st = hal::use_hal_thread_for_notifications();
        if st != 0 {
            println!("note: kAudioHardwarePropertyRunLoop = NULL failed: {}", hal::status_text(st));
        }
        install_guards(opts.timeout.unwrap_or(20));

        let names = if opts.as_stick {
            Names::new("stickybot-bt-mic", "com.stickkey.rust-wireless-mic")
        } else {
            Names::spike()
        };
        let before = ours(&names);
        let total_before = (hal::devices().len(), hal::taps().len());
        let (nd, nt) = mic::cleanup_stale(&names);
        if nd + nt > 0 || !before.is_empty() {
            println!("cleanup: removed {nd} left-over device(s) and {nt} tap(s) of an earlier run: {before:?}");
        }
        if opts.mode == Mode::Cleanup {
            return leak_check(&names, total_before);
        }

        // 1. The probe, through a test-named device (never the real one), like probe() in Python.
        let ok = match probe(&names.check()) {
            Ok(v) => v == Verdict::Ok,
            Err(e) => {
                println!("PROBE ERROR: {e}");
                false
            }
        };
        if !ok {
            println!("The tap isn't effective — nothing else will be played. To fix: {PERMISSION_HINT}");
            let code = leak_check(&names, total_before);
            return if code != 0 { code } else { 2 };
        }

        // 2. The audible part, only on request.
        let code = match opts.mode {
            Mode::Speech | Mode::Tone => match loud(&names, &opts) {
                Ok(()) => 0,
                Err(e) => {
                    println!("ERROR: {e}");
                    1
                }
            },
            Mode::Show(secs) => {
                if opts.timeout.is_none() {
                    set_timeout(30 + secs);
                }
                match TapMic::create(&names) {
                    Ok(mut m) => {
                        describe(&m);
                        println!("{:?} is present for {secs} s (nothing is played). Ctrl-C removes it.", names.name);
                        let t0 = Instant::now();
                        while t0.elapsed() < Duration::from_secs(secs) && m.live() {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        let d = m.destroy();
                        println!("  destroyed in {:.1} ms (status {:?}/{:?})", d.ms, d.device_status, d.tap_status);
                        0
                    }
                    Err(e) => {
                        println!("ERROR: {e}");
                        1
                    }
                }
            }
            _ => 0,
        };
        let leak = leak_check(&names, total_before);
        if leak != 0 { leak } else { code }
    }

    fn exe() -> String {
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    }

    /// Ctrl-C and a watchdog both destroy everything we made, then exit.
    fn install_guards(timeout_s: u64) {
        set_timeout(timeout_s);
        if let Err(e) = ctrlc::set_handler(|| {
            let n = silence_then_destroy_all();
            println!("\ninterrupted: destroyed {n} device/tap object(s)");
            std::process::exit(130);
        }) {
            println!("note: no Ctrl-C handler ({e})");
        }
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_millis(100));
                if since_start_ms() > DEADLINE_MS.load(Ordering::SeqCst) {
                    let n = silence_then_destroy_all();
                    println!(
                        "TIMEOUT: the run took too long (a macOS permission prompt waiting for an answer?). \
                         Destroyed {n} device/tap object(s). Not retrying."
                    );
                    std::process::exit(124);
                }
            }
        });
    }

    fn ours(names: &Names) -> Vec<String> {
        let uids = names.owned_device_uids();
        let taps = names.owned_tap_names();
        let mut v: Vec<String> =
            hal::devices().into_iter().filter(|d| uids.contains(&d.uid)).map(|d| format!("device {} {}", d.id, d.uid)).collect();
        v.extend(hal::taps().into_iter().filter(|t| taps.contains(&t.name)).map(|t| format!("tap {} {}", t.id, t.name)));
        v
    }

    /// Lists devices / taps after the run and compares with before. -> 0, or 4 on a leak.
    fn leak_check(names: &Names, total_before: (usize, usize)) -> i32 {
        // AudioHardwareDestroyAggregateDevice returns before the HAL's device list catches up: a
        // just-destroyed device can still be listed (with its UID) for a few ms. Give it up to 2 s.
        let t0 = Instant::now();
        let mut left = ours(names);
        while !left.is_empty() && t0.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
            left = ours(names);
        }
        if left.is_empty() && t0.elapsed() > Duration::from_millis(5) {
            println!("(the device list caught up {:.0} ms after the destroy calls returned)", t0.elapsed().as_secs_f64() * 1000.0);
        }
        let total_after = (hal::devices().len(), hal::taps().len());
        println!(
            "devices before/after: {}/{}, process taps before/after: {}/{}",
            total_before.0, total_after.0, total_before.1, total_after.1
        );
        if left.is_empty() {
            println!("clean: no spike device or tap left");
            0
        } else {
            println!("LEAK: still present: {left:?}");
            4
        }
    }

    fn list() {
        println!("devices:");
        for d in hal::devices() {
            println!("  {:>4}  {:<40}  {}", d.id, d.name, d.uid);
        }
        println!("default output: {}", hal::default_output_uid().unwrap_or_default());
        println!("tap output (clock + playback): {}", hal::tap_output_uid().unwrap_or_default());
        println!("process taps:");
        for t in hal::taps() {
            println!("  {:>4}  {:<40}  {}", t.id, t.name, t.uuid);
        }
    }

    fn describe(m: &TapMic) {
        println!(
            "  created {:?} (device {}, uid {}) in {:.1} ms: tap {} \"{}\" uuid {}, isMuted={} (1 = CATapMuted), \
             private={}, mono={}, mixdown={}, clock/sub-device {}",
            m.names.name,
            m.dev,
            m.names.uid,
            m.create_ms,
            m.tap.id,
            m.tap.name,
            m.tap.uuid,
            m.tap.mute_behavior,
            m.tap.private,
            m.tap.mono,
            m.tap.mixdown,
            m.out_uid
        );
    }

    /// Creates `check`, plays the -80 dBFS probe, measures, destroys. -> the verdict.
    fn probe(check: &Names) -> Result<Verdict, String> {
        println!("probe: -80 dBFS {PROBE_FREQ} Hz through {:?} (inaudible)", check.name);
        let mut m = TapMic::create(check)?;
        describe(&m);
        let r = measure_probe(&m);
        let d = m.destroy();
        println!(
            "  destroyed in {:.1} ms (device status {:?}, tap status {:?})",
            d.ms, d.device_status, d.tap_status
        );
        r
    }

    fn measure_probe(m: &TapMic) -> Result<Verdict, String> {
        let mut sine = Sine::new(PROBE_FREQ, PROBE_AMP, 48000); // re-made at the output's rate below
        let mut rate_set = false;
        let res = measure(m, PROBE_AMP, Duration::from_secs_f64(PROBE_S), true, move |out, ch, rate| {
            if !rate_set {
                sine = Sine::new(PROBE_FREQ, PROBE_AMP, rate);
                rate_set = true;
            }
            sine.fill(out, ch);
            true
        })?;
        let v = level::verdict(&res.meter, PROBE_AMP);
        let expected = PROBE_AMP as f64 / std::f64::consts::SQRT_2;
        println!(
            "  measured on the device input ({} ch @ {} Hz, {} callbacks, {:.0} ms): rms {:.3e} = {:.1} dBFS \
             (a -80 dBFS sine is {:.3e} = {:.1} dBFS rms), peak {:.1} dBFS, tone after {}",
            res.in_channels,
            res.in_rate,
            res.meter.callbacks,
            res.ms,
            res.meter.rms(),
            level::dbfs(res.meter.rms()),
            expected,
            level::dbfs(expected),
            level::dbfs(res.meter.peak as f64),
            res.meter.first_ms(res.in_rate).map(|x| format!("{x:.0} ms of input")).unwrap_or("— (never)".into()),
        );
        println!(
            "  verdict: {}",
            match v {
                Verdict::Ok => "OK — the tap works (so it mutes this process)",
                Verdict::Silent => "FAILED — the tap heard nothing",
                Verdict::DidntRun => "FAILED — the device's input never ran",
            }
        );
        Ok(v)
    }

    struct Measured {
        meter: Meter,
        in_rate: u32,
        in_channels: usize,
        ms: f64,
    }

    /// Runs an input stream on `m`'s aggregate and an output stream on the default output (which the
    /// tap mutes and captures) fed by `source(buf, channels, rate) -> still playing`, until the source
    /// is done (or `max`; or, with `stop_early`, once ENOUGH_MS of signal arrived).
    fn measure<F>(m: &TapMic, amp: f32, max: Duration, stop_early: bool, mut source: F) -> Result<Measured, String>
    where
        F: FnMut(&mut [f32], usize, u32) -> bool + Send + 'static,
    {
        let host = cpal::default_host();
        let input = find_input(&host, &m.names.uid)?;
        let icfg = macos_audio::system::input_config(&input, &m.names.uid)?;
        if icfg.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("device input is {:?}, expected f32", icfg.sample_format()));
        }
        let in_rate = icfg.sample_rate();
        let in_channels = icfg.channels() as usize;
        let meter = Arc::new(Mutex::new(Meter::new(amp, in_channels)));
        let mm = meter.clone();
        let istream = input
            .build_input_stream::<f32, _, _>(
                icfg.config(),
                move |data, _| {
                    if let Ok(mut m) = mm.lock() {
                        m.push(data);
                    }
                },
                |e| eprintln!("input stream error: {e}"),
                Some(Duration::from_secs(3)),
            )
            .map_err(|e| format!("can't open the device input: {e}"))?;

        // the tap's output (the default output, or the built-in one when the default is Bluetooth)
        let output = macos_audio::system::find_output(&host, &m.out_uid)
            .ok_or_else(|| format!("the output {} is gone: not playing", m.out_uid))?;
        let ocfg = output.default_output_config().map_err(|e| format!("output config: {e}"))?;
        if ocfg.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("output is {:?}, expected f32", ocfg.sample_format()));
        }
        let out_rate = ocfg.sample_rate();
        let out_ch = ocfg.channels() as usize;
        // The gate: audio leaves this process only while it's open, and it is only open while our
        // tap + device exist (closed before they are destroyed). Like `tap_live` in Python.
        let gate = Arc::new(AtomicBool::new(m.live()));
        let done = Arc::new(AtomicBool::new(false));
        let (g, dn) = (gate.clone(), done.clone());
        let ostream = output
            .build_output_stream::<f32, _, _>(
                ocfg.config(),
                move |out, _| {
                    if !g.load(Ordering::SeqCst) || SILENCE.load(Ordering::SeqCst) {
                        out.fill(0.0);
                        return;
                    }
                    if !source(out, out_ch, out_rate) {
                        dn.store(true, Ordering::SeqCst);
                    }
                },
                |e| eprintln!("output stream error: {e}"),
                Some(Duration::from_secs(3)),
            )
            .map_err(|e| format!("can't open the default output: {e}"))?;

        let t0 = Instant::now();
        istream.play().map_err(|e| format!("can't start the device input: {e}"))?;
        ostream.play().map_err(|e| format!("can't start the output: {e}"))?;
        while t0.elapsed() < max && !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
            if !m.live() {
                break; // destroyed under us (Ctrl-C / watchdog)
            }
            if stop_early && meter.lock().map(|x| x.enough(in_rate, ENOUGH_MS)).unwrap_or(false) {
                break;
            }
        }
        gate.store(false, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30)); // let a callback in flight finish gated
        drop(ostream);
        drop(istream);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let meter = meter.lock().map_err(|_| "meter poisoned")?.clone();
        Ok(Measured { meter, in_rate, in_channels, ms })
    }

    fn find_input(host: &cpal::Host, uid: &str) -> Result<cpal::Device, String> {
        let t0 = Instant::now();
        loop {
            if let Ok(devs) = host.input_devices() {
                for d in devs {
                    if d.id().map(|i| i.id() == uid).unwrap_or(false) {
                        return Ok(d);
                    }
                }
            }
            if t0.elapsed() > Duration::from_secs(2) {
                return Err(format!("cpal doesn't list an input device with uid {uid}"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Mono samples at `rate` for the loud test.
    fn loud_samples(opts: &Opts) -> Result<(Vec<f32>, u32), String> {
        if opts.mode == Mode::Tone {
            let rate = 48000;
            let mut s = Sine::new(440.0, 0.1, rate); // -20 dBFS
            let mut v = vec![0.0; rate as usize * 2];
            s.fill(&mut v, 1);
            return Ok((v, rate));
        }
        let dir = std::env::temp_dir().join(format!("tapmic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let wav = dir.join("speech.wav");
        let st = std::process::Command::new("/usr/bin/say")
            .arg("-o")
            .arg(&wav)
            .arg("--data-format=LEI16@22050")
            .arg(&opts.text)
            .status()
            .map_err(|e| format!("say: {e}"))?;
        if !st.success() {
            return Err(format!("say failed: {st}"));
        }
        let mut r = hound::WavReader::open(&wav).map_err(|e| format!("{}: {e}", wav.display()))?;
        let spec = r.spec();
        let ch = spec.channels.max(1) as usize;
        let ints: Vec<i16> = r.samples::<i16>().filter_map(Result::ok).collect();
        let mono: Vec<f32> = ints.chunks(ch).map(|f| f[0] as f32 / 32768.0).collect();
        let _ = std::fs::remove_dir_all(&dir);
        Ok((mono, spec.sample_rate))
    }

    fn loud(names: &Names, opts: &Opts) -> Result<(), String> {
        let (samples, src_rate) = loud_samples(opts)?;
        let secs = samples.len() as f64 / src_rate as f64;
        if opts.timeout.is_none() {
            set_timeout(30 + opts.delay + secs.ceil() as u64);
        }
        println!("\n{} ready: {:.1} s of audio", if opts.mode == Mode::Tone { "tone" } else { "speech" }, secs);
        let mut m = TapMic::create(names)?;
        describe(&m);
        // The same probe through *this* device's tap: nothing loud unless it hears the -80 dBFS tone.
        println!("re-probing through {:?} before anything audible:", names.name);
        if measure_probe(&m)? != Verdict::Ok {
            m.destroy();
            return Err(format!("the tap of {} isn't effective: not playing. {PERMISSION_HINT}", names.name));
        }
        println!(
            "\n>>> {:?} is present now. In Wispr Flow choose it as the microphone and start a dictation \
             (hands-free, or hold your shortcut). Playing in {} s…",
            names.name, opts.delay
        );
        for i in (1..=opts.delay).rev() {
            println!("  {i}");
            std::thread::sleep(Duration::from_secs(1));
            if !m.live() {
                return Err("the device went away".into());
            }
        }
        println!("playing…");
        let mut pos = 0.0f64;
        let tail = Duration::from_millis(800);
        let res = measure(&m, 0.01, Duration::from_secs_f64(secs + 5.0), false, move |out, ch, rate| {
            let step = src_rate as f64 / rate as f64;
            for frame in out.chunks_mut(ch.max(1)) {
                let i = pos as usize;
                let s = if i + 1 < samples.len() {
                    let f = (pos - i as f64) as f32;
                    samples[i] + (samples[i + 1] - samples[i]) * f
                } else {
                    0.0
                };
                frame.fill(s);
                pos += step;
            }
            (pos as usize) < samples.len()
        })?;
        std::thread::sleep(tail); // the device stays while Wispr finishes reading
        println!(
            "  the device heard it: rms {:.1} dBFS, peak {:.1} dBFS over {:.0} ms",
            level::dbfs(res.meter.rms()),
            level::dbfs(res.meter.peak as f64),
            res.ms
        );
        let d = m.destroy();
        println!("  destroyed {:?} in {:.1} ms (status {:?}/{:?})", names.name, d.ms, d.device_status, d.tap_status);
        Ok(())
    }
}
