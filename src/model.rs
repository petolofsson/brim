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
    /// cache_read / window_tokens, bounded [0,1]. None when no cache split (ADR-008).
    pub cache_hit_ratio: Option<f32>,
}

/// One point in a session's fill trajectory (ADR-006, REQ-007).
#[derive(Debug, Clone)]
pub struct TimelinePoint {
    pub at: DateTime<Utc>,
    pub window_tokens: u64,
    pub fill_percent: u8,
    pub cache_hit_ratio: Option<f32>,
}

/// Growth trend derived from a bounded tail read of the last K assistant turns (ADR-006).
#[derive(Debug, Clone)]
pub struct WindowTrend {
    pub points: Vec<TimelinePoint>,
    pub velocity_tokens_per_turn: Option<u64>,
    pub projected_turns_to_overbound: Option<u32>,
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
    /// Velocity/projection trend from the last K turns (ADR-006). None if unavailable.
    pub trend: Option<WindowTrend>,
}
