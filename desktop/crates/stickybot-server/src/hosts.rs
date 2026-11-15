//! Installation identity and human-readable Mac names. Names never confer authority.
use crate::{
    config,
    sysrun::{MacSystem, System},
};
use serde_json::{Value, json};
use std::sync::Mutex;
use unicode_normalization::UnicodeNormalization;

pub struct HostIdentity {
    id: String,
    name: String,
    custom: Mutex<String>,
    path: std::path::PathBuf,
    update: Mutex<()>,
}

impl HostIdentity {
    pub fn load(paths: &config::Paths, fake: bool) -> Result<Self, String> {
        if fake {
            Self::from_system(paths, &crate::sysrun::FakeSystem::new(&paths.home))
        } else {
            Self::from_system(paths, &MacSystem)
        }
    }
    pub fn from_system(paths: &config::Paths, system: &dyn System) -> Result<Self, String> {
        Self::with_name(paths, &system.computer_name())
    }

    pub fn with_name(paths: &config::Paths, name: &str) -> Result<Self, String> {
        let path = paths.home.join("host.json");
        let saved: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let id = saved["id"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(config::new_token);
        let custom = saved["custom_label"]
            .as_str()
            .filter(|s| s.len() <= 12 && s.bytes().all(|c| (32..=126).contains(&c)))
            .unwrap_or("")
            .to_owned();
        let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
        let name = if name.trim().is_empty() { "Mac".into() } else { name };
        let identity = Self { id, name, custom: Mutex::new(custom), path, update: Mutex::new(()) };
        identity.persist()?;
        Ok(identity)
    }

    fn persist(&self) -> Result<(), String> {
        let temp = self.path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(&self.json()).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(temp, &self.path).map_err(|e| e.to_string())
    }

    pub fn set_label(&self, label: &str) -> Result<(), String> {
        let _lock = self.update.lock().unwrap();
        let label = label.trim();
        if label.len() > 12 || !label.bytes().all(|c| (32..=126).contains(&c)) {
            return Err("Short name must be at most 12 printable ASCII characters".into());
        }
        let old = std::mem::replace(&mut *self.custom.lock().unwrap(), label.into());
        if let Err(e) = self.persist() {
            *self.custom.lock().unwrap() = old;
            return Err(e);
        }
        Ok(())
    }

    pub fn json(&self) -> Value {
        let custom = self.custom.lock().unwrap().clone();
        let label = if custom.is_empty() { short_name(&self.name) } else { custom.clone() };
        json!({"id":self.id,"name":self.name,"label":label,"custom_label":custom})
    }

    pub fn wire(&self) -> Value {
        self.json()
    }
}

pub fn short_name(name: &str) -> String {
    let s: String = name.nfkd().filter(|c| c.is_ascii() && !c.is_control()).take(12).collect();
    if s.trim().is_empty() { "Mac".into() } else { s.trim().into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_labels_fit_small_screen() {
        assert_eq!(short_name("José’s MacBook Pro"), "Joses MacBoo");
        assert_eq!(short_name("東京"), "Mac");
        assert!(short_name("Computer with a long name").len() <= 12);
    }
}
