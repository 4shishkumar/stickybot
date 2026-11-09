//! The few web services the home screen reads (Open-Meteo, ipinfo / geojs, the feeds) behind one
//! small trait, so tests answer from memory and never touch the network.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const USER_AGENT: &str = "StickyBot/0.1 (desk gadget)";

/// GETs a JSON document. Blocking: call it from `spawn_blocking`.
pub trait Http: Send + Sync {
    fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Value, String>;
}

/// The real one (ureq, 10 s like the Python app's aiohttp timeouts).
pub struct Ureq {
    agent: ureq::Agent,
}

impl Default for Ureq {
    fn default() -> Self {
        Self { agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).user_agent(USER_AGENT).build() }
    }
}

impl Http for Ureq {
    fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Value, String> {
        let mut req = self.agent.get(url);
        for (k, v) in query {
            req = req.query(k, v);
        }
        let resp = req.call().map_err(|e| e.to_string())?;
        let text = resp.into_string().map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}

/// Answers from a table: `url` (with the query as `?k=v&…` in the given order) -> JSON, or an
/// error for anything else. Records every URL asked for.
#[derive(Default, Clone)]
pub struct FakeHttp {
    pub answers: Arc<Mutex<HashMap<String, Value>>>,
    pub asked: Arc<Mutex<Vec<String>>>,
}

impl FakeHttp {
    pub fn set(&self, url: &str, v: Value) {
        self.answers.lock().unwrap().insert(url.to_string(), v);
    }
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
    pub fn full(url: &str, query: &[(&str, String)]) -> String {
        if query.is_empty() {
            return url.to_string();
        }
        let q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{url}?{}", q.join("&"))
    }
}

impl Http for FakeHttp {
    fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Value, String> {
        let full = Self::full(url, query);
        self.asked.lock().unwrap().push(full.clone());
        let a = self.answers.lock().unwrap();
        a.get(&full).or_else(|| a.get(url)).cloned().ok_or_else(|| format!("offline: {full}"))
    }
}
