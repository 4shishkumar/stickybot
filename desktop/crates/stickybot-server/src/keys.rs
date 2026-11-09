//! API keys in the macOS Keychain (a port of `keychain.py`'s key handling): a generic password,
//! service `com.stickkey.agent`, account `openrouter` — the same item the Python app saves, so a
//! key set there works here (macOS asks once whether this app may read it).
//!
//! Through the Security framework (`security-framework`: SecItem*), never the `security` command: a
//! key on a command line shows in the process list. The key is write-only from the outside: the
//! dashboard can save or remove it and see "saved (…abcd)", but no API returns it, nothing logs it
//! and it never goes into config.json. `STICK_AGENT_OPENROUTER_KEY` overrides the saved key.
//!
//! The store is pluggable ([`KeyStore`]): the fake stick (every test) gets [`MemoryStore`], so tests
//! never reach the real Keychain.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SERVICE: &str = "com.stickkey.agent";

/// The environment variable that overrides an account's saved key.
pub fn env_name(account: &str) -> Option<&'static str> {
    (account == "openrouter").then_some("STICK_AGENT_OPENROUTER_KEY")
}

/// Where keys are kept.
pub trait KeyStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Whether there was one.
    fn delete(&self, account: &str) -> Result<bool, String>;
}

/// Keys in memory (tests, the fake stick).
#[derive(Default)]
pub struct MemoryStore {
    pub items: Mutex<HashMap<String, String>>,
}

impl KeyStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.items.lock().unwrap().get(account).cloned())
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.items.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<bool, String> {
        Ok(self.items.lock().unwrap().remove(account).is_some())
    }
}

/// The login Keychain (macOS).
#[cfg(target_os = "macos")]
pub struct MacKeychain;

#[cfg(target_os = "macos")]
const NOT_FOUND: i32 = -25300; // errSecItemNotFound

#[cfg(target_os = "macos")]
impl KeyStore for MacKeychain {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        match security_framework::passwords::get_generic_password(SERVICE, account) {
            Ok(b) => Ok(Some(String::from_utf8(b).map_err(|_| "the saved key isn't text".to_string())?)),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, account, secret.as_bytes())
            .map_err(|e| e.to_string())
    }
    fn delete(&self, account: &str) -> Result<bool, String> {
        match security_framework::passwords::delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(true),
            Err(e) if e.code() == NOT_FOUND => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The store plus a cache (a Keychain read per voice request isn't needed).
pub struct Keys {
    store: Mutex<Arc<dyn KeyStore>>,
    cache: Mutex<HashMap<String, Option<String>>>,
}

impl Keys {
    pub fn new(store: Arc<dyn KeyStore>) -> Self {
        Self { store: Mutex::new(store), cache: Mutex::default() }
    }

    /// The real Keychain on macOS (in memory elsewhere).
    pub fn system() -> Self {
        #[cfg(target_os = "macos")]
        return Self::new(Arc::new(MacKeychain));
        #[cfg(not(target_os = "macos"))]
        return Self::new(Arc::new(MemoryStore::default()));
    }

    pub fn set_store(&self, store: Arc<dyn KeyStore>) {
        *self.store.lock().unwrap() = store;
        self.cache.lock().unwrap().clear();
    }

    fn env(account: &str) -> Option<String> {
        env_name(account).and_then(|n| std::env::var(n).ok()).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// The key (env override first), or None. May read the Keychain: call off the async threads.
    pub fn get(&self, account: &str) -> Option<String> {
        if let Some(k) = Self::env(account) {
            return Some(k);
        }
        if let Some(k) = self.cache.lock().unwrap().get(account) {
            return k.clone();
        }
        let store = self.store.lock().unwrap().clone();
        match store.get(account) {
            Ok(k) => {
                self.cache.lock().unwrap().insert(account.into(), k.clone());
                k
            }
            Err(e) => {
                log::warn!("{account} key: {e}");
                None
            }
        }
    }

    pub fn save(&self, account: &str, secret: &str) -> Result<(), String> {
        let store = self.store.lock().unwrap().clone();
        store.set(account, secret)?;
        self.cache.lock().unwrap().insert(account.into(), Some(secret.into()));
        log::info!("{account} key saved in the Keychain");
        Ok(())
    }

    pub fn remove(&self, account: &str) -> Result<bool, String> {
        let store = self.store.lock().unwrap().clone();
        let had = store.delete(account)?;
        self.cache.lock().unwrap().insert(account.into(), None);
        log::info!("{account} key {}", if had { "removed from the Keychain" } else { "wasn't in the Keychain" });
        Ok(had)
    }

    fn describe(key: Option<&str>, source: &str, account: &str) -> Value {
        let hint = key.map(|k| {
            let n = k.chars().count();
            if n >= 12 { format!("…{}", k.chars().skip(n - 4).collect::<String>()) } else { "…".into() }
        });
        json!({"saved": key.is_some(), "source": key.map(|_| source), "hint": hint, "env_var": env_name(account)})
    }

    /// `{"saved", "source": "env" | "keychain" | null, "hint": "…abcd", "env_var"}` — never the key.
    /// May read the Keychain.
    pub fn status(&self, account: &str) -> Value {
        if let Some(e) = Self::env(account) {
            return Self::describe(Some(&e), "env", account);
        }
        Self::describe(self.get(account).as_deref(), "keychain", account)
    }

    /// [`Keys::status`] without reading the Keychain (for `/api/status`): `saved: null` = not looked yet.
    pub fn known_status(&self, account: &str) -> Value {
        if let Some(e) = Self::env(account) {
            return Self::describe(Some(&e), "env", account);
        }
        match self.cache.lock().unwrap().get(account) {
            Some(k) => Self::describe(k.as_deref(), "keychain", account),
            None => json!({"saved": null, "source": null, "hint": null, "env_var": env_name(account)}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = concat!("sk-or-v1-", "0123456789abcdef", "0123456789abcdef");

    #[test]
    fn status_never_has_the_key() {
        let mem = Arc::new(MemoryStore::default());
        let k = Keys::new(mem.clone());
        if Keys::env("openrouter").is_some() {
            return; // a developer's override in the environment
        }
        assert_eq!(k.status("openrouter")["saved"], false);
        k.save("openrouter", KEY).unwrap();
        let st = k.status("openrouter");
        assert_eq!(
            st,
            json!({"saved": true, "source": "keychain", "hint": "…cdef", "env_var": "STICK_AGENT_OPENROUTER_KEY"})
        );
        assert!(!st.to_string().contains(KEY));
        assert_eq!(mem.items.lock().unwrap()["openrouter"], KEY);
        assert!(k.remove("openrouter").unwrap());
        assert_eq!(k.status("openrouter")["saved"], false);
    }

    #[test]
    fn known_status_doesnt_read_the_store() {
        if Keys::env("openrouter").is_some() {
            return;
        }
        let mem = Arc::new(MemoryStore::default());
        mem.items.lock().unwrap().insert("openrouter".into(), KEY.into()); // saved earlier, not read yet
        let k = Keys::new(mem);
        assert_eq!(k.known_status("openrouter")["saved"], Value::Null);
        assert_eq!(k.get("openrouter").as_deref(), Some(KEY));
        assert_eq!(k.known_status("openrouter")["hint"], "…cdef");
    }
}
