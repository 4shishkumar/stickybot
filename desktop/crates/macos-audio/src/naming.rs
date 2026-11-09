//! Names and UIDs of the "<stick>-bt-mic" device, its tap and the probe's check device.
//! Mirrors `mac-agent/stick_agent/stick_mic_device.py` (slug, name_for, UID, TAP_SUFFIX, CHECK_SUFFIX).

/// The fallback name: no stick picked yet, or its name has nothing usable.
pub const NAME: &str = "stickybot-bt-mic";
/// The running agent's aggregate UID. The spike never uses it (see [`SPIKE_UID`]).
pub const UID: &str = "com.stickkey.wireless-mic";
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
