//! Bridges AppKit's push-to-talk events to the same voice engine as the stick.
//! Capture and hotkey registration live on the main thread. Transcription and the brain run on Tokio.

use serde_json::{Value, json};
use std::ffi::{CStr, CString, c_char};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use stickybot_server::appctl::{DEFAULT_VOICE_SHORTCUT, valid_voice_shortcut};
use stickybot_server::{AppState, talk::MacTurn};
use tauri::AppHandle;

unsafe extern "C" {
    fn sb_voice_configure(enabled: i32, keycode: u32, modifiers: u32, label: *const c_char) -> *const c_char;
    fn sb_voice_inputs() -> *const c_char;
    fn sb_voice_set_input(uid: *const c_char);
    fn sb_voice_set_sounds(on: i32);
    fn sb_voice_turn_ended();
    fn sb_voice_test(on: i32) -> *const c_char;
    fn sb_voice_test_level() -> f32;
    fn sb_voice_permission() -> *const c_char;
    fn sb_voice_config_error() -> *const c_char;
    fn sb_voice_panel(phase: *const c_char, heard: *const c_char, detail: *const c_char);
    fn sb_voice_shutdown();
}

struct Bridge {
    app: AppHandle,
    root: Arc<AppState>,
    recording: Mutex<Option<MacTurn>>,
    startup_error: Mutex<Option<String>>,
    // Native callbacks for completed turns can still be queued when the next chord goes down.
    generation: AtomicU64,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub fn start(app: &AppHandle, root: &Arc<AppState>) {
    if BRIDGE
        .set(Bridge {
            app: app.clone(),
            root: root.clone(),
            recording: Mutex::new(None),
            startup_error: Mutex::new(None),
            generation: AtomicU64::new(0),
        })
        .is_err()
    {
        return;
    }
    let (enabled, shortcut, input, sounds) = {
        let cfg = root.cfg.lock().unwrap();
        (
            cfg.get("mac_voice_enabled").and_then(Value::as_bool).unwrap_or(false),
            cfg.get("mac_voice_shortcut").and_then(Value::as_str).unwrap_or(DEFAULT_VOICE_SHORTCUT).to_owned(),
            cfg.get("mac_voice_input").and_then(Value::as_str).unwrap_or("").to_owned(),
            cfg.get("mac_voice_sounds").and_then(Value::as_bool).unwrap_or(true),
        )
    };
    let _ = options(app, &input, sounds);
    if enabled && let Err(error) = configure(app, true, &shortcut) {
        log::warn!("Couldn't register the Mac talk shortcut: {error}");
        *BRIDGE.get().unwrap().startup_error.lock().unwrap() = Some(error);
    }
}

// Carbon accepts hardware positions. These correspond to KeyboardEvent.code on the settings page.
fn carbon_chord(shortcut: &str) -> Result<(u32, u32), String> {
    if !valid_voice_shortcut(shortcut) {
        return Err("Choose a supported Mac shortcut.".into());
    }
    let mut parts = shortcut.rsplit('+');
    let key = parts.next().unwrap_or_default();
    const LETTERS: [u32; 26] =
        [0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16, 6];
    const DIGITS: [u32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    let code = match key.as_bytes() {
        b"Space" => 49,
        [b'K', b'e', b'y', c @ b'A'..=b'Z'] => LETTERS[(c - b'A') as usize],
        [b'D', b'i', b'g', b'i', b't', c @ b'0'..=b'9'] => DIGITS[(c - b'0') as usize],
        _ => return Err("Choose a supported key.".into()),
    };
    let mut modifiers = 0;
    for part in parts {
        modifiers |= match part {
            "Command" => 1 << 8,
            "Shift" => 1 << 9,
            "Option" => 1 << 11,
            "Control" => 1 << 12,
            _ => return Err("Choose a supported modifier.".into()),
        };
    }
    Ok((code, modifiers))
}

fn on_main<T: Send + 'static>(app: &AppHandle, operation: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    if objc2::MainThreadMarker::new().is_some() {
        return Ok(operation());
    }
    let (send, receive) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = send.send(operation());
    })
    .map_err(|e| e.to_string())?;
    receive.recv().map_err(|_| "StickyBot is closing.".into())
}

// Native getters return borrowed UTF-8, valid until the next native call on the main thread.
unsafe fn native_text(text: *const c_char) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    (!text.is_empty()).then(|| text.into_owned())
}

/// "Control+Shift+Space" -> "⌃⇧Space", as the panel shows it.
fn chord_label(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| match part {
            "Control" => "⌃",
            "Option" => "⌥",
            "Shift" => "⇧",
            "Command" => "⌘",
            key => key.strip_prefix("Key").or_else(|| key.strip_prefix("Digit")).unwrap_or(key),
        })
        .collect()
}

pub fn configure(app: &AppHandle, enabled: bool, shortcut: &str) -> Result<(), String> {
    let (code, mods) = carbon_chord(shortcut)?;
    let label = CString::new(chord_label(shortcut)).unwrap_or_default();
    if let Some(error) = on_main(app, move || unsafe {
        native_text(sb_voice_configure(i32::from(enabled), code, mods, label.as_ptr()))
    })? {
        return Err(error);
    }
    if let Some(bridge) = BRIDGE.get() {
        bridge.startup_error.lock().unwrap().take();
    }
    Ok(())
}

/// The microphone ("" = the system default) and the start/stop sounds. Applies from the next recording.
pub fn options(app: &AppHandle, input: &str, sounds: bool) -> Result<(), String> {
    let uid = CString::new(input).map_err(|_| "Choose a microphone from the list.")?;
    on_main(app, move || unsafe {
        sb_voice_set_input(uid.as_ptr());
        sb_voice_set_sounds(i32::from(sounds));
    })
}

/// Opens the chosen microphone for a few seconds so Settings can show its level.
pub fn test(app: &AppHandle, on: bool) -> Result<(), String> {
    match on_main(app, move || unsafe { native_text(sb_voice_test(i32::from(on))) })? {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub fn status(app: &AppHandle) -> Value {
    on_main(app, || unsafe {
        let inputs: Value = native_text(sb_voice_inputs()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let level = sb_voice_test_level();
        json!({"input_name": inputs["using"], "inputs": inputs, "permission": native_text(sb_voice_permission()),
               "test_level": (level >= 0.0).then_some(level),
               "error": BRIDGE.get().and_then(|b| b.startup_error.lock().unwrap().clone()).or_else(|| native_text(sb_voice_config_error()))})
    })
    .unwrap_or(Value::Null)
}

fn panel(phase: &str, heard: &str, detail: &str) {
    let string = |s: &str| CString::new(s.replace('\0', "�")).unwrap();
    let (phase, heard, detail) = (string(phase), string(heard), string(detail));
    unsafe {
        sb_voice_panel(phase.as_ptr(), heard.as_ptr(), detail.as_ptr());
    }
}

#[unsafe(no_mangle)]
extern "C" fn sb_voice_begin() -> i32 {
    let Some(bridge) = BRIDGE.get() else {
        return 0;
    };
    let Some(device) = bridge.root.devices.default_session() else {
        return 2;
    };
    let generation = bridge.generation.load(Ordering::Acquire).wrapping_add(1);
    let notify = Arc::new(move |phase: &str, heard: &str, detail: &str| {
        let (phase, heard, detail) = (phase.to_owned(), heard.to_owned(), detail.to_owned());
        let _ = bridge.app.run_on_main_thread(move || {
            if bridge.generation.load(Ordering::Acquire) == generation {
                panel(&phase, &heard, &detail);
            }
        });
    });
    let mut recording = bridge.recording.lock().unwrap();
    if recording.is_some() {
        return 0;
    }
    // Called on the AppKit thread: stopping a reply that is being read aloud tells the stick over Tokio.
    let _runtime = tauri::async_runtime::handle().inner().enter();
    let Ok(turn) = MacTurn::begin(device, notify) else {
        return 0;
    };
    bridge.generation.store(generation, Ordering::Release);
    *recording = Some(turn);
    1
}

/// The panel's cancel button: stops the keyboard's own request (transcribing, thinking, approval).
#[unsafe(no_mangle)]
extern "C" fn sb_voice_cancel_turn() {
    if let Some(bridge) = BRIDGE.get()
        && let Some(device) = bridge.root.devices.default_session()
    {
        let _runtime = tauri::async_runtime::handle().inner().enter();
        stickybot_server::talk::cancel_mac_turn(&device);
    }
}

/// Clears the native "request running" flag however the turn ends, a panic included.
struct TurnEnded(AppHandle);

impl Drop for TurnEnded {
    fn drop(&mut self) {
        let _ = self.0.run_on_main_thread(|| unsafe { sb_voice_turn_ended() });
    }
}

#[unsafe(no_mangle)]
extern "C" fn sb_voice_cancel() {
    if let Some(bridge) = BRIDGE.get() {
        bridge.recording.lock().unwrap().take();
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn sb_voice_audio(pcm: *const f32, samples: usize) {
    let Some(bridge) = BRIDGE.get() else {
        return;
    };
    let ended = TurnEnded(bridge.app.clone());
    let Some(turn) = bridge.recording.lock().unwrap().take() else {
        return;
    };
    if pcm.is_null() || samples > 60 * 16_000 || samples == 0 {
        panel("error", "", "No usable recording. Hold the shortcut and try again.");
        return;
    }
    // The native capture buffer is valid only for this call and holds mono 16 kHz samples.
    let audio = unsafe { std::slice::from_raw_parts(pcm, samples) }.to_vec();
    tauri::async_runtime::spawn(async move {
        let _ended = ended;
        turn.submit(audio).await;
    });
}

#[unsafe(no_mangle)]
extern "C" fn sb_voice_dashboard() {
    if let Some(bridge) = BRIDGE.get() {
        crate::show_dashboard_at(&bridge.app, "activity");
    }
}

pub fn shutdown() {
    if let Some(bridge) = BRIDGE.get() {
        bridge.generation.fetch_add(1, Ordering::Release);
    }
    unsafe {
        sb_voice_shutdown();
    }
    sb_voice_cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_physical_keys_use_carbon_positions() {
        assert_eq!(carbon_chord(DEFAULT_VOICE_SHORTCUT), Ok((49, 0x1200)));
        assert_eq!(carbon_chord("Command+KeyB"), Ok((11, 0x100)));
        assert_eq!(carbon_chord("Option+Digit6"), Ok((22, 0x800)));
        assert!(carbon_chord("Command+Digit/").is_err());
    }

    #[test]
    fn panel_labels_use_mac_symbols() {
        assert_eq!(chord_label(DEFAULT_VOICE_SHORTCUT), "⌃⇧Space");
        assert_eq!(chord_label("Command+Option+KeyB"), "⌘⌥B");
        assert_eq!(chord_label("Option+Digit6"), "⌥6");
    }
}
