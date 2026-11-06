//! The aggregate device's description dictionary, built as plain Rust data first (testable
//! anywhere), then turned into a CFDictionary for `AudioHardwareCreateAggregateDevice`.
//!
//! Same keys and values as `CoreAudioDevices.create_aggregate()` in stick_mic_device.py.

/// Property-list values (just what an aggregate description needs).
#[derive(Debug, Clone, PartialEq)]
pub enum Plist {
    Str(String),
    Int(i32),
    Array(Vec<Plist>),
    /// Ordered key/value pairs (CFDictionary has no order; this keeps tests readable).
    Dict(Vec<(String, Plist)>),
}

/// The CoreAudio dictionary keys (AudioHardware.h). Tests check these against the constants
/// objc2-core-audio exports, so a typo can't creep in.
pub mod keys {
    /// kAudioAggregateDeviceNameKey
    pub const NAME: &str = "name";
    /// kAudioAggregateDeviceUIDKey
    pub const UID: &str = "uid";
    /// kAudioAggregateDeviceIsPrivateKey
    pub const PRIVATE: &str = "private";
    /// kAudioAggregateDeviceIsStackedKey
    pub const STACKED: &str = "stacked";
    /// kAudioAggregateDeviceMainSubDeviceKey (the clock source)
    pub const MAIN: &str = "master";
    /// kAudioAggregateDeviceSubDeviceListKey
    pub const SUBS: &str = "subdevices";
    /// kAudioSubDeviceUIDKey
    pub const SUB_UID: &str = "uid";
    /// kAudioAggregateDeviceTapListKey
    pub const TAPS: &str = "taps";
    /// kAudioAggregateDeviceTapAutoStartKey
    pub const TAP_AUTOSTART: &str = "tapautostart";
    /// kAudioSubTapUIDKey
    pub const TAP_UID: &str = "uid";
    /// kAudioSubTapDriftCompensationKey
    pub const DRIFT: &str = "drift";
}

impl Plist {
    pub fn str(s: impl Into<String>) -> Self {
        Plist::Str(s.into())
    }

    pub fn dict(pairs: Vec<(&str, Plist)>) -> Self {
        Plist::Dict(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// A dictionary's value for `key`.
    pub fn get(&self, key: &str) -> Option<&Plist> {
        match self {
            Plist::Dict(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// A public aggregate with one sub-device, `sub_uid` (also its clock), and — with `tap_uid` — the
/// process tap whose audio is its input (drift-compensated, started with the device).
pub fn aggregate_description(name: &str, uid: &str, sub_uid: &str, tap_uid: Option<&str>) -> Plist {
    use keys::*;
    let mut pairs = vec![
        (NAME, Plist::str(name)),
        (UID, Plist::str(uid)),
        (PRIVATE, Plist::Int(0)),
        (STACKED, Plist::Int(0)),
        (MAIN, Plist::str(sub_uid)),
        (SUBS, Plist::Array(vec![Plist::dict(vec![(SUB_UID, Plist::str(sub_uid))])])),
    ];
    if let Some(tap_uid) = tap_uid {
        pairs.push((
            TAPS,
            Plist::Array(vec![Plist::dict(vec![(TAP_UID, Plist::str(tap_uid)), (DRIFT, Plist::Int(1))])]),
        ));
        pairs.push((TAP_AUTOSTART, Plist::Int(1)));
    }
    Plist::dict(pairs)
}

#[cfg(target_os = "macos")]
mod cf {
    use super::Plist;
    use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};

    fn up<T: objc2_core_foundation::Type>(x: CFRetained<T>) -> CFRetained<CFType> {
        // SAFETY: every CoreFoundation object is a CFType.
        unsafe { CFRetained::cast_unchecked(x) }
    }

    impl Plist {
        /// The CoreFoundation object (CFString / CFNumber(SInt32) / CFArray / CFDictionary).
        pub fn to_cf(&self) -> CFRetained<CFType> {
            match self {
                Plist::Str(s) => up(CFString::from_str(s)),
                Plist::Int(n) => up(CFNumber::new_i32(*n)),
                Plist::Array(items) => {
                    let owned: Vec<CFRetained<CFType>> = items.iter().map(Plist::to_cf).collect();
                    let refs: Vec<&CFType> = owned.iter().map(|x| &**x).collect();
                    up(CFArray::<CFType>::from_objects(&refs)) // the array retains its own references
                }
                Plist::Dict(_) => up(self.to_cf_dict().expect("a dict")),
            }
        }

        /// A `Plist::Dict` as a CFDictionary<CFString, CFType>.
        pub fn to_cf_dict(&self) -> Option<CFRetained<CFDictionary<CFString, CFType>>> {
            let Plist::Dict(pairs) = self else { return None };
            let keys: Vec<CFRetained<CFString>> = pairs.iter().map(|(k, _)| CFString::from_str(k)).collect();
            let vals: Vec<CFRetained<CFType>> = pairs.iter().map(|(_, v)| v.to_cf()).collect();
            let krefs: Vec<&CFString> = keys.iter().map(|x| &**x).collect();
            let vrefs: Vec<&CFType> = vals.iter().map(|x| &**x).collect();
            Some(CFDictionary::from_slices(&krefs, &vrefs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Plist {
        Plist::str(x)
    }

    #[test]
    fn tapped_aggregate_has_the_python_keys() {
        let d = aggregate_description("spike-rust-mic", "com.x", "BuiltInSpeakerDevice", Some("UUID-1"));
        assert_eq!(d.get("name"), Some(&s("spike-rust-mic")));
        assert_eq!(d.get("uid"), Some(&s("com.x")));
        assert_eq!(d.get("private"), Some(&Plist::Int(0)));
        assert_eq!(d.get("stacked"), Some(&Plist::Int(0)));
        assert_eq!(d.get("master"), Some(&s("BuiltInSpeakerDevice")));
        assert_eq!(
            d.get("subdevices"),
            Some(&Plist::Array(vec![Plist::dict(vec![("uid", s("BuiltInSpeakerDevice"))])]))
        );
        assert_eq!(
            d.get("taps"),
            Some(&Plist::Array(vec![Plist::dict(vec![("uid", s("UUID-1")), ("drift", Plist::Int(1))])]))
        );
        assert_eq!(d.get("tapautostart"), Some(&Plist::Int(1)));
        let Plist::Dict(pairs) = &d else { panic!() };
        assert_eq!(pairs.len(), 8);
    }

    #[test]
    fn untapped_aggregate_has_no_tap_keys() {
        let d = aggregate_description("n", "u", "BlackHole2ch_UID", None);
        assert_eq!(d.get("taps"), None);
        assert_eq!(d.get("tapautostart"), None);
        let Plist::Dict(pairs) = &d else { panic!() };
        assert_eq!(pairs.len(), 6);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn keys_match_coreaudio_constants() {
        use objc2_core_audio as ca;
        let k = |c: &std::ffi::CStr| c.to_str().unwrap().to_string();
        assert_eq!(keys::NAME, k(ca::kAudioAggregateDeviceNameKey));
        assert_eq!(keys::UID, k(ca::kAudioAggregateDeviceUIDKey));
        assert_eq!(keys::PRIVATE, k(ca::kAudioAggregateDeviceIsPrivateKey));
        assert_eq!(keys::STACKED, k(ca::kAudioAggregateDeviceIsStackedKey));
        assert_eq!(keys::MAIN, k(ca::kAudioAggregateDeviceMainSubDeviceKey));
        assert_eq!(keys::SUBS, k(ca::kAudioAggregateDeviceSubDeviceListKey));
        assert_eq!(keys::SUB_UID, k(ca::kAudioSubDeviceUIDKey));
        assert_eq!(keys::TAPS, k(ca::kAudioAggregateDeviceTapListKey));
        assert_eq!(keys::TAP_AUTOSTART, k(ca::kAudioAggregateDeviceTapAutoStartKey));
        assert_eq!(keys::TAP_UID, k(ca::kAudioSubTapUIDKey));
        assert_eq!(keys::DRIFT, k(ca::kAudioSubTapDriftCompensationKey));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cf_dictionary_round_trips() {
        use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFString, CFType};
        let d = aggregate_description("spike-rust-mic", "com.x", "Out", Some("UUID-1"));
        let cf = d.to_cf_dict().unwrap();
        let get = |k: &str| cf.get(&CFString::from_str(k)).unwrap();
        let str_of = |v: objc2_core_foundation::CFRetained<CFType>| v.downcast::<CFString>().unwrap().to_string();
        let int_of = |v: objc2_core_foundation::CFRetained<CFType>| v.downcast::<CFNumber>().unwrap().as_i32().unwrap();
        assert_eq!(str_of(get("name")), "spike-rust-mic");
        assert_eq!(str_of(get("master")), "Out");
        assert_eq!(int_of(get("private")), 0);
        assert_eq!(int_of(get("tapautostart")), 1);
        let taps = get("taps").downcast::<CFArray>().unwrap();
        assert_eq!(taps.len(), 1);
        // SAFETY: we put CFDictionary<CFString, CFType>s in that array.
        let tap = unsafe { taps.cast_unchecked::<CFDictionary<CFString, CFType>>() }.get(0).unwrap();
        assert_eq!(str_of(tap.get(&CFString::from_str("uid")).unwrap()), "UUID-1");
        assert_eq!(int_of(tap.get(&CFString::from_str("drift")).unwrap()), 1);
    }
}
