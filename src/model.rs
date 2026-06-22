use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub window_tokens: u64,
    pub fill_percent: u8,
    pub model: String,
    pub context_limit: u64,
}

#[derive(Debug, Clone)]
pub struct SessionNode {
    /// Parent sessions: the session UUID. Sub-agents: the parent UUID (join key).
    pub session_uuid: String,
    /// Sub-agent identifier extracted from filename. None for parent sessions.
    pub agent_id: Option<String>,
    /// Encoded project directory name (leading '-' stripped) from ~/.claude/projects/<key>/.
    pub project_key: String,
    pub window: Option<WindowInfo>,
    pub children: Vec<SessionNode>,
    /// Timestamp of the latest assistant turn used for the window computation.
    pub last_turn_at: Option<DateTime<Utc>>,
}
