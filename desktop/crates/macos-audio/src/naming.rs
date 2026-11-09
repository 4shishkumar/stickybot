//! Names and UIDs of the "<stick>-bt-mic" device, its tap and the probe's check device.
//! Mirrors `mac-agent/stick_agent/stick_mic_device.py` (slug, name_for, UID, TAP_SUFFIX, CHECK_SUFFIX).

/// The fallback name: no stick picked yet, or its name has nothing usable.
pub const NAME: &str = "stickybot-bt-mic";
/// The Python agent's aggregate UID. The Rust app never uses it: the Python agent destroys every
/// aggregate with this UID as "left over", and the Rust app's cleanup must never touch the agent's.
pub const UID: &str = "com.stickkey.wireless-mic";
/// The Rust app's aggregate UID. The *name* is the same as the Python agent's ("<stick>-bt-mic"),
/// because Wispr Flow ranks microphones by name: users keep their Wispr settings.
pub const RUST_UID: &str = "com.stickkey.rust-wireless-mic";
/// The fixed name before agent 1.8.1.
pub const OLD_NAME: &str = "stick-mic-bluetooth";
/// A tap's name = the aggregate's UID + this (how stale ones are found).
pub const TAP_SUFFIX: &str = ".tap";
/// The probe's device: UID + this, name + "-check" (never the real one).
pub const CHECK_SUFFIX: &str = ".check";

/// The spike's own device: distinct from everything the running agent owns, so cleanup by UID
/// can never touch the agent's "stickybot-bt-mic".
pub const SPIKE_NAME: &str = "spike-rust-mic";
pub const SPIKE_UID: &str = "com.stickkey.spike-rust-mic";

/// 'Arbaaz StickS3' -> 'arbaaz-sticks3' (same rule as the firmware's USB mic name and the
/// Python `slug()`: lowercase, ASCII alphanumerics kept, any run of other characters becomes
/// one '-', no leading/trailing '-').
pub fn slug(stick_name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in stick_name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// The Bluetooth mic's name for this stick: '<stick>-bt-mic' ('stickybot-bt-mic' if the name has
/// nothing usable).
pub fn name_for(stick_name: &str) -> String {
    let s = slug(stick_name);
    if s.is_empty() { NAME.to_string() } else { format!("{s}-bt-mic") }
}

/// A saved `wispr_mic_name` that is an older automatic name ("stick-mic-bluetooth",
/// "<slug>-mic-bluetooth", agent 1.8.1-1.8.6): it means "automatic", so the mic follows the
/// stick's name. Anything else was typed in and wins.
pub fn is_auto_name(name: &str) -> bool {
    let name = name.trim();
    if name == OLD_NAME {
        return true;
    }
    // [a-z0-9]+(-[a-z0-9]+)*-mic-bluetooth
    let Some(stem) = name.strip_suffix("-mic-bluetooth") else { return false };
    !stem.is_empty()
        && stem.split('-').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// The names of one device + its tap, and of the probe's check device + its tap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    pub name: String,
    pub uid: String,
}

impl Names {
    pub fn new(name: impl Into<String>, uid: impl Into<String>) -> Self {
        Self { name: name.into(), uid: uid.into() }
    }

    /// The Rust app's device for this stick: the Python app's name, the Rust UID.
    pub fn rust(name: impl Into<String>) -> Self {
        Self::new(name, RUST_UID)
    }

    /// The spike's device ("spike-rust-mic", "com.stickkey.spike-rust-mic").
    pub fn spike() -> Self {
        Self::new(SPIKE_NAME, SPIKE_UID)
    }

    /// The tap's name: `<uid>.tap`.
    pub fn tap_name(&self) -> String {
        format!("{}{TAP_SUFFIX}", self.uid)
    }

    /// The probe's device: `<name>-check`, UID `<uid>.check` (so its tap is `<uid>.check.tap`).
    pub fn check(&self) -> Self {
        Self::new(format!("{}-check", self.name), format!("{}{CHECK_SUFFIX}", self.uid))
    }

    /// Aggregate UIDs that belong to us (the device and its check device): what cleanup removes.
    pub fn owned_device_uids(&self) -> [String; 2] {
        [self.uid.clone(), self.check().uid]
    }

    /// Tap names that belong to us.
    pub fn owned_tap_names(&self) -> [String; 2] {
        [self.tap_name(), self.check().tap_name()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_matches_python() {
        assert_eq!(slug("Arbaaz StickS3"), "arbaaz-sticks3");
        assert_eq!(slug("  --My  Stick!!  "), "my-stick");
        assert_eq!(slug("Ünïcode Stick"), "n-code-stick"); // non-ASCII letters are separators, like Python
        assert_eq!(slug("a__b"), "a-b");
        assert_eq!(slug(""), "");
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug("StickyBot"), "stickybot");
    }

    #[test]
    fn name_for_falls_back() {
        assert_eq!(name_for("Arbaaz StickS3"), "arbaaz-sticks3-bt-mic");
        assert_eq!(name_for(""), NAME);
        assert_eq!(name_for("***"), NAME);
    }

    #[test]
    fn names_mirror_python_suffixes() {
        let n = Names::new(NAME, UID);
        assert_eq!(n.tap_name(), "com.stickkey.wireless-mic.tap");
        let c = n.check();
        assert_eq!(c.name, "stickybot-bt-mic-check");
        assert_eq!(c.uid, "com.stickkey.wireless-mic.check");
        assert_eq!(c.tap_name(), "com.stickkey.wireless-mic.check.tap");
    }

    /// test_stick_mic_device.py `test_names_follow_the_stick`
    #[test]
    fn names_follow_the_stick() {
        assert_eq!(slug("  stick-key "), "stick-key");
        assert_eq!(slug("Ana's  key!"), "ana-s-key");
        assert_eq!(name_for("StickyBot"), "stickybot-bt-mic"); // a default-named stick
        assert_eq!(name_for("stick-key"), "stick-key-bt-mic");
        assert_eq!(name_for("!!"), "stickybot-bt-mic"); // no stick picked yet
    }

    #[test]
    fn old_automatic_names_mean_automatic() {
        for saved in
            ["stick-mic-bluetooth", "arbaaz-sticks3-mic-bluetooth", "stick-key-mic-bluetooth", "  desk-mic-bluetooth "]
        {
            assert!(is_auto_name(saved), "{saved}");
        }
        for typed in ["my mic", "Desk Mic-bluetooth", "stickybot-bt-mic", "wispr mic bluetooth", "", "-mic-bluetooth"] {
            assert!(!is_auto_name(typed), "{typed}");
        }
    }

    #[test]
    fn rust_app_never_owns_the_python_agents_objects() {
        let python = Names::new(NAME, UID);
        let rust = Names::rust(NAME);
        assert_eq!(rust.name, python.name); // same name: Wispr's ranking finds it
        for u in rust.owned_device_uids() {
            assert!(!python.owned_device_uids().contains(&u));
        }
        for t in rust.owned_tap_names() {
            assert!(!python.owned_tap_names().contains(&t));
        }
    }

    #[test]
    fn spike_never_owns_the_agents_objects() {
        let agent = Names::new(NAME, UID);
        let spike = Names::spike();
        for u in spike.owned_device_uids() {
            assert!(!agent.owned_device_uids().contains(&u));
        }
        for t in spike.owned_tap_names() {
            assert!(!agent.owned_tap_names().contains(&t));
        }
        assert_ne!(spike.name, agent.name);
    }
}
