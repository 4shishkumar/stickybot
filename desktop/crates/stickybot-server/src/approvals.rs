//! Claude Code permission prompts on the stick (core.py `ask_permission` / `_decide`): the hook
//! long-polls `POST /api/permission`; the prompt goes to the stick as Claude's Hardware Buddy
//! message (`{"src":"cli","prompt":{…}}`, re-sent every 3 s: the stick drops a prompt it hasn't
//! heard about for 30 s) and to the dashboard; A / B on the stick
//! (`{"cmd":"permission","id","decision":"once"|"deny"}`) or the dashboard decides.

use crate::AppState;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

pub const RESEND: Duration = Duration::from_secs(3);
pub const WAIT: Duration = Duration::from_secs(120);

struct Prompt {
    tool: String,
    hint: String,
    from: String,
    answer: Option<oneshot::Sender<&'static str>>,
}

#[derive(Default)]
pub struct Approvals {
    prompts: Mutex<HashMap<String, Prompt>>,
    /// a shorter resend interval for tests
    pub resend: Mutex<Option<Duration>>,
}

impl Approvals {
    /// What waits, for `GET /api/status`.
    pub fn list(&self) -> Value {
        let p = self.prompts.lock().unwrap();
        Value::Array(
            p.iter().map(|(id, p)| json!({"id": id, "tool": p.tool, "hint": p.hint, "from": p.from})).collect(),
        )
    }

    pub fn pending(&self) -> bool {
        !self.prompts.lock().unwrap().is_empty()
    }

    /// An answer from the stick or the dashboard. False if there's no such prompt (any more).
    pub fn decide(&self, id: &str, decision: &str, by: &str) -> bool {
        let tx = self.prompts.lock().unwrap().get_mut(id).and_then(|p| p.answer.take());
        let Some(tx) = tx else { return false };
        let allow = matches!(decision, "once" | "allow" | "always");
        log::info!("prompt {id} {} on {by}", if allow { "approved" } else { "denied" });
        tx.send(if allow { "allow" } else { "deny" }).is_ok()
    }
}

fn new_id() -> String {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("system randomness");
    format!("cli-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

/// Shows a permission prompt and waits for an answer: `Some("allow" | "deny")`, or `None` when
/// nobody answered within `wait` (at most 120 s) or prompts on the stick are turned off (Claude
/// Code then asks in the terminal).
pub async fn ask_permission(
    st: &Arc<AppState>,
    tool: &str,
    hint: &str,
    origin: &str,
    wait: Duration,
) -> Option<String> {
    if !st.cfg.lock().unwrap().get("stick_approvals").and_then(Value::as_bool).unwrap_or(true) {
        return None;
    }
    let id = new_id();
    let (tx, mut rx) = oneshot::channel();
    st.approvals
        .prompts
        .lock()
        .unwrap()
        .insert(id.clone(), Prompt { tool: tool.into(), hint: hint.into(), from: origin.into(), answer: Some(tx) });
    st.hub.emit("prompt", json!({"id": id, "tool": tool, "hint": hint, "origin": origin}));
    let short: String = hint.chars().take(160).collect();
    let prompt = json!({"src": "cli", "prompt": {"id": id, "tool": tool, "hint": short, "from": origin}});
    let resend = st.approvals.resend.lock().unwrap().unwrap_or(RESEND);
    let deadline = tokio::time::Instant::now() + wait.min(WAIT);
    let mut decision = None;
    while tokio::time::Instant::now() < deadline {
        st.link.send(&prompt).await;
        let until = (tokio::time::Instant::now() + resend).min(deadline);
        match tokio::time::timeout_at(until, &mut rx).await {
            Ok(Ok(d)) => {
                decision = Some(d.to_string());
                break;
            }
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    st.approvals.prompts.lock().unwrap().remove(&id);
    st.link.send(&json!({"src": "cli"})).await; // clear it on the stick
    st.hub.emit("prompt_done", json!({"id": id, "decision": decision}));
    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::talk::tests::agent;

    fn sent_prompts(t: &crate::talk::tests::T) -> Vec<Value> {
        let text = String::from_utf8(t.fake.writes().concat()).unwrap();
        text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|m| m["src"] == "cli").collect()
    }

    // --- test_core.py ----------------------------------------------------------------------------------

    #[tokio::test]
    async fn answered_on_the_stick() {
        let t = agent("perm-stick", false).await;
        let st = t.st.clone();
        let task =
            tokio::spawn(
                async move { ask_permission(&st, "Bash", "rm -rf build/", "web", Duration::from_secs(5)).await },
            );
        tokio::time::sleep(Duration::from_millis(100)).await;
        let p = sent_prompts(&t);
        let id = p[0]["prompt"]["id"].as_str().unwrap().to_string();
        assert_eq!(p[0]["prompt"]["tool"], "Bash");
        assert_eq!(t.st.approvals.list()[0]["id"], id);
        assert_eq!(t.st.status()["prompts"][0]["tool"], "Bash");
        t.fake.notify(&json!({"cmd": "permission", "id": id, "decision": "once"}));
        assert_eq!(task.await.unwrap().as_deref(), Some("allow"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sent_prompts(&t).last().unwrap(), &json!({"src": "cli"})); // cleared on the stick
        assert!(!t.st.approvals.pending());
    }

    #[tokio::test]
    async fn denied_from_the_dashboard() {
        let t = agent("perm-dash", false).await;
        let st = t.st.clone();
        let task =
            tokio::spawn(async move { ask_permission(&st, "Edit", "/etc/hosts", "web", Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let id = t.st.approvals.list()[0]["id"].as_str().unwrap().to_string();
        assert!(t.st.approvals.decide(&id, "deny", "dashboard"));
        assert_eq!(task.await.unwrap().as_deref(), Some("deny"));
    }

    #[tokio::test]
    async fn nobody_answers() {
        let t = agent("perm-none", false).await;
        *t.st.approvals.resend.lock().unwrap() = Some(Duration::from_millis(50));
        assert_eq!(ask_permission(&t.st, "Bash", "ls", "web", Duration::from_millis(200)).await, None);
        assert!(sent_prompts(&t).iter().filter(|m| m.get("prompt").is_some()).count() >= 3); // re-sent
    }

    #[tokio::test]
    async fn turned_off() {
        let t = agent("perm-off", false).await;
        t.st.cfg.lock().unwrap().insert("stick_approvals".into(), json!(false));
        assert_eq!(ask_permission(&t.st, "Bash", "ls", "web", WAIT).await, None);
        assert!(sent_prompts(&t).is_empty());
    }
}
