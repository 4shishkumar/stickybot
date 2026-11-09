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

/// A question from a Claude session (the stick tools' `ask`): A = the first option, B = the second.
struct Ask {
    question: String,
    options: Vec<String>,
    from: String,
    answer: Option<oneshot::Sender<String>>,
}

#[derive(Default)]
pub struct Approvals {
    prompts: Mutex<HashMap<String, Prompt>>,
    asks: Mutex<HashMap<String, Ask>>,
    /// a shorter resend interval for tests
    pub resend: Mutex<Option<Duration>>,
}

impl Approvals {
    /// Questions waiting, for `GET /api/status` (`asks`).
    pub fn asks(&self) -> Value {
        let a = self.asks.lock().unwrap();
        Value::Array(
            a.iter()
                .map(|(id, a)| json!({"id": id, "question": a.question, "options": a.options, "from": a.from}))
                .collect(),
        )
    }

    pub fn asking(&self) -> bool {
        !self.asks.lock().unwrap().is_empty()
    }

    /// A / B / shake on a question (core.py `_answer`): "once" = the first option, "deny" = the
    /// second, anything else = dismissed. False if there's no such question.
    pub fn answer(&self, id: &str, decision: &str, by: &str) -> bool {
        let mut asks = self.asks.lock().unwrap();
        let Some(a) = asks.get_mut(id) else { return false };
        let Some(tx) = a.answer.take() else { return false };
        let choice = match decision {
            "once" | "allow" | "always" => a.options[0].clone(),
            "deny" if a.options.len() > 1 => a.options[1].clone(),
            _ => "dismissed".to_string(),
        };
        log::info!("question {id} answered on {by}: {choice}");
        tx.send(choice).is_ok()
    }

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

    /// An answer from the stick or the dashboard (a prompt, or a question). False if there's no
    /// such prompt (any more).
    pub fn decide(&self, id: &str, decision: &str, by: &str) -> bool {
        if self.asks.lock().unwrap().contains_key(id) {
            return self.answer(id, decision, by);
        }
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

/// Why the stick can't take a question now ("" = it can) (core.py `busy_reason`).
pub fn busy_reason(st: &AppState) -> &'static str {
    if st.approvals.pending() {
        return "an approval prompt is waiting on the stick";
    }
    if st.approvals.asking() {
        return "another question is waiting on the stick";
    }
    if matches!(st.agent_state().as_str(), "listening" | "transcribing") {
        return "the user is talking to the stick";
    }
    if st.wmic.streaming() {
        return "the user is dictating with the stick";
    }
    ""
}

/// Clears a question on the stick and in the list however `ask_user` ends (also when its caller
/// goes away: the MCP client closed the stream).
struct AskGuard {
    st: Arc<AppState>,
    id: String,
    shown: bool,
    answer: String,
}

impl Drop for AskGuard {
    fn drop(&mut self) {
        self.st.approvals.asks.lock().unwrap().remove(&self.id);
        let st = self.st.clone();
        let (shown, id, answer) = (self.shown, self.id.clone(), std::mem::take(&mut self.answer));
        let clear = async move {
            if shown && !st.approvals.pending() {
                st.link.send(&json!({"src": "cli"})).await; // clear it on the stick (never an approval)
            }
            st.hub.emit("ask_done", json!({"id": id, "answer": answer}));
        };
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(clear);
        }
    }
}

/// Shows a question on the stick (A = `options[0]`, B = `options[1]`) and the dashboard (core.py
/// `ask_user`). Returns the chosen label, "dismissed", "timeout", "busy: <why>" or "stick not
/// connected". Shown as the approval prompt's message (`"src":"cli"`) plus `"kind":"ask"` and the
/// button labels (firmware 1.7.2+). An approval that arrives meanwhile goes first: the question
/// waits (its time keeps running) and comes back once the approval is answered.
pub async fn ask_user(st: &Arc<AppState>, question: &str, options: &[String], wait: Duration, origin: &str) -> String {
    use crate::talk::stick_text;
    if !st.link.connected() {
        return "stick not connected".into();
    }
    let why = busy_reason(st);
    if !why.is_empty() {
        return format!("busy: {why}");
    }
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("system randomness");
    let id = format!("ask-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
    let q: String = stick_text(question).chars().take(160).collect();
    let labels: Vec<String> = options.iter().map(|o| stick_text(o).chars().take(16).collect()).collect();
    let (tx, mut rx) = oneshot::channel();
    st.approvals.asks.lock().unwrap().insert(
        id.clone(),
        Ask { question: q.clone(), options: labels.clone(), from: origin.into(), answer: Some(tx) },
    );
    let mut guard = AskGuard { st: st.clone(), id: id.clone(), shown: false, answer: "cancelled".into() };
    st.hub.emit(
        "ask",
        json!({"id": id, "question": question, "options": labels, "origin": origin, "timeout_s": wait.as_secs()}),
    );
    let fallback = format!(
        " - {}",
        labels.iter().zip(["A", "B"]).map(|(o, k)| format!("{k}: {o}")).collect::<Vec<_>>().join(" / ")
    );
    let from: String = stick_text(origin).chars().take(24).collect();
    let prompt = json!({"src": "cli", "prompt": {"id": id, "tool": "Question",
        "hint": format!("{q}{fallback}").chars().take(160).collect::<String>(), "from": from, "kind": "ask",
        "q": q, "a": labels[0], "b": labels.get(1).cloned().unwrap_or_default()}});
    let resend = st.approvals.resend.lock().unwrap().unwrap_or(RESEND);
    let deadline = tokio::time::Instant::now() + wait;
    let mut answer = None;
    while tokio::time::Instant::now() < deadline {
        let step = if st.approvals.pending() {
            guard.shown = false; // an approval is on the stick: wait for it
            Duration::from_millis(500)
        } else {
            guard.shown = st.link.send(&prompt).await || guard.shown;
            resend
        };
        let until = (tokio::time::Instant::now() + step).min(deadline);
        match tokio::time::timeout_at(until, &mut rx).await {
            Ok(Ok(a)) => {
                answer = Some(a);
                break;
            }
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    let answer = answer.unwrap_or_else(|| "timeout".into());
    guard.answer = answer.clone();
    answer
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
