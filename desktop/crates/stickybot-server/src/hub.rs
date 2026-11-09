//! The dashboard's live feed (`/api/events`): the last events for a page that connects, and a
//! broadcast for pages already listening (core.py `emit`).

use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::broadcast;

const KEEP: usize = 300;

pub struct Hub {
    recent: Mutex<VecDeque<Value>>,
    tx: broadcast::Sender<Value>,
}

impl Default for Hub {
    fn default() -> Self {
        Self { recent: Mutex::new(VecDeque::with_capacity(KEEP)), tx: broadcast::channel(256).0 }
    }
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Hub {
    /// `{"t", "kind", ...data}`
    pub fn emit(&self, kind: &str, data: Value) -> Value {
        let mut ev = Map::new();
        ev.insert("t".into(), json!(now()));
        ev.insert("kind".into(), json!(kind));
        if let Value::Object(d) = data {
            ev.extend(d);
        }
        let ev = Value::Object(ev);
        {
            let mut r = self.recent.lock().unwrap();
            if r.len() == KEEP {
                r.pop_front();
            }
            r.push_back(ev.clone());
        }
        let _ = self.tx.send(ev.clone());
        ev
    }

    /// The last `n` events and a receiver for the next ones (no gap between them).
    pub fn subscribe(&self, n: usize) -> (Vec<Value>, broadcast::Receiver<Value>) {
        let r = self.recent.lock().unwrap();
        let rx = self.tx.subscribe();
        let skip = r.len().saturating_sub(n);
        (r.iter().skip(skip).cloned().collect(), rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catch_up_then_live() {
        let hub = Hub::default();
        for i in 0..5 {
            hub.emit("x", json!({"i": i}));
        }
        let (old, mut rx) = hub.subscribe(3);
        assert_eq!(old.iter().map(|e| e["i"].as_i64().unwrap()).collect::<Vec<_>>(), vec![2, 3, 4]);
        hub.emit("link", json!({"state": "connected"}));
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev["kind"], "link");
        assert_eq!(ev["state"], "connected");
        assert!(ev["t"].as_f64().unwrap() > 1.6e9);
    }

    #[test]
    fn only_the_last_300_are_kept() {
        let hub = Hub::default();
        for i in 0..310 {
            hub.emit("x", json!({"i": i}));
        }
        let (old, _) = hub.subscribe(1000);
        assert_eq!(old.len(), 300);
        assert_eq!(old[0]["i"], 10);
    }
}
