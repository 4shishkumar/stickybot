//! Selectable brains (a port of `brains.py`): who answers a request that isn't a quick command
//! (config `brain`): "claude" (Claude Code, the user's subscription; `brain.rs`) or "openrouter"
//! (any model on OpenRouter with the user's own key; `openrouter.rs`).
//!
//! Each request goes to the brain picked in Settings, so a change applies from the next request on,
//! without a restart. Switching away forgets the old one's conversation (Claude's session process
//! stops); Claude isn't pre-warmed while another brain is picked.

use crate::AppState;
use crate::talk::{Brain, BrainEvent};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;

pub const NAMES: &[&str] = &["claude", "openrouter"];

pub fn selected(cfg: &Map<String, Value>) -> &'static str {
    match cfg.get("brain").and_then(Value::as_str) {
        Some("openrouter") => "openrouter",
        _ => "claude",
    }
}

pub struct Brains {
    claude: Arc<dyn Brain>,
    openrouter: Arc<dyn Brain>,
    st: Weak<AppState>,
    name: Mutex<&'static str>,
    /// the brain of the last request (its `cancelled` flag)
    last: Mutex<Arc<dyn Brain>>,
}

impl Brains {
    pub fn new(claude: Arc<dyn Brain>, openrouter: Arc<dyn Brain>, st: Weak<AppState>) -> Self {
        let name = st.upgrade().map(|s| selected(&s.cfg.lock().unwrap())).unwrap_or("claude");
        Self { last: Mutex::new(claude.clone()), claude, openrouter, st, name: Mutex::new(name) }
    }

    pub fn name(&self) -> &'static str {
        self.st.upgrade().map(|s| selected(&s.cfg.lock().unwrap())).unwrap_or("claude")
    }

    fn by_name(&self, n: &str) -> Arc<dyn Brain> {
        if n == "openrouter" { self.openrouter.clone() } else { self.claude.clone() }
    }

    fn current(&self) -> Arc<dyn Brain> {
        self.by_name(self.name())
    }
}

impl Brain for Brains {
    fn busy(&self) -> bool {
        self.claude.busy() || self.openrouter.busy()
    }

    fn ask(&self, text: String, events: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)> {
        self.switched(); // (a config.json edited by hand)
        if self.busy() {
            return Box::pin(async { (false, "Still working on the last request.".to_string()) });
        }
        let b = self.current();
        *self.last.lock().unwrap() = b.clone();
        b.ask(text, events)
    }

    fn cancel(&self) {
        self.claude.cancel();
        self.openrouter.cancel();
    }

    fn cancelled(&self) -> bool {
        self.last.lock().unwrap().cancelled()
    }

    fn forget(&self) {
        self.claude.forget();
        self.openrouter.forget();
    }

    fn prewarm(&self) {
        if self.name() == "claude" {
            self.claude.prewarm();
        }
    }

    fn status(&self) -> Value {
        let name = self.name();
        let mut s = match self.current().status() {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        s.insert("brain".into(), json!(name));
        Value::Object(s)
    }

    /// The picked brain changed: the old one forgets its conversation (Claude's process stops).
    fn switched(&self) {
        let name = self.name();
        let old = std::mem::replace(&mut *self.name.lock().unwrap(), name);
        if old != name {
            log::info!("brain: {old} -> {name}");
            self.by_name(old).forget();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrouter::tests::{answer, orb};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeClaude {
        asked: Mutex<Vec<String>>,
        forgot: AtomicUsize,
        warmed: AtomicUsize,
    }

    impl Brain for FakeClaude {
        fn busy(&self) -> bool {
            false
        }
        fn ask(&self, text: String, _e: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)> {
            self.asked.lock().unwrap().push(text);
            Box::pin(async { (true, "Claude says hi.".to_string()) })
        }
        fn cancel(&self) {}
        fn cancelled(&self) -> bool {
            false
        }
        fn forget(&self) {
            self.forgot.fetch_add(1, Ordering::Relaxed);
        }
        fn prewarm(&self) {
            self.warmed.fetch_add(1, Ordering::Relaxed);
        }
        fn status(&self) -> Value {
            json!({"mode": "session", "alive": false, "brain": "claude"})
        }
    }

    async fn ask(b: &dyn Brain, text: &str) -> (bool, String) {
        let (tx, _rx) = mpsc::unbounded_channel();
        b.ask(text.into(), tx).await
    }

    fn set(st: &AppState, k: &str, v: Value) {
        st.update_config([(k.to_string(), v)].into_iter().collect()).unwrap();
    }

    #[tokio::test]
    async fn switching_brains_without_a_restart() {
        let (t, orb_brain, fake) = orb("brains-switch").await;
        let claude = Arc::new(FakeClaude::default());
        let sw = Arc::new(Brains::new(claude.clone(), Arc::new(orb_brain.clone()), Arc::downgrade(&t.st)));
        t.st.set_brain(sw.clone());
        assert_eq!((sw.name(), sw.status()["brain"].as_str()), ("claude", Some("claude")));
        assert_eq!(ask(sw.as_ref(), "hi").await, (true, "Claude says hi.".to_string()));
        sw.prewarm();
        assert_eq!(claude.warmed.load(Ordering::Relaxed), 1);
        set(&t.st, "brain", json!("openrouter"));
        assert_eq!(claude.forgot.load(Ordering::Relaxed), 1); // Claude's session stops
        fake.reply(answer(&["OpenRouter says hi."], 0.0));
        assert_eq!(ask(sw.as_ref(), "hi").await, (true, "OpenRouter says hi.".to_string()));
        assert_eq!(*claude.asked.lock().unwrap(), ["hi"]);
        assert_eq!(sw.status()["brain"], "openrouter");
        assert_eq!(sw.status()["provider"], "OpenRouter");
        sw.prewarm();
        assert_eq!(claude.warmed.load(Ordering::Relaxed), 1); // no Claude Code start-up meanwhile
        set(&t.st, "brain", json!("claude"));
        assert!(orb_brain.history().is_empty());
    }

    #[tokio::test]
    async fn voice_requests_and_new_chat_go_through_openrouter() {
        let (t, orb_brain, fake) = orb("brains-voice").await;
        let claude = Arc::new(FakeClaude::default());
        t.st.set_brain(Arc::new(Brains::new(claude, Arc::new(orb_brain.clone()), Arc::downgrade(&t.st))));
        set(&t.st, "brain", json!("openrouter"));
        fake.reply(answer(&["It's Paris."], 0.0));
        crate::talk::handle_text(t.st.clone(), "capital of France?".into(), "typed").await;
        let screens = t.fake.screens();
        assert_eq!(screens.last().map(|s| (s.0.as_str(), s.2.as_str())), Some(("done", "It's Paris.")));
        assert!(!orb_brain.history().is_empty());
        crate::talk::handle_text(t.st.clone(), "new chat".into(), "typed").await;
        assert!(orb_brain.history().is_empty());
    }
}
