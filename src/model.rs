use chrono::{DateTime, Utc};

use crate::verdict::{Thresholds, Verdict, absolute_verdict};

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

/// Aggregated health across a node and all its descendants (ADR-007).
/// Cost aggregation omitted: brim reads point-in-time window, not cumulative spend (ADR-002).
/// Per-metric rules preserved — metrics are NOT blended into a single scalar.
#[derive(Debug, Clone)]
pub struct SubtreeInfo {
    /// Saturating sum of window_tokens across the subtree.
    pub total_subtree_tokens: u64,
    /// Fill percent of the fullest node in the subtree.
    pub worst_fill_percent: u8,
    /// Node identifier (agent_id if sub-agent, else session_uuid) of the fullest node.
    pub worst_fill_node: String,
    /// Smallest projected_turns_to_overbound in the subtree (earliest deadline; worst growth).
    /// None when no node in the subtree has projection data.
    pub worst_projection: Option<u32>,
    /// Node owning worst_projection. Invariant: Some iff worst_projection is Some.
    pub worst_projection_node: Option<String>,
    /// Maximum velocity_tokens_per_turn across the subtree.
    pub max_velocity: Option<u64>,
    /// Worst verdict in the subtree (Over > Nearing > Ok).
    pub worst_verdict: Verdict,
    /// Node owning worst_verdict.
    pub worst_verdict_node: String,
}

/// Walk node + all descendants; return aggregated SubtreeInfo (ADR-007).
/// Recursion is bounded by the existing in-memory tree (caps enforced upstream per ADR-001/REQ-003).
/// Flat-provider sessions (no children) produce subtree == self with no special-casing.
/// Uses saturating arithmetic; no unwrap/expect.
pub fn compute_subtree(node: &SessionNode, thresholds: &Thresholds) -> SubtreeInfo {
    let node_id = node
        .agent_id
        .as_deref()
        .unwrap_or(&node.session_uuid)
        .to_string();

    let (self_tokens, self_fill, self_verdict) = if let Some(w) = &node.window {
        let projected_turns = node
            .trend
            .as_ref()
            .and_then(|t| t.projected_turns_to_overbound);
        let (v, _) = absolute_verdict(
            w.window_tokens,
            projected_turns,
            w.cache_hit_ratio,
            thresholds.watch_tokens,
            thresholds.recycle_backstop,
        );
        (w.window_tokens, w.fill_percent, v)
    } else {
        (0, 0, Verdict::Ok)
    };

    let self_projection = node
        .trend
        .as_ref()
        .and_then(|t| t.projected_turns_to_overbound);
    let self_velocity = node.trend.as_ref().and_then(|t| t.velocity_tokens_per_turn);

    let mut total_subtree_tokens = self_tokens;
    let mut worst_fill_percent = self_fill;
    let mut worst_fill_node = node_id.clone();
    let mut worst_projection = self_projection;
    let mut worst_projection_node = self_projection.map(|_| node_id.clone());
    let mut max_velocity = self_velocity;
    let mut worst_verdict = self_verdict;
    let mut worst_verdict_node = node_id;

    for child in &node.children {
        let ci = compute_subtree(child, thresholds);

        total_subtree_tokens = total_subtree_tokens.saturating_add(ci.total_subtree_tokens);

        if ci.worst_fill_percent > worst_fill_percent {
            worst_fill_percent = ci.worst_fill_percent;
            worst_fill_node = ci.worst_fill_node;
        }

        match (worst_projection, ci.worst_projection) {
            (None, Some(cv)) => {
                worst_projection = Some(cv);
                worst_projection_node = ci.worst_projection_node;
            }
            (Some(curr), Some(cv)) if cv < curr => {
                worst_projection = Some(cv);
                worst_projection_node = ci.worst_projection_node;
            }
            _ => {}
        }

        match (max_velocity, ci.max_velocity) {
            (None, Some(cv)) => max_velocity = Some(cv),
            (Some(curr), Some(cv)) if cv > curr => max_velocity = Some(cv),
            _ => {}
        }

        if ci.worst_verdict > worst_verdict {
            worst_verdict = ci.worst_verdict;
            worst_verdict_node = ci.worst_verdict_node;
        }
    }

    SubtreeInfo {
        total_subtree_tokens,
        worst_fill_percent,
        worst_fill_node,
        worst_projection,
        worst_projection_node,
        max_velocity,
        worst_verdict,
        worst_verdict_node,
    }
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
