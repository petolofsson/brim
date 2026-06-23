use chrono::{DateTime, Utc};

/// Provenance of the reported window occupancy (REQ-005 machine-readable).
/// `LastTurn` = computed from the latest point-in-time usage record.
/// `Aggregate` = fell back to session-aggregate columns (no point-in-time record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSource {
    LastTurn,
    Aggregate,
}

impl WindowSource {
    pub fn as_json_str(self) -> &'static str {
        match self {
            WindowSource::LastTurn => "last_turn",
            WindowSource::Aggregate => "aggregate",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub window_tokens: u64,
    pub fill_percent: u8,
    pub model: String,
    pub context_limit: u64,
    pub window_source: WindowSource,
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
