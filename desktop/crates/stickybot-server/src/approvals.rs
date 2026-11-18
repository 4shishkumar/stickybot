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
    destinations: Vec<(String, u64)>,
    owner: String,
    tool: String,
    hint: String,
    from: String,
    answer: Option<oneshot::Sender<&'static str>>,
    bound: Option<Arc<Bound>>,
}

/// A question from a Claude session (the stick tools' `ask`): A = the first option, B = the second.
struct Ask {
    destinations: Vec<(String, u64)>,
    owner: String,
    question: String,
    options: Vec<String>,
    from: String,
    answer: Option<oneshot::Sender<String>>,
}

#[derive(Default)]
pub struct Approvals {
    prompts: Mutex<HashMap<String, Prompt>>,
    asks: Mutex<HashMap<String, Ask>>,
    slots: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// a shorter resend interval for tests
    pub resend: Mutex<Option<Duration>>,
}

impl Approvals {
    /// FIFO per stick, shared by broker and built-in hooks. Sorted acquisition prevents
    /// deadlocks for multi-stick prompts; unrelated destination sets can proceed together.
    async fn reserve(
        &self,
        destinations: &[(Arc<AppState>, u64)],
        deadline: tokio::time::Instant,
        valid: impl Fn() -> bool,
    ) -> Option<Vec<tokio::sync::OwnedMutexGuard<()>>> {
        let locks = {
            let mut ids: Vec<_> = destinations.iter().map(|(st, _)| st.device_id.clone()).collect();
            ids.sort();
            ids.dedup();
            let mut slots = self.slots.lock().unwrap();
            ids.into_iter().map(|id| slots.entry(id).or_default().clone()).collect::<Vec<_>>()
        };
        let acquire = async move {
            let mut guards = Vec::new();
            for lock in locks {
                guards.push(lock.lock_owned().await);
            }
            guards
        };
        tokio::pin!(acquire);
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        loop {
            if !valid() || tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::select! {
                guards = &mut acquire => return valid().then_some(guards),
                _ = tokio::time::sleep_until(deadline) => return None,
                _ = tick.tick() => {},
            }
        }
    }

    pub fn pending_for(&self, device: &str) -> bool {
        self.prompts
            .lock()
            .unwrap()
            .values()
            .any(|p| p.owner == device || p.destinations.iter().any(|(id, _)| id == device))
    }
    pub fn asking_for(&self, device: &str) -> bool {
        self.asks
            .lock()
            .unwrap()
            .values()
            .any(|p| p.owner == device || p.destinations.iter().any(|(id, _)| id == device))
    }

    pub fn list_for(&self, device: &str) -> Value {
        json!(
            self.prompts
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, p)| p.owner == device || p.destinations.iter().any(|(id, _)| id == device))
                .map(|(id, p)| json!({"id":id,"tool":p.tool,"hint":p.hint,"from":p.from}))
                .collect::<Vec<_>>()
        )
    }
    pub fn asks_for(&self, device: &str) -> Value {
        json!(
            self.asks
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, p)| p.owner == device || p.destinations.iter().any(|(id, _)| id == device))
                .map(|(id, p)| json!({"id":id,"question":p.question,"options":p.options,"from":p.from}))
                .collect::<Vec<_>>()
        )
    }
    pub fn decide_dashboard(&self, id: &str, decision: &str, device: &str) -> bool {
        let allowed = self
            .prompts
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|p| p.owner == device || p.destinations.iter().any(|(id, _)| id == device))
            || self
                .asks
                .lock()
                .unwrap()
                .get(id)
                .is_some_and(|p| p.owner == device || p.destinations.iter().any(|(id, _)| id == device));
        allowed && self.decide(id, decision, "dashboard")
    }

    /// Broker approvals belong to an exact connection; legacy built-in prompts retain their behavior.
    pub fn decide_from_device(&self, id: &str, decision: &str, device: &AppState) -> bool {
        let expected = (device.device_id.clone(), device.link.generation());
        if !device.link.connected() || device.link.bound_generation() != expected.1 {
            return false;
        }
        {
            let mut prompts = self.prompts.lock().unwrap();
            if let Some(p) = prompts.get_mut(id)
                && let Some(bound) = &p.bound
            {
                if !p.destinations.contains(&expected) || !bound.live() || device.host.json()["id"] != bound.binding.mac
                {
                    return false;
                }
                let Some(tx) = p.answer.take() else {
                    return false;
                };
                *bound.responder.lock().unwrap() = Some(expected);
                return tx.send(if decision == "once" { "allow" } else { "deny" }).is_ok();
            }
        }
        let allowed = self
            .prompts
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|p| p.destinations.iter().any(|(id, _)| id == &device.device_id))
            || self.asks.lock().unwrap().get(id).is_some_and(|p| p.destinations.contains(&expected));
        allowed && self.decide(id, decision, "stick")
    }
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
        let tx = self.prompts.lock().unwrap().get_mut(id).and_then(|p| {
            // Neither a dashboard click nor an ordinary ask can mint a broker approval.
            if p.bound.is_some() && matches!(decision, "once" | "allow" | "always") {
                return None;
            }
            p.answer.take()
        });
        let Some(tx) = tx else { return false };
        let allow = matches!(decision, "once" | "allow" | "always");
        log::info!("prompt {id} {} on {by}", if allow { "approved" } else { "denied" });
        tx.send(if allow { "allow" } else { "deny" }).is_ok()
    }
}

fn new_id() -> String {
    let mut b = [0u8; 16];
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
    let destinations = destinations(st, "approval_devices");
    if destinations.is_empty() {
        return None;
    }
    let deadline = tokio::time::Instant::now() + wait.min(WAIT);
    let leases = st.approvals.reserve(&destinations, deadline, || true).await?;
    let owners = destinations.iter().map(|(s, g)| (s.device_id.clone(), *g)).collect();
    let id = new_id();
    let (tx, mut rx) = oneshot::channel();
    st.approvals.prompts.lock().unwrap().insert(
        id.clone(),
        Prompt {
            destinations: owners,
            owner: st.device_id.clone(),
            tool: tool.into(),
            hint: hint.into(),
            from: origin.into(),
            answer: Some(tx),
            bound: None,
        },
    );
    let mut guard = PromptGuard {
        st: st.clone(),
        id: id.clone(),
        destinations: destinations.clone(),
        leases: Some(leases),
        strict: false,
        decision: None,
    };
    st.hub.emit("prompt", json!({"id": id, "tool": tool, "hint": hint, "origin": origin}));
    for (device, _) in &destinations {
        if device.device_id != st.device_id {
            device.hub.emit("prompt", json!({"id":id,"tool":tool,"hint":hint,"origin":origin}));
        }
    }
    let short: String = hint.chars().take(160).collect();
    let prompt = json!({"src": "cli", "prompt": {"id": id, "tool": tool, "hint": short, "from": origin}});
    let resend = st.approvals.resend.lock().unwrap().unwrap_or(RESEND);
    let mut decision = None;
    while tokio::time::Instant::now() < deadline {
        for (device, _) in &destinations {
            let generation = device.link.generation();
            device.link.with_generation(generation, device.link.send(&prompt)).await;
        }
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
    guard.decision = decision.clone();
    decision
}

/// The action and authority being approved, immutable throughout the wait.
#[derive(Clone, PartialEq, Eq)]
struct Binding {
    mac: Value,
    device: String,
    generation: Option<u64>,
    session: String,
    turn: String,
    caller: crate::tool_policy::Caller,
    tool: String,
    arguments: Value,
}
impl Binding {
    fn new(st: &AppState, ctx: &crate::tool_policy::Context, tool: &str, arguments: &Value) -> Self {
        Self {
            mac: st.host.json()["id"].clone(),
            device: st.device_id.clone(),
            generation: (tool != "calendar_join").then(|| st.link.generation()),
            session: ctx.scope.session.clone(),
            turn: ctx.scope.turn.clone(),
            caller: ctx.caller,
            tool: tool.into(),
            arguments: arguments.clone(),
        }
    }
}
struct Bound {
    binding: Binding,
    target: std::sync::Weak<AppState>,
    context: crate::tool_policy::Context,
    deadline: tokio::time::Instant,
    destinations: Vec<(std::sync::Weak<AppState>, u64)>,
    responder: Mutex<Option<(String, u64)>>,
}
impl Bound {
    fn live(&self) -> bool {
        self.context.active()
            && tokio::time::Instant::now() < self.deadline
            && self.target.upgrade().is_some_and(|st| {
                self.binding.generation.is_none_or(|generation| connection(&st, generation))
                    && st.host.json()["id"] == self.binding.mac
            })
    }
    fn responder_live(&self) -> bool {
        let responder = self.responder.lock().unwrap();
        self.destinations.iter().any(|(device, generation)| {
            device.upgrade().is_some_and(|st| {
                responder.as_ref() == Some(&(st.device_id.clone(), *generation)) && connection(&st, *generation)
            })
        })
    }
}
fn connection(st: &AppState, generation: u64) -> bool {
    st.link.connected() && st.link.generation() == generation && st.link.bound_generation() == generation
}

/// Not serializable or cloneable. Consumption takes the only grant, including on a mismatch.
pub(crate) struct Permit(Option<Arc<Bound>>);
impl Permit {
    pub(crate) fn consume(
        &mut self,
        st: &AppState,
        ctx: &crate::tool_policy::Context,
        tool: &str,
        arguments: &Value,
    ) -> Result<(), String> {
        let bound = self.0.take().ok_or("Approval already used")?;
        if bound.binding != Binding::new(st, ctx, tool, arguments)
            || !ctx.active()
            || !bound.live()
            || !bound.responder_live()
        {
            return Err("Approval is stale or does not match this action".into());
        }
        Ok(())
    }
}
struct PromptGuard {
    st: Arc<AppState>,
    id: String,
    destinations: Vec<(Arc<AppState>, u64)>,
    leases: Option<Vec<tokio::sync::OwnedMutexGuard<()>>>,
    strict: bool,
    decision: Option<String>,
}
impl Drop for PromptGuard {
    fn drop(&mut self) {
        self.st.approvals.prompts.lock().unwrap().remove(&self.id);
        let destinations = self.destinations.clone();
        let id = self.id.clone();
        let leases = self.leases.take();
        let strict = self.strict;
        let decision = self.decision.clone();
        self.st.hub.emit("prompt_done", json!({"id":id,"decision":decision}));
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _leases = leases; // Hold the queue slots until the old screen has been cleared.
                for (st, generation) in destinations {
                    let generation = if strict { generation } else { st.link.generation() };
                    // Old firmware's clear has no id: do not clear a newer prompt.
                    if !st.approvals.pending_for(&st.device_id) && !st.approvals.asking_for(&st.device_id) {
                        st.link.with_generation(generation, st.link.send(&clear_message(&st, &id))).await;
                    }
                    st.hub.emit("prompt_done", json!({"id":id}));
                }
            });
        }
    }
}

pub(crate) async fn approve_tool(
    st: &Arc<AppState>,
    ctx: &crate::tool_policy::Context,
    tool: &str,
    prepared: &crate::tool_policy::Prepared,
    wait: Duration,
) -> Result<Permit, String> {
    if !st.cfg.lock().unwrap().get("stick_approvals").and_then(Value::as_bool).unwrap_or(true) {
        return Err("Stick approvals are turned off; approval is required".into());
    }
    let destinations: Vec<_> =
        destinations(st, "approval_devices").into_iter().filter(|(s, g)| connection(s, *g)).collect();
    if destinations.is_empty() {
        return Err("Stick approval is required: no connected approval device".into());
    }
    let bound = Arc::new(Bound {
        binding: Binding::new(st, ctx, tool, &prepared.arguments),
        target: Arc::downgrade(st),
        context: ctx.clone(),
        deadline: tokio::time::Instant::now() + wait.min(WAIT),
        destinations: destinations.iter().map(|(s, g)| (Arc::downgrade(s), *g)).collect(),
        responder: Mutex::new(None),
    });
    if !bound.live() {
        return Err("The device or turn is no longer active".into());
    }
    let leases = st
        .approvals
        .reserve(&destinations, bound.deadline, || {
            bound.live() && destinations.iter().any(|(st, generation)| connection(st, *generation))
        })
        .await
        .ok_or("Approval expired or was cancelled while waiting for the stick")?;
    let (tx, mut rx) = oneshot::channel();
    let id = new_id();
    let origin = format!("{} · {}", ctx.who, st.host.json()["label"].as_str().unwrap_or("Mac"));
    {
        let mut prompts = st.approvals.prompts.lock().unwrap();
        if prompts.contains_key(&id) {
            return Err("Could not create a unique approval".into());
        }
        prompts.insert(
            id.clone(),
            Prompt {
                destinations: destinations.iter().map(|(s, g)| (s.device_id.clone(), *g)).collect(),
                owner: st.device_id.clone(),
                tool: tool.into(),
                hint: prepared.hint.clone(),
                from: origin.clone(),
                answer: Some(tx),
                bound: Some(bound.clone()),
            },
        );
    }
    let _guard = PromptGuard {
        st: st.clone(),
        id: id.clone(),
        destinations: destinations.clone(),
        leases: Some(leases),
        strict: true,
        decision: None,
    };
    let shown = &prepared.wire_hint;
    let mac_label = st.host.json()["label"].as_str().unwrap_or("Mac").to_string();
    for (device, _) in &destinations {
        device.hub.emit("prompt", json!({"id":id,"tool":tool,"hint":prepared.hint,"origin":origin}));
    }
    let resend = st.approvals.resend.lock().unwrap().unwrap_or(RESEND);
    let mut send_at = tokio::time::Instant::now();
    loop {
        if !bound.live() || !destinations.iter().any(|(s, g)| connection(s, *g)) {
            return Err("Approval expired or the device/turn disconnected".into());
        }
        if tokio::time::Instant::now() >= send_at {
            for (device, generation) in &destinations {
                if connection(device, *generation) {
                    // Shared firmware already prefixes its trusted paired Mac label.
                    let from = if device.link.multi_host() { "Agent" } else { &mac_label };
                    let prompt = json!({"src":"cli","prompt":{"id":id,"tool":tool,"hint":shown,"from":from}});
                    device.link.with_generation(*generation, device.link.send(&prompt)).await;
                }
            }
            send_at = tokio::time::Instant::now() + resend;
        }
        tokio::select! {
            answer = &mut rx => return match answer {
                Ok("allow") if bound.live() && bound.responder_live() => Ok(Permit(Some(bound))),
                _ => Err("the user didn't approve it".into()),
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

/// Why the stick can't take a question now ("" = it can) (core.py `busy_reason`).
pub fn busy_reason(st: &AppState) -> &'static str {
    if st.approvals.pending_for(&st.device_id) {
        return "an approval prompt is waiting on the stick";
    }
    if st.approvals.asking_for(&st.device_id) {
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
    generation: u64,
    answer: String,
}

impl Drop for AskGuard {
    fn drop(&mut self) {
        self.st.approvals.asks.lock().unwrap().remove(&self.id);
        let st = self.st.clone();
        let (shown, id, answer) = (self.shown, self.id.clone(), std::mem::take(&mut self.answer));
        let generation = self.generation;
        let clear = async move {
            if shown && !st.approvals.pending_for(&st.device_id) {
                st.link.with_generation(generation, st.link.send(&clear_message(&st, &id))).await; // only this question/session
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
    let generation = st.link.bound_generation();
    if st.link.generation() != generation {
        return "cancelled".into();
    }
    let mut connection = st.link.watch_state();
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
        Ask {
            destinations: vec![(st.device_id.clone(), generation)],
            owner: st.device_id.clone(),
            question: q.clone(),
            options: labels.clone(),
            from: origin.into(),
            answer: Some(tx),
        },
    );
    let mut guard = AskGuard { st: st.clone(), id: id.clone(), shown: false, generation, answer: "cancelled".into() };
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
        if !st.link.connected() || st.link.generation() != generation {
            answer = Some("cancelled".into());
            break;
        }
        let step = if st.approvals.pending_for(&st.device_id) {
            guard.shown = false;
            Duration::from_millis(500)
        } else {
            guard.shown = st.link.with_generation(generation, st.link.send(&prompt)).await || guard.shown;
            resend
        };
        let until = (tokio::time::Instant::now() + step).min(deadline);
        tokio::select! {
            result=tokio::time::timeout_at(until,&mut rx) => match result {
                Ok(Ok(choice))=>{answer=Some(choice);break;},
                Ok(Err(_))=>break,
                Err(_)=>{},
            },
            result=connection.changed()=>{
                if result.is_err() || !st.link.connected() || st.link.generation()!=generation {
                    answer=Some("cancelled".into()); break;
                }
            }
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

/// Explicit destination IDs, or the calling/default device. Captured once, not retargeted mid-flight.
pub fn destinations(st: &Arc<AppState>, setting: &str) -> Vec<(Arc<AppState>, u64)> {
    let ids = st.cfg.lock().unwrap().get(setting).and_then(Value::as_array).cloned();
    let devices = match ids {
        Some(ids) => ids.iter().filter_map(|v| v.as_str().and_then(|id| st.devices.resolve(id))).collect::<Vec<_>>(),
        None => vec![st.clone()],
    };
    devices
        .into_iter()
        .filter(|s| s.stick().is_some())
        .map(|s| {
            let generation = s.link.generation();
            (s, generation)
        })
        .collect()
}

fn clear_message(st: &AppState, id: &str) -> Value {
    if st.link.multi_host() { json!({"src":"cli","clear":id}) } else { json!({"src":"cli"}) }
}

#[cfg(test)]
mod multi_device_tests {
    use super::*;
    use crate::talk::tests::agent;
    use stick_link::Backend;

    async fn wait_for_question(st: &AppState) -> String {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(id) = st.approvals.asks_for(&st.device_id)[0]["id"].as_str() {
                    return id.to_owned();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn questions_and_home_approval_cards_do_not_leak_or_block_sibling_devices() {
        let t = agent("question-siblings", false).await;
        t.st.devices.init(&t.st).unwrap();
        let b = t.st.devices.add(&t.st, "B", "Two").unwrap();
        let mut bw = b.link.watch_state();
        bw.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        let a = t.st.clone();
        let question = tokio::spawn(async move {
            ask_user(&a, "A private question", &["Yes".into(), "No".into()], Duration::from_secs(3), "A project").await
        });
        let id = wait_for_question(&t.st).await;
        assert!(!crate::home::home_card(&b).to_string().contains("A private"));
        assert_eq!(busy_reason(&b), "");
        assert_eq!(b.prompts_waiting(), 0);
        assert_eq!(
            ask_user(&b, "B question", &["Yes".into(), "No".into()], Duration::from_millis(30), "B project").await,
            "timeout"
        );
        assert!(t.st.approvals.decide_from_device(&id, "once", &t.st));
        assert_eq!(question.await.unwrap(), "Yes");
        t.st.devices.remove(&t.st, &b.device_id).unwrap();
    }

    #[tokio::test]
    async fn disconnected_question_retires_and_late_cleanup_cannot_clear_new_session() {
        let t = agent("question-generation", false).await;
        *t.st.approvals.resend.lock().unwrap() = Some(Duration::from_millis(20));
        let generation = t.st.link.generation();
        let a = t.st.clone();
        let old = tokio::spawn(async move {
            ask_user(&a, "Old private question", &["Yes".into(), "No".into()], Duration::from_secs(3), "old").await
        });
        let old_id = wait_for_question(&t.st).await;
        t.st.link.pause(true);
        let mut states = t.st.link.watch_state();
        states.wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(1), old).await.unwrap().unwrap(), "cancelled");
        assert!(!t.st.approvals.asking_for(&t.st.device_id));
        t.st.link.pause(false);
        states.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        assert_ne!(t.st.link.generation(), generation);
        let before = t.fake.writes().len();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(!String::from_utf8(t.fake.writes()[before..].concat()).unwrap().contains("Old private question"));
        assert!(!t.st.approvals.decide_from_device(&old_id, "once", &t.st));
        let a = t.st.clone();
        let fresh = tokio::spawn(async move {
            ask_user(&a, "New question", &["Yes".into(), "No".into()], Duration::from_secs(3), "new").await
        });
        let fresh_id = wait_for_question(&t.st).await;
        let before = t.fake.writes().len();
        drop(AskGuard { st: t.st.clone(), id: old_id, shown: true, generation, answer: "cancelled".into() });
        tokio::time::sleep(Duration::from_millis(30)).await;
        let after: Vec<Value> = String::from_utf8(t.fake.writes()[before..].concat())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert!(!after.iter().any(|m| m["src"] == "cli" && m.get("prompt").is_none()));
        assert!(t.st.approvals.decide_from_device(&fresh_id, "once", &t.st));
        assert_eq!(fresh.await.unwrap(), "Yes");
    }

    #[tokio::test]
    async fn permissions_follow_reconnect_but_reject_old_connection_work() {
        let t = agent("permission-reconnect", false).await;
        *t.st.approvals.resend.lock().unwrap() = Some(Duration::from_millis(20));
        for initially_offline in [false, true] {
            let mut state = t.st.link.watch_state();
            if initially_offline {
                t.st.link.pause(true);
                state.wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
            }
            let root = t.st.clone();
            let task = tokio::spawn(async move { ask_permission(&root, "Bash", "test", "test", WAIT).await });
            let id = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if let Some(id) = t.st.approvals.list()[0]["id"].as_str() {
                        break id.to_owned();
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let old = t.st.link.generation();
            if !initially_offline {
                t.st.link.pause(true);
                state.wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
            }
            t.st.link.pause(false);
            state.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
            let before = t.fake.writes().len();
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let writes = t.fake.writes();
                    let text = String::from_utf8(writes[before..].concat()).unwrap();
                    if text
                        .lines()
                        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                        .any(|m| m["prompt"]["id"] == id)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            let stale =
                t.st.link.with_generation(old, async { t.st.approvals.decide_from_device(&id, "once", &t.st) }).await;
            assert!(!stale);
            assert!(t.st.approvals.decide_from_device(&id, "once", &t.st));
            assert!(!t.st.approvals.decide_from_device(&id, "deny", &t.st));
            assert_eq!(task.await.unwrap().as_deref(), Some("allow"));
        }
    }

    #[tokio::test]
    async fn empty_or_missing_approval_destinations_return_without_waiting() {
        let t = agent("permission-no-destinations", false).await;
        t.st.devices.init(&t.st).unwrap();
        for ids in [json!([]), json!(["removed-device"])] {
            t.st.update_config(json!({"approval_devices": ids}).as_object().unwrap().clone()).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_millis(100), ask_permission(&t.st, "Bash", "test", "test", WAIT))
                    .await
                    .unwrap(),
                None
            );
            assert!(!t.st.approvals.pending());
        }
        t.st.cfg.lock().unwrap().insert("stick".into(), Value::Null);
        t.st.cfg.lock().unwrap().insert("approval_devices".into(), json!(["primary"]));
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(100), ask_permission(&t.st, "Bash", "test", "test", WAIT))
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn fanout_accepts_one_destination_answer_and_clears_every_copy() {
        let t = agent("fanout-devices", false).await;
        t.st.devices.init(&t.st).unwrap();
        let child = t.st.devices.add(&t.st, "SECOND", "Second").unwrap();
        let outsider = t.st.devices.add(&t.st, "OTHER", "Other").unwrap();
        let mut state = child.link.watch_state();
        state.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        t.st.update_config(json!({"approval_devices":["primary",child.device_id]}).as_object().unwrap().clone())
            .unwrap();
        let root = t.st.clone();
        let pending =
            tokio::spawn(
                async move { ask_permission(&root, "Bash", "run tests", "project", Duration::from_secs(3)).await },
            );
        tokio::time::sleep(Duration::from_millis(30)).await;
        let id = t.st.approvals.list()[0]["id"].as_str().unwrap().to_owned();
        assert_eq!(busy_reason(&outsider), "");
        assert!(busy_reason(&child).contains("approval"));
        assert!(!crate::home::home_card(&outsider).to_string().contains("Bash"));
        assert!(crate::home::home_card(&child).to_string().contains("Bash"));
        assert!(!t.st.approvals.decide_from_device(&id, "once", &outsider));
        assert!(t.st.approvals.decide_from_device(&id, "once", &child));
        assert!(!t.st.approvals.decide_from_device(&id, "deny", &t.st));
        assert_eq!(pending.await.unwrap(), Some("allow".into()));
        let Backend::Fake(fake) = &child.backend else { panic!("fake only") };
        let messages: Vec<Value> = String::from_utf8(fake.writes().concat())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        assert!(messages.iter().any(|m| m["prompt"]["id"] == id));
        assert_eq!(messages.last().unwrap(), &json!({"src":"cli"}));
        assert!(!t.st.approvals.pending());
        t.st.devices.remove(&t.st, &child.device_id).unwrap();
        t.st.devices.remove(&t.st, &outsider.device_id).unwrap();
    }
}

#[cfg(test)]
pub(crate) async fn approve_next(st: &AppState, decision: &str) -> String {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(id) = st.approvals.list_for(&st.device_id)[0]["id"].as_str() {
                let id = id.to_owned();
                assert!(st.approvals.decide_from_device(&id, decision, st));
                return id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("approval must appear")
}

#[cfg(test)]
mod broker_tests {
    use super::*;
    use crate::{
        talk::tests::agent,
        tool_policy::{Context, Prepared},
    };

    async fn grant(st: &Arc<AppState>, ctx: &Context, wait: Duration) -> (Permit, Prepared, String) {
        let action = Prepared::resolve(st, "open_url", json!({"url":"HTTPS://EXAMPLE.COM:443/a"}).as_object().unwrap())
            .await
            .unwrap();
        let (permit, id) = tokio::join!(approve_tool(st, ctx, "open_url", &action, wait), approve_next(st, "once"));
        (permit.unwrap(), action, id)
    }

    async fn prompt_for(st: &AppState, tool: &str) -> String {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                for p in st.approvals.list().as_array().unwrap() {
                    if p["tool"] == tool {
                        return p["id"].as_str().unwrap().to_string();
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    fn permission_task(
        st: Arc<AppState>,
        legacy: bool,
        wait: Duration,
    ) -> tokio::task::JoinHandle<Result<String, String>> {
        tokio::spawn(async move {
            if legacy {
                ask_permission(&st, "Bash", "pwd", "terminal", wait).await.ok_or("expired".into())
            } else {
                let ctx = Context::internal("test");
                let prepared =
                    Prepared::resolve(&st, "open_url", json!({"url":"https://example.com/a"}).as_object().unwrap())
                        .await?;
                let mut grant = approve_tool(&st, &ctx, "open_url", &prepared, wait).await?;
                grant.consume(&st, &ctx, "open_url", &prepared.arguments)?;
                Ok("allow".into())
            }
        })
    }

    #[tokio::test]
    async fn hook_and_broker_permissions_queue_in_both_orders() {
        for legacy_first in [true, false] {
            let t = agent(&format!("broker-queue-{legacy_first}"), false).await;
            let first = permission_task(t.st.clone(), legacy_first, WAIT);
            let first_id = prompt_for(&t.st, if legacy_first { "Bash" } else { "open_url" }).await;
            let mut second = permission_task(t.st.clone(), !legacy_first, WAIT);
            assert!(tokio::time::timeout(Duration::from_millis(80), &mut second).await.is_err());
            assert_eq!(t.st.approvals.list().as_array().unwrap().len(), 1);
            assert_eq!(t.st.approvals.list()[0]["id"], first_id);
            assert!(t.st.approvals.decide_from_device(&first_id, "once", &t.st));
            assert_eq!(first.await.unwrap().unwrap(), "allow");
            let second_id = prompt_for(&t.st, if legacy_first { "open_url" } else { "Bash" }).await;
            assert_ne!(first_id, second_id);
            assert!(t.st.approvals.decide_from_device(&second_id, "once", &t.st));
            assert_eq!(second.await.unwrap().unwrap(), "allow");
        }
    }

    #[tokio::test]
    async fn queue_expiry_and_cancellation_do_not_replace_or_block_other_prompts() {
        let t = agent("broker-queue-cancel", false).await;
        let first = permission_task(t.st.clone(), true, WAIT);
        let id = prompt_for(&t.st, "Bash").await;
        assert!(permission_task(t.st.clone(), false, Duration::from_millis(80)).await.unwrap().is_err());
        assert_eq!(t.st.approvals.list()[0]["id"], id);
        let mut cancelled = permission_task(t.st.clone(), false, WAIT);
        assert!(tokio::time::timeout(Duration::from_millis(80), &mut cancelled).await.is_err());
        cancelled.abort();
        let _ = cancelled.await;
        assert!(t.st.approvals.decide_from_device(&id, "once", &t.st));
        assert_eq!(first.await.unwrap().unwrap(), "allow");
        let next = permission_task(t.st.clone(), false, WAIT);
        approve_next(&t.st, "once").await;
        assert_eq!(next.await.unwrap().unwrap(), "allow");
        assert!(t.ran.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn offline_calendar_target_can_use_another_connected_approval_stick() {
        let t = agent("calendar-offline-approval", false).await;
        t.st.devices.init(&t.st).unwrap();
        let responder = t.st.devices.add(&t.st, "SECOND", "Approver").unwrap();
        responder.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        let calendar = macos_calendar::fake::FakeCalendar::new();
        calendar.set_events(vec![macos_calendar::Event {
            id: "meeting".into(),
            title: "Standup".into(),
            start: 1_800_000_900,
            end: 1_800_001_800,
            url: "https://meet.google.com/abc-defg-hij".into(),
            ..Default::default()
        }]);
        t.st.calendar.set_source(Arc::new(calendar));
        t.st.calendar.set_clock(1_800_000_180, 0);
        crate::connections::calendar::connect(&t.st).await.unwrap();
        t.st.cfg.lock().unwrap().insert("approval_devices".into(), json!([responder.device_id]));
        t.st.link.pause(true);
        t.st.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
        let ctx = Context::internal("test");
        let args = json!({});
        let (result, _) =
            tokio::join!(crate::mcp::call(&t.st, "calendar_join", &args, &ctx), approve_next(&responder, "once"));
        assert!(!result.1, "{}", result.0);
        assert_eq!(
            *t.ran.lock().unwrap(),
            [crate::commands::Action::Open(vec!["https://meet.google.com/abc-defg-hij".into()])]
        );
        responder.link.pause(true);
        responder.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
        let failed = crate::mcp::call(&t.st, "calendar_join", &json!({}), &Context::internal("test")).await;
        assert!(failed.1);
        assert_eq!(t.ran.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn exact_action_is_consumed_once_and_cannot_be_replayed() {
        let t = agent("broker-once", false).await;
        let ctx = Context::internal("test");
        let (mut permit, action, id) = grant(&t.st, &ctx, WAIT).await;
        assert_eq!(action.hint, "https://example.com/a");
        assert!(permit.consume(&t.st, &ctx, "open_url", &action.arguments).is_ok());
        assert!(permit.consume(&t.st, &ctx, "open_url", &action.arguments).is_err());
        assert!(!t.st.approvals.decide_from_device(&id, "once", &t.st));
        assert!(t.ran.lock().unwrap().is_empty()); // only the broker executes, never the approval API
    }

    #[tokio::test]
    async fn grants_bind_url_tool_mac_device_caller_session_and_turn() {
        let t = agent("broker-bindings", false).await;
        let other = agent("broker-other-mac", false).await;
        t.st.devices.init(&t.st).unwrap();
        let sibling = t.st.devices.add(&t.st, "SECOND", "Second").unwrap();
        sibling.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        for field in ["url", "tool", "mac", "device", "caller", "session", "turn"] {
            let ctx = Context::internal("test");
            let (mut permit, action, _) = grant(&t.st, &ctx, WAIT).await;
            let mut changed = ctx.clone();
            let mut args = action.arguments.clone();
            let mut tool = "open_url";
            let target = match field {
                "mac" => &other.st,
                "device" => &sibling,
                _ => &t.st,
            };
            match field {
                "url" => args = json!({"open":["https://example.com/b"]}),
                "tool" => tool = "calendar_join",
                "caller" => changed.caller = crate::tool_policy::Caller::Mcp,
                "session" => changed.scope.session = "different-session".into(),
                "turn" => changed.scope.turn = "different-turn".into(),
                _ => {}
            }
            assert!(permit.consume(target, &changed, tool, &args).is_err(), "{field}");
            assert!(permit.consume(&t.st, &ctx, "open_url", &action.arguments).is_err());
        }
    }

    #[tokio::test]
    async fn expiry_turn_end_and_disconnect_invalidate_grants() {
        let t = agent("broker-stale", false).await;
        for reason in ["expiry", "turn", "disconnect"] {
            let ctx = Context::internal("test");
            let wait = WAIT;
            let (mut permit, action, _) = grant(&t.st, &ctx, wait).await;
            match reason {
                "expiry" => {
                    tokio::time::pause();
                    tokio::time::advance(WAIT + Duration::from_secs(1)).await;
                    tokio::time::resume();
                }
                "turn" => ctx.scope.cancel(),
                _ => {
                    t.st.link.pause(true);
                    t.st.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
                }
            }
            assert!(permit.consume(&t.st, &ctx, "open_url", &action.arguments).is_err(), "{reason}");
        }
    }

    #[tokio::test]
    async fn dashboard_and_ask_answers_cannot_authorize_and_abort_cleans_up() {
        let t = agent("broker-dashboard", false).await;
        let st = t.st.clone();
        let task = tokio::spawn(async move {
            let ctx = Context::internal("test");
            let action =
                Prepared::resolve(&st, "open_url", json!({"url":"https://example.com/a"}).as_object().unwrap())
                    .await
                    .unwrap();
            approve_tool(&st, &ctx, "open_url", &action, WAIT).await
        });
        let id = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(id) = t.st.approvals.list()[0]["id"].as_str() {
                    break id.to_owned();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!t.st.approvals.decide_dashboard(&id, "once", &t.st.device_id));
        assert!(!t.st.approvals.decide(&id, "always", "stick"));
        assert!(!t.st.approvals.answer(&id, "once", "stick"));
        task.abort();
        let _ = task.await;
        assert!(!t.st.approvals.pending());
        assert!(!t.st.approvals.decide_from_device(&id, "once", &t.st));
    }

    #[tokio::test]
    async fn only_selected_device_can_answer_and_responder_disconnect_revokes_grant() {
        let t = agent("broker-selected", false).await;
        t.st.devices.init(&t.st).unwrap();
        let other = t.st.devices.add(&t.st, "SECOND", "Second").unwrap();
        other.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        t.st.cfg.lock().unwrap().insert("approval_devices".into(), json!([other.device_id]));
        let ctx = Context::internal("test");
        let prepared =
            Prepared::resolve(&t.st, "open_url", json!({"url":"https://example.com/a"}).as_object().unwrap())
                .await
                .unwrap();
        let answer = async {
            let id = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if let Some(id) = t.st.approvals.list()[0]["id"].as_str() {
                        break id.to_string();
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(!t.st.approvals.decide_from_device(&id, "once", &t.st));
            assert!(t.st.approvals.decide_from_device(&id, "once", &other));
        };
        let (grant, ()) = tokio::join!(approve_tool(&t.st, &ctx, "open_url", &prepared, WAIT), answer);
        let mut grant = grant.unwrap();
        other.link.pause(true);
        other.link.watch_state().wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
        assert!(t.st.link.connected());
        assert!(grant.consume(&t.st, &ctx, "open_url", &prepared.arguments).is_err());
    }

    #[tokio::test]
    async fn disconnected_prompt_cannot_resume_after_reconnect() {
        let t = agent("broker-reconnect", false).await;
        let ctx = Context::internal("test");
        let st = t.st.clone();
        let call = tokio::spawn(async move {
            crate::mcp::call(&st, "open_url", &json!({"url":"https://example.com"}), &ctx).await
        });
        let id = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(id) = t.st.approvals.list()[0]["id"].as_str() {
                    break id.to_owned();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        t.st.link.pause(true);
        let mut states = t.st.link.watch_state();
        states.wait_for(|s| *s == stick_link::LinkState::Paused).await.unwrap();
        assert!(!t.st.approvals.decide_from_device(&id, "once", &t.st));
        t.st.link.pause(false);
        states.wait_for(|s| *s == stick_link::LinkState::Connected).await.unwrap();
        assert!(!t.st.approvals.decide_from_device(&id, "once", &t.st));
        assert!(tokio::time::timeout(Duration::from_secs(2), call).await.unwrap().unwrap().1);
        assert!(t.ran.lock().unwrap().is_empty());
    }
}
