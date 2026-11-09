//! Experimental: something to read on the home screen when nothing needs you (a port of
//! `feed.py`). Sources (free, no keys), config `idle_content`: `facts` (uselessfacts.jsph.pl),
//! `hn` (top Hacker News stories), `onthisday` (Wikipedia's "on this day"), `wikipedia` (its
//! featured article). Refreshed every 30 minutes (5 when nothing came).

use crate::net::Http;
use chrono::Datelike;
use serde_json::{Map, Value, json};

pub const REFRESH_S: f64 = 30.0 * 60.0;

fn rand_below(n: usize) -> usize {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("system randomness");
    (u64::from_le_bytes(b) % n.max(1) as u64) as usize
}

fn shuffle<T>(v: &mut [T]) {
    for i in (1..v.len()).rev() {
        v.swap(i, rand_below(i + 1));
    }
}

#[derive(Default)]
pub struct Feed {
    /// `[{"k": "fact"|"news"|"wiki"|"history", "a": title, "b": text}]`
    pub items: Vec<Value>,
    next: f64,
    i: usize,
}

impl Feed {
    /// Settings changed: fetch again on the next refresh.
    pub fn due_now(&mut self) {
        self.next = 0.0;
    }

    /// Blocking (network). `today`: (year, month, day) for the Wikipedia feeds.
    pub fn refresh(&mut self, http: &dyn Http, cfg: &Map<String, Value>, now: f64, today: (i32, u32, u32)) {
        let sources: Vec<&str> =
            cfg.get("idle_content").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if sources.is_empty() {
            self.items.clear();
            return;
        }
        if now < self.next {
            return;
        }
        let mut items = Vec::new();
        let (y, m, d) = today;
        if sources.contains(&"facts") {
            for _ in 0..3 {
                match http.get_json("https://uselessfacts.jsph.pl/api/v2/facts/random?language=en", &[]) {
                    Ok(v) => match v["text"].as_str() {
                        Some(t) => items.push(json!({"k": "fact", "a": "Fun fact", "b": t.trim()})),
                        None => break,
                    },
                    Err(e) => {
                        log::info!("facts: {e}");
                        break;
                    }
                }
            }
        }
        if sources.contains(&"hn") {
            let r = (|| -> Result<(), String> {
                let ids = http.get_json("https://hacker-news.firebaseio.com/v0/topstories.json", &[])?;
                for id in ids.as_array().into_iter().flatten().take(3) {
                    let it = http.get_json(&format!("https://hacker-news.firebaseio.com/v0/item/{id}.json"), &[])?;
                    items.push(json!({"k": "news", "a": "Hacker News", "b": it["title"].as_str().ok_or("no title")?}));
                }
                Ok(())
            })();
            if let Err(e) = r {
                log::info!("hn: {e}");
            }
        }
        if sources.contains(&"wikipedia") {
            match http.get_json(&format!("https://api.wikimedia.org/feed/v1/wikipedia/en/featured/{y}/{m:02}/{d:02}"), &[]) {
                Ok(v) => {
                    let tfa = &v["tfa"];
                    let title = tfa["titles"]["normalized"].as_str().or(tfa["title"].as_str()).unwrap_or("");
                    let extract = tfa["extract"].as_str().unwrap_or("").trim();
                    if !title.is_empty() && !extract.is_empty() {
                        let first = extract.split(". ").next().unwrap_or("").trim_end_matches('.');
                        items.push(json!({"k": "wiki", "a": "Wikipedia", "b": format!("{first}.")}));
                    }
                }
                Err(e) => log::info!("wikipedia: {e}"),
            }
        }
        if sources.contains(&"onthisday") {
            match http.get_json(&format!("https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/selected/{m:02}/{d:02}"), &[]) {
                Ok(v) => {
                    let mut sel: Vec<&Value> = v["selected"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
                    shuffle(&mut sel);
                    for p in sel.into_iter().take(2) {
                        let Some(text) = p["text"].as_str() else { continue };
                        let year = match &p["year"] {
                            Value::Null => "None".to_string(),
                            y => y.as_str().map(str::to_string).unwrap_or_else(|| y.to_string()),
                        };
                        items.push(json!({"k": "history", "a": "On this day", "b": format!("{year}: {}", text.trim())}));
                    }
                }
                Err(e) => log::info!("onthisday: {e}"),
            }
        }
        let got = !items.is_empty();
        if got {
            shuffle(&mut items);
            self.items = items;
        }
        self.next = now + if got { REFRESH_S } else { 5.0 * 60.0 };
        log::info!("feed: {} items", self.items.len());
    }

    pub fn next_item(&mut self) -> Option<Value> {
        if self.items.is_empty() {
            return None;
        }
        self.i = (self.i + 1) % self.items.len();
        Some(self.items[self.i].clone())
    }
}

/// Today in local time, for [`Feed::refresh`].
pub fn today() -> (i32, u32, u32) {
    let t = chrono::Local::now();
    (t.year(), t.month(), t.day())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeHttp;

    #[test]
    fn every_source_like_python() {
        let http = FakeHttp::default();
        http.set("https://uselessfacts.jsph.pl/api/v2/facts/random?language=en", json!({"text": " Otters hold hands. "}));
        http.set("https://hacker-news.firebaseio.com/v0/topstories.json", json!([11, 22, 33, 44]));
        for (id, t) in [(11, "Rust 2.0"), (22, "Show HN: a stick"), (33, "Ask HN: why")] {
            http.set(&format!("https://hacker-news.firebaseio.com/v0/item/{id}.json"), json!({"title": t}));
        }
        http.set(
            "https://api.wikimedia.org/feed/v1/wikipedia/en/featured/2026/11/09",
            json!({"tfa": {"titles": {"normalized": "Otter"}, "extract": "The otter is a mammal. It swims."}}),
        );
        http.set(
            "https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/selected/11/09",
            json!({"selected": [{"year": 1989, "text": "The Berlin Wall falls. "}]}),
        );
        let mut f = Feed::default();
        let cfg = json!({"idle_content": ["facts", "hn", "onthisday", "wikipedia"]}).as_object().unwrap().clone();
        f.refresh(&http, &cfg, 0.0, (2026, 11, 9));
        let mut got: Vec<String> = f.items.iter().map(|v| format!("{}|{}|{}", v["k"].as_str().unwrap(), v["a"].as_str().unwrap(), v["b"].as_str().unwrap())).collect();
        got.sort();
        assert_eq!(
            got,
            [
                "fact|Fun fact|Otters hold hands.",
                "fact|Fun fact|Otters hold hands.",
                "fact|Fun fact|Otters hold hands.",
                "history|On this day|1989: The Berlin Wall falls.",
                "news|Hacker News|Ask HN: why",
                "news|Hacker News|Rust 2.0",
                "news|Hacker News|Show HN: a stick",
                "wiki|Wikipedia|The otter is a mammal.",
            ]
        );
        assert!(f.next_item().is_some());
        // not due again for 30 min
        let n = http.asked().len();
        f.refresh(&http, &cfg, 60.0, (2026, 11, 9));
        assert_eq!(http.asked().len(), n);
    }

    #[test]
    fn nothing_picked_or_offline() {
        let http = FakeHttp::default();
        let mut f = Feed { items: vec![json!({"k": "fact"})], ..Feed::default() };
        f.refresh(&http, &Map::new(), 0.0, (2026, 1, 1));
        assert!(f.items.is_empty() && f.next_item().is_none());
        let cfg = json!({"idle_content": ["facts", "hn"]}).as_object().unwrap().clone();
        f.refresh(&http, &cfg, 0.0, (2026, 1, 1));
        assert!(f.items.is_empty());
        let n = http.asked().len();
        f.refresh(&http, &cfg, 299.0, (2026, 1, 1));
        assert_eq!(http.asked().len(), n); // retried after 5 min
        f.refresh(&http, &cfg, 300.0, (2026, 1, 1));
        assert!(http.asked().len() > n);
    }
}
