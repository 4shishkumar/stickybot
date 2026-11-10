//! What carries the stick's audio into the "<stick>-bt-mic" device's input
//! (stick_mic_device.py `pick_carrier`):
//!
//! - **tap** (macOS 14.2+, no driver): our own process tap inside the aggregate; we play into the
//!   default output, the tap mutes it and hands it to the device. Only once the inaudible check
//!   passed (without the System Audio Recording permission a tap is silent *and doesn't mute*).
//! - **blackhole**: BlackHole 2ch as the aggregate's only sub-device; we play into its output.
//!
//! Config `wireless_mic_carrier`: `auto` (the tap once its check passed, else BlackHole if
//! installed) | `tap` | `blackhole`.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Carrier {
    Tap,
    #[serde(rename = "blackhole")]
    BlackHole,
}

impl Carrier {
    pub fn as_str(self) -> &'static str {
        match self {
            Carrier::Tap => "tap",
            Carrier::BlackHole => "blackhole",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Carrier::Tap => "Apple audio tap",
            Carrier::BlackHole => "BlackHole 2ch",
        }
    }
}

/// BlackHole's device name (what the aggregate is built on, and what we play into).
pub const BLACKHOLE_NAME: &str = "BlackHole 2ch";
/// The values of `wireless_mic_carrier`.
pub const PREFS: [&str; 3] = ["auto", "tap", "blackhole"];
pub const TAP_MIN_MACOS: (u32, u32) = (14, 2);

pub const PERMISSION_HINT: &str = "allow System Audio Recording for StickyBot (your terminal app when it runs from a terminal): \
     System Settings → Privacy & Security → Screen & System Audio Recording → System Audio Recording Only";
pub const NO_BLACKHOLE: &str = "BlackHole 2ch isn't installed — run: brew install --cask blackhole-2ch";

/// The probe's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    Ok,
    Failed,
    Error,
    /// running now (never stored in a result)
    Checking,
}

/// -> (carrier, why not). `pref`: config `wireless_mic_carrier` (unknown = auto); `tap_state`:
/// the check's result (`None` = not run yet); `blackhole`: BlackHole 2ch is installed.
pub fn pick_carrier(
    pref: &str,
    tap_supported: bool,
    tap_state: Option<CheckState>,
    blackhole: bool,
) -> (Option<Carrier>, String) {
    let pref = if PREFS.contains(&pref) { pref } else { "auto" };
    let bh = || if blackhole { (Some(Carrier::BlackHole), String::new()) } else { (None, NO_BLACKHOLE.to_string()) };
    if pref == "blackhole" {
        return bh();
    }
    if tap_supported && tap_state == Some(CheckState::Ok) {
        return (Some(Carrier::Tap), String::new());
    }
    if pref == "tap" || (tap_supported && !blackhole) {
        let why = if !tap_supported {
            format!("the Apple audio tap needs macOS {}.{} or newer", TAP_MIN_MACOS.0, TAP_MIN_MACOS.1)
        } else {
            match tap_state {
                None | Some(CheckState::Checking) => "checking the Apple audio tap…".to_string(),
                Some(CheckState::Failed) => {
                    format!("the Apple audio tap isn't allowed yet — {PERMISSION_HINT}, then press Check again")
                }
                _ => "the Apple audio tap doesn't work on this Mac — use BlackHole 2ch instead".to_string(),
            }
        };
        return (None, why);
    }
    bh()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Carrier::*;
    use CheckState::*;

    /// test_tap_mic.py `test_carrier_matrix`
    #[test]
    fn carrier_matrix() {
        type Case = (&'static str, bool, Option<CheckState>, bool, Option<Carrier>);
        let cases: &[Case] = &[
            ("auto", true, Some(Ok), true, Some(Tap)), // the tap wins once it's proven
            ("auto", true, Some(Ok), false, Some(Tap)),
            ("auto", true, Some(Failed), true, Some(BlackHole)), // not allowed: BlackHole meanwhile
            ("auto", true, None, true, Some(BlackHole)),         // not checked yet: BlackHole meanwhile
            ("auto", true, Some(Failed), false, None),
            ("auto", true, None, false, None),
            ("auto", false, None, true, Some(BlackHole)), // older macOS
            ("auto", false, None, false, None),
            ("tap", true, Some(Ok), true, Some(Tap)),
            ("tap", true, Some(Failed), true, None), // asked for the tap: no silent BlackHole
            ("tap", false, None, true, None),
            ("blackhole", true, Some(Ok), true, Some(BlackHole)),
            ("blackhole", true, Some(Ok), false, None),
            ("nonsense", true, Some(Ok), false, Some(Tap)), // unknown setting = auto
        ];
        for (pref, supported, state, bh, want) in cases {
            let (c, why) = pick_carrier(pref, *supported, *state, *bh);
            assert_eq!(c, *want, "{pref} {supported} {state:?} {bh}");
            assert_eq!(!why.is_empty(), want.is_none(), "{pref} {supported} {state:?} {bh}: {why}");
        }
    }

    /// test_tap_mic.py `test_why_not_says_what_to_do`
    #[test]
    fn why_not_says_what_to_do() {
        assert!(pick_carrier("auto", true, Some(Failed), false).1.contains("System Audio Recording"));
        assert!(pick_carrier("auto", true, None, false).1.contains("checking"));
        assert!(pick_carrier("tap", false, None, true).1.contains("14.2"));
        assert!(pick_carrier("blackhole", true, Some(Ok), false).1.contains("brew install --cask blackhole-2ch"));
        assert!(pick_carrier("auto", true, Some(Error), false).1.contains("BlackHole"));
    }

    #[test]
    fn serialized_like_python() {
        assert_eq!(serde_json::to_value(BlackHole).unwrap(), "blackhole");
        assert_eq!(serde_json::to_value(Tap).unwrap(), "tap");
        assert_eq!(serde_json::to_value(Failed).unwrap(), "failed");
    }
}
