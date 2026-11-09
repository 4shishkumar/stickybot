//! The stick on USB (firmware_api.py `usb_proto` / `Stick.usb_sticks`, flasher.py's port helpers):
//! Espressif USB serial ports, `@…` protocol lines over them (the fallback when Bluetooth isn't
//! there, `via: "usb"`), and the 1200-baud "touch" that reboots the stick into its bootloader.
//!
//! Behind [`Usb`] so tests use [`FakeUsb`]; the real one ([`SystemUsb`]) uses the `serialport`
//! crate. Listing ports never opens them; a port is opened only for a protocol line or the touch.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ESPRESSIF_VID: u16 = 0x303A;
/// sticks on USB are asked for their version at most this often (opening a port is slow)
pub const USB_CACHE: Duration = Duration::from_secs(10);

/// One Espressif USB serial port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    pub port: String,
    pub serial: String,
    pub product: String,
    pub pid: u16,
}

impl PortInfo {
    /// The ROM bootloader (manual download mode, or after the touch) shows up as the chip's own
    /// USB-JTAG/serial unit; our firmware's port is named after the stick.
    pub fn in_bootloader(&self) -> bool {
        self.product.to_lowercase().contains("jtag")
    }
}

pub trait Usb: Send + Sync {
    /// Espressif USB devices only (`/dev/cu.*`).
    fn ports(&self) -> Vec<PortInfo>;
    /// Sends one `@…` line and returns the `@{…}` reply, or `None`.
    fn proto(&self, port: &str, line: &str, timeout: Duration) -> Option<Value>;
    /// Opens + closes the port at 1200 baud: the firmware reboots into the bootloader.
    fn touch_1200(&self, port: &str) -> Result<(), String>;
}

/// The Mac's serial ports.
pub struct SystemUsb;

impl Usb for SystemUsb {
    fn ports(&self) -> Vec<PortInfo> {
        let Ok(ports) = serialport::available_ports() else { return Vec::new() };
        ports
            .into_iter()
            .filter(|p| p.port_name.starts_with("/dev/cu.") || !cfg!(target_os = "macos"))
            .filter_map(|p| match p.port_type {
                serialport::SerialPortType::UsbPort(u) if u.vid == ESPRESSIF_VID => Some(PortInfo {
                    port: p.port_name,
                    serial: u.serial_number.unwrap_or_default(),
                    product: u.product.unwrap_or_default(),
                    pid: u.pid,
                }),
                _ => None,
            })
            .collect()
    }

    fn proto(&self, port: &str, line: &str, timeout: Duration) -> Option<Value> {
        let mut s = match serialport::new(port, 115_200).timeout(Duration::from_millis(200)).open() {
            Ok(s) => s,
            Err(e) => {
                log::info!("usb {port}: {e}");
                return None;
            }
        };
        let _ = s.clear(serialport::ClearBuffer::Input);
        if let Err(e) = s.write_all(format!("{}\n", line.trim()).as_bytes()) {
            log::info!("usb {port}: {e}");
            return None;
        }
        let deadline = Instant::now() + timeout;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        while Instant::now() < deadline {
            match s.read(&mut chunk) {
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    log::info!("usb {port}: {e}");
                    return None;
                }
            }
            if let Some(v) = find_reply(&buf) {
                return Some(v);
            }
        }
        None
    }

    fn touch_1200(&self, port: &str) -> Result<(), String> {
        let mut s = serialport::new(port, 1200)
            .timeout(Duration::from_millis(200))
            .open()
            .map_err(|e| format!("couldn't open {port} ({e}) — is something else using it?"))?;
        let _ = s.write_data_terminal_ready(false);
        std::thread::sleep(Duration::from_millis(100));
        drop(s);
        Ok(())
    }
}

/// The first complete `@{…}` line in what the stick wrote (it also logs other lines).
pub fn find_reply(buf: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(buf);
    let mut lines: Vec<&str> = text.split('\n').collect();
    lines.pop(); // the last piece may be cut off
    lines.into_iter().map(str::trim).filter(|l| l.starts_with("@{")).find_map(|l| serde_json::from_str(&l[1..]).ok())
}

/// The stick-on-USB part of the app: the port backend, "flashing" (no port opened meanwhile),
/// and the cached list of sticks with their versions.
pub struct UsbSticks {
    usb: Mutex<Arc<dyn Usb>>,
    pub flashing: std::sync::atomic::AtomicBool,
    cache: Mutex<Option<(Instant, Vec<Value>)>>,
}

impl Default for UsbSticks {
    fn default() -> Self {
        Self::new(Arc::new(SystemUsb))
    }
}

impl UsbSticks {
    pub fn new(usb: Arc<dyn Usb>) -> Self {
        Self { usb: Mutex::new(usb), flashing: Default::default(), cache: Mutex::new(None) }
    }

    pub fn backend(&self) -> Arc<dyn Usb> {
        self.usb.lock().unwrap().clone()
    }

    pub fn set_backend(&self, usb: Arc<dyn Usb>) {
        *self.usb.lock().unwrap() = usb;
        self.forget();
    }

    pub fn is_flashing(&self) -> bool {
        self.flashing.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The next list asks the sticks again (after a flash).
    pub fn forget(&self) {
        *self.cache.lock().unwrap() = None;
    }

    /// `POST /api/stick/proto` over USB: `{"ok","via","reply","error"}` (firmware_api.py `run`'s USB half).
    pub async fn run(&self, line: &str) -> Value {
        if self.is_flashing() {
            return json!({"ok": false, "via": null, "reply": null, "error": "the stick is being flashed"});
        }
        let usb = self.backend();
        let line = line.to_string();
        tokio::task::spawn_blocking(move || {
            let ports: Vec<PortInfo> = usb.ports().into_iter().filter(|p| !p.in_bootloader()).collect();
            match ports.len() {
                0 => json!({"ok": false, "via": null, "reply": null, "error": "no stick connected (Bluetooth or USB)"}),
                1 => match usb.proto(&ports[0].port, &line, Duration::from_millis(2500)) {
                    Some(reply) => json!({"ok": reply.get("ok").and_then(Value::as_bool).unwrap_or(true), "via": "usb",
                                          "error": reply.get("error").cloned().unwrap_or(Value::Null), "reply": reply}),
                    None => json!({"ok": false, "via": "usb", "reply": null, "error": "the stick on USB didn't answer"}),
                },
                _ => json!({"ok": false, "via": null, "reply": null, "error": "several sticks on USB — connect one"}),
            }
        })
        .await
        .unwrap_or_else(|e| json!({"ok": false, "via": null, "reply": null, "error": e.to_string()}))
    }

    /// Sticks on USB with their version (`GET /api/firmware` `usb`), cached for 10 s.
    pub async fn sticks(&self) -> Vec<Value> {
        if let Some((at, list)) = self.cache.lock().unwrap().as_ref()
            && (at.elapsed() < USB_CACHE || self.is_flashing())
        {
            return list.clone();
        }
        if self.is_flashing() {
            return Vec::new();
        }
        let usb = self.backend();
        let out = tokio::task::spawn_blocking(move || {
            usb.ports()
                .into_iter()
                .map(|p| {
                    let mut e = json!({"port": p.port, "serial": p.serial, "version": null, "board": null});
                    if p.in_bootloader() {
                        e["board"] = json!("download mode");
                    } else if let Some(st) = usb.proto(&p.port, "@state", Duration::from_millis(1500)) {
                        e["version"] = st.get("fw").cloned().unwrap_or(Value::Null);
                        e["board"] = st.get("board").cloned().unwrap_or(Value::Null);
                    }
                    e
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        *self.cache.lock().unwrap() = Some((Instant::now(), out.clone()));
        out
    }
}

// --- a stick on USB in memory ---------------------------------------------------------------------------

/// Sticks on USB for tests: ports, answers to `@…` lines, the touch moving a port into the bootloader.
#[derive(Default)]
pub struct FakeUsb {
    pub ports: Mutex<Vec<PortInfo>>,
    /// port -> the `@state` it answers (anything else: `{"ok": true, "line": …}`)
    pub states: Mutex<std::collections::HashMap<String, Value>>,
    pub lines: Mutex<Vec<(String, String)>>,
    pub touches: Mutex<Vec<String>>,
    /// after a touch the port comes back as the bootloader under this name (`None`: it doesn't)
    pub boot_port: Mutex<Option<String>>,
}

impl FakeUsb {
    pub fn stick(port: &str, fw: &str) -> Arc<Self> {
        let f = FakeUsb::default();
        f.ports.lock().unwrap().push(PortInfo {
            port: port.into(),
            serial: "SN1".into(),
            product: "StickyBot".into(),
            pid: 0x1001,
        });
        f.states.lock().unwrap().insert(port.into(), json!({"ok": true, "fw": fw, "board": "m5stick-s3", "name": "StickyBot"}));
        *f.boot_port.lock().unwrap() = Some(port.into());
        Arc::new(f)
    }
}

impl Usb for FakeUsb {
    fn ports(&self) -> Vec<PortInfo> {
        self.ports.lock().unwrap().clone()
    }
    fn proto(&self, port: &str, line: &str, _timeout: Duration) -> Option<Value> {
        self.lines.lock().unwrap().push((port.into(), line.into()));
        let st = self.states.lock().unwrap().get(port).cloned()?;
        Some(if line.trim() == "@state" { st } else { json!({"ok": true, "line": line}) })
    }
    fn touch_1200(&self, port: &str) -> Result<(), String> {
        self.touches.lock().unwrap().push(port.into());
        let boot = self.boot_port.lock().unwrap().clone();
        let mut ports = self.ports.lock().unwrap();
        ports.retain(|p| p.port != port);
        if let Some(b) = boot {
            ports.push(PortInfo { port: b, serial: "SN1".into(), product: "USB JTAG/serial debug unit".into(), pid: 0x1001 });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_found_among_log_lines() {
        assert_eq!(find_reply(b"boot...\r\n@{\"ok\":true,\"fw\":\"2.3.0\"}\r\n"), Some(json!({"ok": true, "fw": "2.3.0"})));
        assert_eq!(find_reply(b"@{\"ok\":tr"), None); // not complete yet
        assert_eq!(find_reply(b"@{nope}\n@{\"a\":1}\n"), Some(json!({"a": 1})));
    }

    #[tokio::test]
    async fn proto_over_usb_and_the_list() {
        let fake = FakeUsb::stick("/dev/cu.usbmodem1", "2.2.0");
        let u = UsbSticks::new(fake.clone());
        let r = u.run("@brightness pct=60").await;
        assert_eq!((r["ok"].as_bool(), r["via"].as_str()), (Some(true), Some("usb")));
        let list = u.sticks().await;
        assert_eq!(list, vec![json!({"port": "/dev/cu.usbmodem1", "serial": "SN1", "version": "2.2.0", "board": "m5stick-s3"})]);
        // cached: the stick isn't asked again
        let n = fake.lines.lock().unwrap().len();
        u.sticks().await;
        assert_eq!(fake.lines.lock().unwrap().len(), n);
        // in the bootloader: listed, never opened
        fake.touch_1200("/dev/cu.usbmodem1").unwrap();
        u.forget();
        assert_eq!(u.sticks().await[0]["board"], "download mode");
        assert_eq!(fake.lines.lock().unwrap().len(), n);
        assert_eq!(u.run("@state").await["error"], "no stick connected (Bluetooth or USB)");
        // while flashing, no port is opened
        u.flashing.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(u.run("@state").await["error"], "the stick is being flashed");
    }
}
