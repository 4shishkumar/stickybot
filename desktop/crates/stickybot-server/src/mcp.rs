//! The stick tools: an MCP server inside the app, so any Claude session can page you on the stick
//! (a port of `mcp_server.py`).

/// The server's name: Claude Code calls the tools `mcp__stick__<tool>`.
pub const SERVER_NAME: &str = "stick";

/// The tools that only give feedback (pre-allowed); `open_url`, `open_app`, `quick_command` change
/// the Mac, so Claude Code asks first.
pub const FEEDBACK_TOOLS: &[&str] = &["notify", "ask", "celebrate", "progress", "pin_home", "stick_status", "speak"];
pub const MAC_TOOLS: &[&str] = &["open_url", "open_app", "quick_command"];

/// The feedback tools as Claude Code names them (for `--allowedTools`).
pub fn allowed_tool_names() -> Vec<String> {
    FEEDBACK_TOOLS.iter().map(|t| format!("mcp__{SERVER_NAME}__{t}")).collect()
}
