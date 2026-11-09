//! The real backends (macOS): [`SystemCoreAudio`] (the HAL + process taps, every object recorded
//! in the process-wide registry of [`crate::mic`], so `destroy_all_live()` still works from a
//! signal handler) and [`CpalOutputs`] (output streams through cpal).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::ca::{CoreAudio, DeviceInfo, Id, Measure, Status, TapInfo};
use crate::level::{ENOUGH_MS, Meter, PROBE_FREQ, Sine};
use crate::mic::{Live, claim, record};
use crate::player::{OutDevice, OutStream, Outputs, Render};
use crate::plist::aggregate_description;
use crate::{hal, tap};

static HAL_THREAD: Once = Once::new();

/// Core Audio through the HAL (`CoreAudioDevices` in stick_mic_device.py).
pub struct SystemCoreAudio;

impl SystemCoreAudio {
    pub fn new() -> Result<Self, String> {
        HAL_THREAD.call_once(|| {
            let st = hal::use_hal_thread_for_notifications();
            if st != 0 {
                log::info!("kAudioHardwarePropertyRunLoop = NULL failed: {}", hal::status_text(st));
            }
        });
        Ok(Self)
    }
}

impl CoreAudio for SystemCoreAudio {
    fn devices(&mut self) -> Result<Vec<DeviceInfo>, String> {
        Ok(hal::devices())
    }

    fn create_aggregate(&mut self, name: &str, uid: &str, sub_uid: &str, tap_uuid: Option<&str>) -> Result<Id, Status> {
        let dev = hal::create_aggregate(&aggregate_description(name, uid, sub_uid, tap_uuid))?;
        record(Live::Device(dev));
        Ok(dev)
    }

    fn destroy_aggregate(&mut self, dev: Id) -> Status {
        claim(Live::Device(dev));
        hal::destroy_aggregate(dev)
    }

    fn process_object(&mut self, pid: i32) -> Option<Id> {
        hal::process_object(pid)
    }

    fn tap_output_uid(&mut self) -> Option<String> {
        hal::tap_output_uid()
    }

    fn create_tap(&mut self, name: &str, process: Id) -> Result<(Id, String), String> {
        let t = tap::create_process_tap(name, process)?;
        record(Live::Tap(t.id));
        Ok((t.id, t.uuid))
    }

    fn destroy_tap(&mut self, t: Id) -> Status {
        claim(Live::Tap(t));
        tap::destroy_process_tap(t)
    }

    fn taps(&mut self) -> Result<Vec<TapInfo>, String> {
        tap::available()?;
        Ok(hal::taps())
    }

    fn input_users(&mut self, dev: Id) -> Option<Vec<i32>> {
        hal::input_users(dev)
    }

    fn measure_input(
        &mut self,
        _dev: Id,
        dev_uid: &str,
        out_uid: &str,
        secs: f64,
        amp: f32,
    ) -> Result<Measure, String> {
        let mut sine: Option<Sine> = None;
        let (meter, ms) = measure(dev_uid, out_uid, amp, Duration::from_secs_f64(secs), move |out, ch, rate| {
            sine.get_or_insert_with(|| Sine::new(PROBE_FREQ, amp, rate)).fill(out, ch);
        })?;
        Ok(Measure { callbacks: meter.0.callbacks, rms: meter.0.rms(), first_ms: meter.0.first_ms(meter.1), ms })
    }
}

/// The input of the device `dev_uid` (cpal, by UID: never a fallback device — cpal makes its own
/// *unmuted* loopback tap when asked for the input of an output-only device), looked up for up to 2 s.
pub fn find_input(host: &cpal::Host, uid: &str) -> Result<cpal::Device, String> {
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

/// The input config at the device's *nominal* rate. cpal's `default_input_config()` can be wrong
/// for a tapped aggregate (48 kHz while the aggregate, clocked by 44.1 kHz Bluetooth headphones,
/// runs at 44.1 kHz: opening it then fails with "Sample rate 48000 Hz is not supported").
pub fn input_config(d: &cpal::Device, uid: &str) -> Result<cpal::SupportedStreamConfig, String> {
    let nominal = hal::devices().into_iter().find(|x| x.uid == uid).and_then(|x| hal::nominal_rate(x.id));
    if let Some(rate) = nominal.map(|r| r.round() as u32)
        && let Ok(ranges) = d.supported_input_configs()
    {
        for r in ranges {
            if r.sample_format() == cpal::SampleFormat::F32
                && r.min_sample_rate() <= rate
                && rate <= r.max_sample_rate()
            {
                return Ok(r.with_sample_rate(rate));
            }
        }
    }
    d.default_input_config().map_err(|e| format!("device input config: {e}"))
}

/// The output device with this UID (cpal).
pub fn find_output(host: &cpal::Host, uid: &str) -> Option<cpal::Device> {
    host.output_devices().ok()?.find(|d| d.id().map(|i| i.id() == uid).unwrap_or(false))
}

/// Plays `source(buf, channels, rate)` into the tap's output (`out_uid`, muted by the tap) while
/// metering the device's input, until ENOUGH_MS of signal arrived or `max`. -> (meter, its rate), ms.
fn measure<F>(
    dev_uid: &str,
    out_uid: &str,
    amp: f32,
    max: Duration,
    mut source: F,
) -> Result<((Meter, u32), f64), String>
where
    F: FnMut(&mut [f32], usize, u32) + Send + 'static,
{
    let host = cpal::default_host();
    let input = find_input(&host, dev_uid)?;
    let icfg = input_config(&input, dev_uid)?;
    if icfg.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("device input is {:?}, expected f32", icfg.sample_format()));
    }
    let in_rate = icfg.sample_rate();
    let meter = Arc::new(Mutex::new(Meter::new(amp, icfg.channels() as usize)));
    let mm = meter.clone();
    let istream = input
        .build_input_stream::<f32, _, _>(
            icfg.config(),
            move |data, _| {
                if let Ok(mut m) = mm.lock() {
                    m.push(data);
                }
            },
            |e| log::warn!("check input: {e}"),
            Some(Duration::from_secs(3)),
        )
        .map_err(|e| format!("can't open the check device's input: {e}"))?;
    let output = find_output(&host, out_uid).ok_or_else(|| format!("the output {out_uid} is gone: not playing"))?;
    let ocfg = output.default_output_config().map_err(|e| format!("output config: {e}"))?;
    let (out_rate, out_ch) = (ocfg.sample_rate(), ocfg.channels() as usize);
    // the gate: audio leaves only while it's open (closed before the device / tap are destroyed)
    let gate = Arc::new(AtomicBool::new(true));
    let g = gate.clone();
    let ostream = output
        .build_output_stream::<f32, _, _>(
            ocfg.config(),
            move |out, _| {
                if g.load(Ordering::SeqCst) {
                    source(out, out_ch, out_rate);
                } else {
                    out.fill(0.0);
                }
            },
            |e| log::warn!("check output: {e}"),
            Some(Duration::from_secs(3)),
        )
        .map_err(|e| format!("can't open the default output: {e}"))?;
    let t0 = Instant::now();
    istream.play().map_err(|e| format!("can't start the check input: {e}"))?;
    ostream.play().map_err(|e| format!("can't start the output: {e}"))?;
    while t0.elapsed() < max {
        std::thread::sleep(Duration::from_millis(10));
        if meter.lock().map(|m| m.enough(in_rate, ENOUGH_MS)).unwrap_or(false) {
            break;
        }
    }
    gate.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(30)); // a callback in flight finishes gated
    drop(ostream);
    drop(istream);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let m = meter.lock().map_err(|_| "meter poisoned")?.clone();
    Ok(((m, in_rate), ms))
}

// --- outputs ---

/// Output streams through cpal (the tap's output for the tap, BlackHole 2ch otherwise).
#[derive(Default)]
pub struct CpalOutputs;

fn describe(d: &cpal::Device) -> Option<OutDevice> {
    let uid = d.id().ok()?.id().to_string();
    let name = d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| uid.clone());
    let cfg = d.default_output_config().ok()?;
    Some(OutDevice { uid, name, rate: cfg.sample_rate(), channels: cfg.channels() as usize })
}

struct CpalStream {
    _stream: cpal::Stream,
    alive: Arc<AtomicBool>,
}

impl OutStream for CpalStream {
    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

impl Outputs for CpalOutputs {
    fn tap_output(&self) -> Option<OutDevice> {
        describe(&find_output(&cpal::default_host(), &hal::tap_output_uid()?)?)
    }

    fn find(&self, name: &str) -> Option<OutDevice> {
        let n = name.to_lowercase();
        cpal::default_host()
            .output_devices()
            .ok()?
            .filter_map(|d| describe(&d))
            .find(|d| d.name.to_lowercase().contains(&n) && d.channels > 0)
    }

    fn open(&self, dev: &OutDevice, mut render: Render) -> Result<Box<dyn OutStream>, String> {
        let host = cpal::default_host();
        let device = find_output(&host, &dev.uid).ok_or_else(|| format!("{} is gone", dev.name))?;
        let cfg = device.default_output_config().map_err(|e| e.to_string())?;
        if cfg.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("its output is {:?}, expected f32", cfg.sample_format()));
        }
        let ch = cfg.channels() as usize;
        let alive = Arc::new(AtomicBool::new(true));
        let a = alive.clone();
        let stream = device
            .build_output_stream::<f32, _, _>(
                cfg.config(),
                move |out, _| render(out, ch),
                move |e| {
                    log::warn!("wireless mic output: {e}");
                    a.store(false, Ordering::SeqCst);
                },
                Some(Duration::from_secs(3)),
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Box::new(CpalStream { _stream: stream, alive }))
    }
}
