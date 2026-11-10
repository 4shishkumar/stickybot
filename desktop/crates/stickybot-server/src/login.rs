//! Start at login. The menu bar app registers itself as a login item (macOS 13+
//! `SMAppService.mainAppService`, see `stickybot-app/src/login.rs`): no LaunchAgent plist and no
//! `launchctl` (the Python app's way), and macOS lists it under System Settings → General → Login
//! Items. The server only sees this trait, so the headless binary and the tests have none / a fake.

use serde_json::{Value, json};

/// The login item's state, as `SMAppServiceStatus` says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    Off,
    On,
    /// registered, but the user has to allow it in System Settings → Login Items
    NeedsApproval,
    /// not a bundled app (a debug binary) or macOS can't find it
    Unavailable,
}

impl LoginState {
    pub fn from_status(n: isize) -> Self {
        match n {
            // 3 (SMAppServiceStatusNotFound): what macOS answers for an app that was never
            // registered, e.g. one opened from a build folder; registering it works
            0 | 3 => LoginState::Off,
            1 => LoginState::On,
            2 => LoginState::NeedsApproval,
            _ => LoginState::Unavailable,
        }
    }

    pub fn to_json(self) -> Value {
        let (state, on, detail) = match self {
            LoginState::Off => ("off", false, ""),
            LoginState::On => ("on", true, ""),
            LoginState::NeedsApproval => {
                ("needs_approval", true, "Allow StickyBot in System Settings → General → Login Items.")
            }
            LoginState::Unavailable => {
                ("unavailable", false, "Only the bundled app can start at login (not a binary run from a terminal).")
            }
        };
        json!({"available": self != LoginState::Unavailable, "state": state, "on": on, "detail": detail})
    }
}

pub trait LoginItem: Send + Sync {
    fn state(&self) -> LoginState;
    /// Registers / unregisters; the state afterwards, or why it failed.
    fn set(&self, on: bool) -> Result<LoginState, String>;
}

/// No login item (the headless binary).
pub struct NoLoginItem;

impl LoginItem for NoLoginItem {
    fn state(&self) -> LoginState {
        LoginState::Unavailable
    }
    fn set(&self, _on: bool) -> Result<LoginState, String> {
        Err("only the menu bar app can start at login".into())
    }
}

/// In memory, for tests.
#[derive(Default)]
pub struct FakeLoginItem {
    pub on: std::sync::Mutex<bool>,
}

impl LoginItem for FakeLoginItem {
    fn state(&self) -> LoginState {
        if *self.on.lock().unwrap() { LoginState::On } else { LoginState::Off }
    }
    fn set(&self, on: bool) -> Result<LoginState, String> {
        *self.on.lock().unwrap() = on;
        Ok(self.state())
    }
}
