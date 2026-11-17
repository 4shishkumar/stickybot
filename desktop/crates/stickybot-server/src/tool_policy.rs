//! Caller policy and immutable Mac actions. The MCP broker checks these before any tool runs.

use crate::{AppState, commands, config};
use serde_json::{Map, Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    Mcp,
    Claude,
    OpenRouter,
    Internal,
}

impl Caller {
    pub fn key(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Claude => "claude",
            Self::OpenRouter => "openrouter",
            Self::Internal => "internal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Default,
    Assistant,
    Denied,
}

impl Profile {
    pub fn configured(cfg: &Map<String, Value>, caller: Caller) -> Self {
        if cfg.get("tool_profiles").is_some_and(|v| !v.is_object()) {
            return Self::Denied;
        }
        let value = cfg.get("tool_profiles").and_then(|v| v.get(caller.key()));
        match value.and_then(Value::as_str) {
            Some("default") => Self::Default,
            Some("assistant") => Self::Assistant,
            None if value.is_none() => Self::Default,
            _ => Self::Denied, // malformed saved configuration must not expand authority
        }
    }

    pub fn allows(self, tool: &str) -> bool {
        match self {
            Self::Default => true,
            Self::Denied => false,
            Self::Assistant => matches!(
                tool,
                "ask"
                    | "stick_status"
                    | "ir_list"
                    | "calendar_next"
                    | "calendar_today"
                    | "open_app"
                    | "open_url"
                    | "calendar_join"
            ),
        }
    }
}

pub fn allowed(st: &AppState, caller: Caller, tool: &str) -> bool {
    Profile::configured(&st.cfg.lock().unwrap(), caller).allows(tool)
}

/// Server-created identifiers, never a claimed session/turn supplied in tool arguments.
#[derive(Clone)]
pub struct Scope {
    pub session: String,
    pub turn: String,
    active: Arc<AtomicBool>,
}

impl Scope {
    fn new(session: String) -> Self {
        Self { session, turn: config::new_token(), active: Arc::new(AtomicBool::new(true)) }
    }
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.active.store(false, Ordering::Release);
    }
}

pub struct TurnGuard(pub Scope);
impl Drop for TurnGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub struct Session {
    id: String,
    current: Mutex<Option<Scope>>,
}
impl Default for Session {
    fn default() -> Self {
        Self { id: config::new_token(), current: Mutex::new(None) }
    }
}
impl Session {
    pub fn begin(&self) -> TurnGuard {
        let mut current = self.current.lock().unwrap();
        if let Some(old) = current.take() {
            old.cancel();
        }
        let scope = Scope::new(self.id.clone());
        *current = Some(scope.clone());
        TurnGuard(scope)
    }
    pub fn current(&self) -> Option<Scope> {
        self.current.lock().unwrap().clone().filter(Scope::active)
    }
    pub fn cancel(&self) {
        if let Some(scope) = self.current.lock().unwrap().take() {
            scope.cancel();
        }
    }
}

#[derive(Clone)]
pub struct Context {
    pub caller: Caller,
    pub who: String,
    pub scope: Scope,
    call_active: Arc<AtomicBool>,
}
pub(crate) struct CallGuard(Arc<AtomicBool>);
impl Drop for CallGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Context {
    pub fn active(&self) -> bool {
        self.scope.active() && self.call_active.load(Ordering::Acquire)
    }
    pub(crate) fn guard(&self) -> CallGuard {
        CallGuard(self.call_active.clone())
    }

    pub fn internal(who: &str) -> Self {
        Self::request(Caller::Internal, who)
    }
    /// Stateless MCP requests each receive their own server-owned session and turn.
    pub fn request(caller: Caller, who: &str) -> Self {
        Self {
            caller,
            who: who.into(),
            scope: Scope::new(config::new_token()),
            call_active: Arc::new(AtomicBool::new(true)),
        }
    }
    pub fn brain(st: &AppState, caller: Caller) -> Result<Self, String> {
        let scope = st.mcp.session.current().ok_or("The agent turn has ended")?;
        Ok(Self { caller, who: "Agent".into(), scope, call_active: Arc::new(AtomicBool::new(true)) })
    }
}

pub fn normalized_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("only http:// or https:// URLs (no spaces or control characters)".into());
    }
    let url = url::Url::parse(value).map_err(|_| "invalid URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("only http:// or https:// URLs without embedded credentials".into());
    }
    Ok(url.to_string())
}

/// Legacy firmware wraps only on spaces, including URLs. Insert soft word breaks so the
/// full destination fits both 135x240 portrait (7 lines) and 240x135 landscape (4 lines).
/// Widths: existing Inter 10 VLW glyph extent/advance + 1px margin; no transliteration.
/// Seven <=100px pieces fit portrait, and pairs fit the 220px landscape hint area.
pub(crate) fn display_hint(hint: &str) -> Result<String, String> {
    const WIDTH: [u8; 95] = [
        4, 4, 6, 8, 8, 11, 8, 4, 5, 5, 6, 8, 4, 6, 4, 5, 8, 5, 7, 7, 8, 7, 8, 7, 8, 8, 4, 4, 8, 8, 7, 6, 12, 8, 8, 9,
        9, 7, 7, 9, 9, 4, 6, 8, 7, 10, 9, 10, 8, 10, 8, 8, 8, 9, 8, 11, 8, 8, 8, 5, 5, 5, 6, 6, 4, 7, 7, 6, 7, 7, 6, 7,
        7, 4, 4, 7, 4, 10, 7, 6, 7, 7, 5, 6, 5, 7, 7, 9, 7, 7, 7, 6, 4, 6, 8,
    ];
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut pixels = 0u16;
    for byte in hint.bytes() {
        if !(32..=126).contains(&byte) {
            return Err("The exact destination contains characters the stick cannot display".into());
        }
        let width = u16::from(WIDTH[usize::from(byte - 32)]);
        if pixels + width > 100 {
            pieces.push(std::mem::take(&mut piece));
            pixels = 0;
        }
        piece.push(char::from(byte));
        pixels += width;
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    if pieces.len() > 7 {
        return Err("The complete destination will not fit on the stick; use a shorter destination".into());
    }
    Ok(pieces.join(" "))
}

/// Preparation cannot run a command. Approval and execution use the same captured action.
pub struct Prepared {
    action: commands::Action,
    pub arguments: Value,
    pub hint: String,
    reply: String,
    shell_output: bool,
}

impl Prepared {
    pub async fn resolve(st: &Arc<AppState>, tool: &str, args: &Map<String, Value>) -> Result<Self, String> {
        let s = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
        let (action, reply, shell_output) = match tool {
            "open_url" => {
                let url = normalized_url(s("url"))?;
                let host = url::Url::parse(&url).unwrap().host_str().unwrap().to_string();
                (commands::Action::Open(vec![url]), format!("opened {host}"), false)
            }
            "open_app" => {
                let name = s("name").trim().to_string();
                if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || " _.&+'()-".contains(c)) {
                    return Err("not an app name".into());
                }
                let apps = st.apps.clone();
                let target = tokio::task::spawn_blocking(move || apps.resolve(&name))
                    .await
                    .map_err(|_| "Couldn't resolve the app")?
                    .ok_or("No installed app matches that name")?;
                (commands::Action::Open(vec!["-a".into(), target.clone()]), format!("opened {target}"), false)
            }
            "calendar_join" => {
                let (url, reply) = crate::connections::calendar::resolve_join(st)?;
                (commands::Action::Open(vec![normalized_url(&url)?]), reply, false)
            }
            "quick_command" => {
                let phrase = s("phrase").split_whitespace().collect::<Vec<_>>().join(" ");
                let (command, slots) = st.commands.find(&phrase).ok_or("no quick command matches that phrase")?;
                let apps = st.apps.clone();
                let (action, reply) = tokio::task::spawn_blocking(move || {
                    let captured = Arc::new(Mutex::new(None));
                    let save = captured.clone();
                    let recorder: commands::Runner = Arc::new(move |action| {
                        *save.lock().unwrap() = Some(action.clone());
                        String::new()
                    });
                    let reply = commands::run(&command, &slots, true, &apps, &recorder);
                    let action = captured.lock().unwrap().take();
                    (action, reply)
                })
                .await
                .map_err(|_| "Couldn't resolve the quick command")?;
                let mut action = action.ok_or("that command doesn't apply here")?;
                // Capture resolved launch targets too; command settings may change during approval.
                if let commands::Action::Open(values) = &mut action {
                    if values.first().is_some_and(|v| v == "-a") {
                        let app = st.apps.resolve(&values[1]).ok_or("No installed app matches that name")?;
                        values[1] = app;
                        if let Some(url) = values.get_mut(2) {
                            *url = normalized_url(url)?;
                        }
                    } else if let Some(url) = values.first_mut() {
                        *url = normalized_url(url)?;
                    }
                }
                let shell = matches!(action, commands::Action::Shell(_));
                (action, reply.unwrap_or_default(), shell)
            }
            _ => return Err("Not a Mac-changing tool".into()),
        };
        let (arguments, hint) = match &action {
            commands::Action::Open(values) => (
                json!({"open": values}),
                values.iter().filter(|s| *s != "-a").cloned().collect::<Vec<_>>().join(" -> "),
            ),
            commands::Action::Shell(command) => (json!({"shell": command}), command.clone()),
        };
        // Existing firmware's prompt is bounded. Never approve a silently truncated destination.
        if hint.chars().count() > 160 || hint.chars().any(char::is_control) {
            return Err("The destination is too long to confirm safely on the stick".into());
        }
        display_hint(&hint)?;
        Ok(Self { action, arguments, hint, reply, shell_output })
    }

    pub fn run(self, runner: &commands::Runner) -> String {
        let output = runner(&self.action);
        if self.shell_output {
            let short: String = crate::talk::stick_text(output.trim()).chars().take(200).collect();
            if short.is_empty() { "Done.".into() } else { short }
        } else {
            self.reply
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_destinations_fit_without_losing_any_url_characters() {
        for hint in [
            "https://example.com/a?value=123#anchor",
            "https://meet.google.com/abc-defg-hij",
            "/Applications/Slack.app",
        ] {
            let display = display_hint(hint).unwrap();
            assert_eq!(display.replace(' ', ""), hint);
            assert!(display.split(' ').all(|word| word.len() <= 25));
        }
        assert!(display_hint(&"W".repeat(100)).is_err());
        assert!(display_hint("/Applications/日本.app").is_err());
        assert!(display_hint("https://example.com/\u{202e}moc.live").is_err());
    }

    #[tokio::test]
    async fn quick_command_and_app_resolution_freeze_the_real_action() {
        let t = crate::talk::tests::agent("policy-resolution", false).await;
        t.st.commands
            .save(
                json!({"commands":[{"name":"Docs","phrases":["docs"],"open_url":"https://example.com/a"}]})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        let prepared =
            Prepared::resolve(&t.st, "quick_command", json!({"phrase":"docs"}).as_object().unwrap()).await.unwrap();
        t.st.commands
            .save(
                json!({"commands":[{"name":"Docs","phrases":["docs"],"open_url":"https://example.com/b"}]})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(prepared.arguments, json!({"open":["https://example.com/a"]}));
        prepared.run(&t.st.runner());
        assert_eq!(*t.ran.lock().unwrap(), [commands::Action::Open(vec!["https://example.com/a".into()])]);
        let app = Prepared::resolve(&t.st, "open_app", json!({"name":"Slack"}).as_object().unwrap()).await.unwrap();
        assert_eq!(app.arguments, json!({"open":["-a","/Applications/Slack.app"]}));
        assert_eq!(app.hint, "/Applications/Slack.app");
    }

    #[test]
    fn profiles_default_and_invalid_config_fail_closed() {
        let defaults = config::defaults();
        for caller in [Caller::Mcp, Caller::Claude, Caller::OpenRouter, Caller::Internal] {
            assert_eq!(Profile::configured(&defaults, caller), Profile::Default);
            for tool in crate::mcp::tools() {
                assert!(Profile::Default.allows(tool["name"].as_str().unwrap()));
            }
        }
        for invalid in [json!(null), json!("assistant"), json!({"mcp":"typo"}), json!({"mcp":null})] {
            let cfg = json!({"tool_profiles":invalid}).as_object().unwrap().clone();
            assert_eq!(Profile::configured(&cfg, Caller::Mcp), Profile::Denied);
            assert!(!config::invalid(&cfg).is_empty());
        }
        for tool in ["quick_command", "ir_send", "future_mutator"] {
            assert!(!Profile::Assistant.allows(tool));
        }
        let openai = crate::mcp::openai_tools_for(Profile::Assistant);
        assert!(!openai.iter().any(|t| t["function"]["name"] == "quick_command" || t["function"]["name"] == "ir_send"));
    }
    #[test]
    fn url_normalization_rejects_credentials_and_non_web_schemes() {
        assert_eq!(normalized_url(" HTTPS://EXAMPLE.COM:443/a ").unwrap(), "https://example.com/a");
        for bad in ["file:///tmp/x", "https://user:password@example.com", "x"] {
            assert!(normalized_url(bad).is_err());
        }
        assert!(normalized_url("https://exa\nmple.com").is_err());
    }
    #[tokio::test]
    async fn internal_caller_is_enforced_and_shortened_destinations_fail_closed() {
        let t = crate::talk::tests::agent("policy-internal", false).await;
        t.st.cfg.lock().unwrap().insert("tool_profiles".into(), json!({"internal":"assistant"}));
        for tool in ["quick_command", "ir_send"] {
            assert!(crate::mcp::call(&t.st, tool, &json!({}), &Context::internal("test")).await.1);
        }
        let args = json!({"url":format!("https://example.com/{}","x".repeat(170))});
        assert!(Prepared::resolve(&t.st, "open_url", args.as_object().unwrap()).await.is_err());
        assert!(t.ran.lock().unwrap().is_empty());
    }
}
