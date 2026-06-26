use crate::model::{RecycleRecommendation, SessionNode, SubtreeInfo, compute_subtree};
use crate::parser::short_id;
use crate::verdict::{FamilyVoteInputs, Thresholds, Verdict, family_vote_verdict};
use crate::window::sustained_cache_thrash;
use chrono::{DateTime, Utc};
use serde::Serialize;

pub(crate) const PROJECT_COL_WIDTH: usize = 28;

#[derive(Serialize, Clone)]
pub(crate) struct JsonWindowTrend {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "velocity")]
    velocity_tokens_per_turn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "proj_turns")]
    projected_turns_to_recycle: Option<u32>,
}

/// Subtree aggregation over a node + all descendants (ADR-007).
/// Cost omitted: brim reads point-in-time window only, not cumulative spend (ADR-002).
#[derive(Serialize, Clone)]
pub(crate) struct JsonSubtreeInfo {
    #[serde(rename = "subtree_tokens")]
    total_subtree_tokens: u64,
    worst_tokens: u64,
    #[serde(rename = "worst_node")]
    worst_tokens_node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "worst_proj")]
    worst_projection: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "worst_proj_node")]
    worst_projection_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_velocity: Option<u64>,
    worst_verdict: &'static str,
    worst_verdict_node: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct JsonBlastRadiusEntry {
    #[serde(rename = "node")]
    node_id: String,
    active: bool,
}

/// Recycle recommendation in JSON output (ADR-009, REQ-005). Advisory only (ADR-010 §5).
#[derive(Serialize, Clone)]
pub(crate) struct JsonRecycleRecommendation {
    #[serde(rename = "target")]
    target_node_id: String,
    is_root: bool,
    target_verdict: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict_gate: Option<&'static str>,
    blast_radius: Vec<JsonBlastRadiusEntry>,
}

#[derive(Serialize, Clone)]
pub(crate) struct JsonFamilyVotes {
    pub volume: bool,
    pub speed: bool,
    pub thrash: bool,
    pub behavior: bool,
    pub drift: bool,
    pub count: u8,
}

#[derive(Serialize)]
pub(crate) struct JsonOutput {
    pub(crate) nodes: Vec<JsonNode>,
}

#[derive(Serialize, Clone)]
pub(crate) struct JsonNode {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_tokens: Option<u64>,
    /// Quality verdict: OR of ADR-010 absolute-budget, projection, and cache-thrash signals.
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<&'static str>,
    /// Which ADR-010 OR-gate fired (null when verdict is ok).
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict_gate: Option<&'static str>,
    /// Provenance of the reported window occupancy: "last_turn" or "aggregate" (REQ-005).
    #[serde(skip_serializing_if = "Option::is_none")]
    window_source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_turn_at: Option<String>,
    active: bool,
    /// Per-turn fill trajectory: velocity, projection, cache-hit ratio (ADR-006, ADR-008).
    /// ADR-013: trend.points (per-turn timeline array) is dropped from JSON output;
    /// only velocity + proj_turns are serialized.
    #[serde(skip_serializing_if = "Option::is_none")]
    trend: Option<JsonWindowTrend>,
    /// Subtree aggregation: self + all descendants (ADR-007).
    subtree: JsonSubtreeInfo,
    /// Recycle recommendation for this session tree (ADR-009). Null when subtree is healthy.
    /// Set for root nodes only; null for child nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    recycle_recommendation: Option<JsonRecycleRecommendation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    family_votes: Option<JsonFamilyVotes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decisive_override: Option<bool>,
    children: Vec<JsonNode>,
}

pub(crate) fn build_json_recycle_rec(rec: &RecycleRecommendation) -> JsonRecycleRecommendation {
    JsonRecycleRecommendation {
        target_node_id: rec.target_node_id.clone(),
        is_root: rec.is_root,
        target_verdict: rec.target_verdict.as_json_str(),
        verdict_gate: rec.verdict_gate.map(|g| g.as_json_str()),
        blast_radius: rec
            .blast_radius
            .iter()
            .map(|e| JsonBlastRadiusEntry {
                node_id: e.node_id.clone(),
                active: e.active,
            })
            .collect(),
    }
}

pub(crate) fn to_json_node(
    node: &SessionNode,
    si: &SubtreeInfo,
    parent_uuid: Option<&str>,
    thresholds: &Thresholds,
    active_mins: u32,
    rec: Option<JsonRecycleRecommendation>,
) -> JsonNode {
    let (
        window_tokens,
        model,
        verdict,
        verdict_gate,
        window_source,
        tier,
        family_votes,
        decisive_override,
    ) = if let Some(w) = node.window.as_ref() {
        let trend = node.trend.as_ref();
        let projected_turns = trend.and_then(|t| t.projected_turns_to_recycle);
        let thrash = trend.is_some_and(|t| sustained_cache_thrash(&t.points));
        let drift_score = trend.and_then(|t| t.drift_score);
        let inputs = FamilyVoteInputs {
            window_tokens: w.window_tokens,
            watch_tokens: thresholds.watch_tokens,
            projected_turns,
            sustained_cache_thrash: thrash,
            behavior: node.behavior.as_ref(),
            drift_score,
        };
        let result = family_vote_verdict(&inputs);
        let fv = Some(JsonFamilyVotes {
            volume: result.families[0],
            speed: result.families[1],
            thrash: result.families[2],
            behavior: result.families[3],
            drift: result.families[4],
            count: result.count,
        });
        let do_override = if result.decisive_override {
            Some(true)
        } else {
            None
        };
        (
            Some(w.window_tokens),
            Some(w.model.clone()),
            Some(result.verdict.as_json_str()),
            result.verdict_gate.map(|g| g.as_json_str()),
            Some(w.window_source.as_json_str()),
            Some(result.tier.as_json_str()),
            fv,
            do_override,
        )
    } else {
        (None, None, None, None, None, None, None, None)
    };

    // For sub-agents: session_id = agent_id (the sub-agent's own UUID).
    // For roots: session_id = session_uuid.
    let session_id = node
        .agent_id
        .as_deref()
        .unwrap_or(&node.session_uuid)
        .to_string();

    let trend = node.trend.as_ref().map(|t| JsonWindowTrend {
        velocity_tokens_per_turn: t.velocity_tokens_per_turn,
        projected_turns_to_recycle: t.projected_turns_to_recycle,
    });

    let subtree = JsonSubtreeInfo {
        total_subtree_tokens: si.total_subtree_tokens,
        worst_tokens: si.worst_tokens,
        worst_tokens_node: si.worst_tokens_node.clone(),
        worst_projection: si.worst_projection,
        worst_projection_node: si.worst_projection_node.clone(),
        max_velocity: si.max_velocity,
        worst_verdict: si.worst_verdict.as_json_str(),
        worst_verdict_node: si.worst_verdict_node.clone(),
    };

    JsonNode {
        session_id,
        parent_session_id: parent_uuid.map(|s| s.to_string()),
        agent_id: node.agent_id.clone(),
        project: node.project_key.clone(),
        model,
        window_tokens,
        verdict,
        verdict_gate,
        window_source,
        last_turn_at: node.last_turn_at.map(|ts| ts.to_rfc3339()),
        active: crate::is_active(node, active_mins),
        trend,
        subtree,
        recycle_recommendation: rec,
        tier,
        family_votes,
        decisive_override,
        children: node
            .children
            .iter()
            .map(|c| {
                let child_si = compute_subtree(c, thresholds);
                to_json_node(
                    c,
                    &child_si,
                    Some(&node.session_uuid),
                    thresholds,
                    active_mins,
                    None,
                )
            })
            .collect(),
    }
}

pub(crate) fn age_str(last_turn_at: Option<DateTime<Utc>>) -> String {
    let Some(ts) = last_turn_at else {
        return "-".to_string();
    };
    let secs = Utc::now().signed_duration_since(ts).num_seconds();
    if secs < 0 {
        return "0m".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        format!("{mins}m")
    } else if mins < 60 * 24 {
        format!("{}h", mins / 60)
    } else {
        let days = mins / (60 * 24);
        if days >= 1000 {
            ">999d".to_string()
        } else {
            format!("{}d", days)
        }
    }
}

/// Truncate to PROJECT_COL_WIDTH chars, appending '…' if clipped.
pub(crate) fn truncate_project(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= PROJECT_COL_WIDTH {
        s.to_string()
    } else {
        let truncated: String = chars.into_iter().take(PROJECT_COL_WIDTH - 1).collect();
        format!("{truncated}\u{2026}")
    }
}

pub(crate) fn tokens_str(node: &SessionNode) -> String {
    node.window
        .as_ref()
        .map(|w| w.window_tokens.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn verdict_str(node: &SessionNode, thresholds: &Thresholds) -> String {
    let Some(w) = node.window.as_ref() else {
        return "-".to_string();
    };
    let trend = node.trend.as_ref();
    let projected_turns = trend.and_then(|t| t.projected_turns_to_recycle);
    let thrash = trend.is_some_and(|t| sustained_cache_thrash(&t.points));
    let drift_score = trend.and_then(|t| t.drift_score);
    let inputs = FamilyVoteInputs {
        window_tokens: w.window_tokens,
        watch_tokens: thresholds.watch_tokens,
        projected_turns,
        sustained_cache_thrash: thrash,
        behavior: node.behavior.as_ref(),
        drift_score,
    };
    let result = family_vote_verdict(&inputs);
    match result.verdict_gate {
        Some(g) => format!("{} ({})", result.verdict.as_str(), g.as_str()),
        None => result.verdict.as_str().to_string(),
    }
}

pub(crate) fn model_str(node: &SessionNode) -> &str {
    node.window
        .as_ref()
        .and_then(|w| {
            if w.model.is_empty() {
                None
            } else {
                Some(w.model.as_str())
            }
        })
        .unwrap_or("-")
}

pub(crate) fn project_str(node: &SessionNode) -> &str {
    if node.project_key.is_empty() {
        "-"
    } else {
        &node.project_key
    }
}

/// Advisory recycle block (ADR-009, ADR-010 §5). Recommend, never imperative.
pub(crate) fn print_recycle_advisory(rec: &RecycleRecommendation) {
    let active_count = rec.blast_radius.iter().filter(|e| e.active).count();
    let gate_str = rec
        .verdict_gate
        .map(|g| format!(" via {}", g.as_str()))
        .unwrap_or_default();
    println!(
        "ADVISORY  candidate={}  verdict={}{}  blast={} desc ({} active orphans)",
        short_id(&rec.target_node_id),
        rec.target_verdict.as_str(),
        gate_str,
        rec.blast_radius.len(),
        active_count,
    );
    if rec.is_root {
        println!(
            "          note: target is ROOT — recycling this session restarts the whole operation"
        );
    }
    let active_ids: Vec<_> = rec
        .blast_radius
        .iter()
        .filter(|e| e.active)
        .map(|e| short_id(&e.node_id))
        .collect();
    if !active_ids.is_empty() {
        println!("          active orphans: {}", active_ids.join(" "));
    }
    println!();
}

/// Top-line glanceable subtree summary across all root sessions (ADR-007).
/// Self readings appear in the per-session table below; this line shows subtree aggregates.
pub(crate) fn print_subtree_summary(pairs: &[(SubtreeInfo, SessionNode)]) {
    if pairs.is_empty() {
        return;
    }
    let mut total_tokens: u64 = 0;
    let mut worst_tokens: u64 = 0;
    let mut worst_tokens_node = String::new();
    let mut worst_proj: Option<u32> = None;
    let mut worst_proj_node = String::new();
    let mut worst_verd = Verdict::Ok;
    let mut worst_verd_node = String::new();
    let mut initialized = false;

    for (si, _) in pairs {
        total_tokens = total_tokens.saturating_add(si.total_subtree_tokens);
        if !initialized || si.worst_tokens > worst_tokens {
            worst_tokens = si.worst_tokens;
            worst_tokens_node = short_id(&si.worst_tokens_node);
        }
        match (worst_proj, si.worst_projection) {
            (None, Some(v)) => {
                worst_proj = Some(v);
                worst_proj_node = si
                    .worst_projection_node
                    .as_deref()
                    .map(short_id)
                    .unwrap_or_default();
            }
            (Some(curr), Some(v)) if v < curr => {
                worst_proj = Some(v);
                worst_proj_node = si
                    .worst_projection_node
                    .as_deref()
                    .map(short_id)
                    .unwrap_or_default();
            }
            _ => {}
        }
        if !initialized || si.worst_verdict > worst_verd {
            worst_verd = si.worst_verdict;
            worst_verd_node = short_id(&si.worst_verdict_node);
        }
        initialized = true;
    }

    let proj_str = match worst_proj {
        Some(p) if !worst_proj_node.is_empty() => format!("{p} turns({worst_proj_node})"),
        Some(p) => format!("{p} turns"),
        None => "-".to_string(),
    };
    println!(
        "SUBTREE  all={total_tokens} tok  worst-tok={worst_tokens}({worst_tokens_node})  deadline={proj_str}  verdict={}({worst_verd_node})",
        worst_verd.as_str()
    );
    println!();
}

pub(crate) fn print_flat(sessions: &[SessionNode], thresholds: &Thresholds) {
    println!(
        "{:<16} {:>8}  {:<30}  {:<5}  {:<28} MODEL",
        "SESSION", "TOKENS", "VERDICT", "AGE", "PROJECT"
    );
    for s in sessions {
        let id = short_id(&s.session_uuid);
        let sub = if s.children.is_empty() {
            String::new()
        } else {
            format!("  [{} sub-agent(s)]", s.children.len())
        };
        println!(
            "{:<16} {:>8}  {:<30}  {:<5}  {:<28} {}{}",
            id,
            tokens_str(s),
            verdict_str(s, thresholds),
            age_str(s.last_turn_at),
            truncate_project(project_str(s)),
            model_str(s),
            sub,
        );
    }
}

pub(crate) fn print_tree(sessions: &[SessionNode], thresholds: &Thresholds) {
    for s in sessions {
        println!(
            "{:<16} {:>8}  {:<30}  {:<5}  {:<28} {}",
            short_id(&s.session_uuid),
            tokens_str(s),
            verdict_str(s, thresholds),
            age_str(s.last_turn_at),
            truncate_project(project_str(s)),
            model_str(s),
        );
        let last = s.children.len().saturating_sub(1);
        for (i, child) in s.children.iter().enumerate() {
            let conn = if i < last { "├──" } else { "└──" };
            let child_id = child
                .agent_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {} agent:{:<16} {:>8}  {:<30}  {:<5}  {:<28} {}",
                conn,
                child_id,
                tokens_str(child),
                verdict_str(child, thresholds),
                age_str(child.last_turn_at),
                "",
                model_str(child),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        SessionNode, TimelinePoint, WindowInfo, WindowSource, WindowTrend, compute_subtree,
        recycle_recommendation,
    };

    #[test]
    fn test_json_full_id_nested_children_no_transcript_content() {
        let thresholds = Thresholds::default();
        let parent_uuid = "aaaabbbb-cccc-dddd-eeee-111122223333";
        let child_agent_id = "ffff0000-1111-2222-3333-444455556666";

        let child = SessionNode {
            session_uuid: parent_uuid.to_string(),
            agent_id: Some(child_agent_id.to_string()),
            project_key: "test-project".to_string(),
            window: Some(WindowInfo {
                window_tokens: 10_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
            behavior: None,
        };

        let parent = SessionNode {
            session_uuid: parent_uuid.to_string(),
            agent_id: None,
            project_key: "test-project".to_string(),
            window: Some(WindowInfo {
                // 40k: above ABSOLUTE_WATCH_TOKENS (32k), below ABSOLUTE_RECYCLE_BACKSTOP (128k)
                window_tokens: 40_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: vec![child],
            last_turn_at: None,
            trend: None,
            behavior: None,
        };

        let parent_si = compute_subtree(&parent, &thresholds);
        let jnode = to_json_node(&parent, &parent_si, None, &thresholds, 30, None);

        // Full untruncated ID
        assert_eq!(jnode.session_id, parent_uuid);
        assert_eq!(jnode.parent_session_id, None);
        assert_eq!(jnode.agent_id, None);
        // 40k >= ABSOLUTE_WATCH_TOKENS → Nearing via abs-watch gate (ADR-010)
        assert_eq!(jnode.verdict, Some("nearing"));
        assert_eq!(jnode.verdict_gate, Some("absolute_watch"));
        assert_eq!(jnode.children.len(), 1);

        let child_j = &jnode.children[0];
        assert_eq!(child_j.session_id, child_agent_id);
        assert_eq!(child_j.parent_session_id, Some(parent_uuid.to_string()));
        assert_eq!(child_j.agent_id, Some(child_agent_id.to_string()));

        // No transcript content in serialized output
        let json_str = serde_json::to_string(&jnode).unwrap();
        assert!(!json_str.contains("message"));
        assert!(!json_str.contains("content"));
        assert!(!json_str.contains("prompt"));
    }

    #[test]
    fn test_json_verdict_over_recycle_string() {
        let thresholds = Thresholds::default();
        let node = SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: None,
            project_key: "test".to_string(),
            window: Some(WindowInfo {
                window_tokens: 190_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
            behavior: None,
        };
        let node_si = compute_subtree(&node, &thresholds);
        let jnode = to_json_node(&node, &node_si, None, &thresholds, 30, None);
        // 190k >= ABSOLUTE_WATCH_TOKENS (32k) → volume fires (1 family) → Nearing via abs-watch gate (ADR-025)
        assert_eq!(jnode.verdict, Some("nearing"));
        assert_eq!(jnode.verdict_gate, Some("absolute_watch"));
    }

    #[test]
    fn test_truncate_project_short_passthrough() {
        let short = "home-pol-code-brim";
        assert_eq!(truncate_project(short), short);
    }

    #[test]
    fn test_truncate_project_long_clips_to_width() {
        let long = "home-pol-code-very-long-project-name-here";
        let result = truncate_project(long);
        let char_count = result.chars().count();
        assert_eq!(char_count, PROJECT_COL_WIDTH);
        assert!(result.ends_with('\u{2026}'));
    }

    // M1: no-window node must skip model, window_tokens, verdict (ADR-013 skip-null).
    #[test]
    fn test_json_null_fields_when_no_window() {
        let thresholds = Thresholds::default();
        let node = SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: None,
            project_key: "test".to_string(),
            window: None,
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
            behavior: None,
        };
        let node_si = compute_subtree(&node, &thresholds);
        let jnode = to_json_node(&node, &node_si, None, &thresholds, 30, None);
        assert_eq!(jnode.model, None);
        assert_eq!(jnode.window_tokens, None);
        assert_eq!(jnode.verdict, None);
        let json_str = serde_json::to_string(&jnode).unwrap();
        // ADR-013: null Option fields are skipped, not emitted as null.
        assert!(!json_str.contains("\"model\""));
        assert!(!json_str.contains("\"verdict\""));
        assert!(!json_str.contains("\"window_tokens\""));
    }

    // Trend serialized in JSON output when present (ADR-013: no points).
    #[test]
    fn test_json_trend_serialized() {
        let thresholds = Thresholds::default();
        let at = Utc::now();
        let node = SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: None,
            project_key: "test".to_string(),
            window: Some(WindowInfo {
                window_tokens: 100_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: Some(0.5),
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: Some(WindowTrend {
                points: vec![TimelinePoint {
                    at,
                    window_tokens: 100_000,
                    cache_hit_ratio: Some(0.5),
                }],
                velocity_tokens_per_turn: Some(10_000),
                projected_turns_to_recycle: Some(10),
                drift_score: None,
            }),
            behavior: None,
        };
        let node_si = compute_subtree(&node, &thresholds);
        let jnode = to_json_node(&node, &node_si, None, &thresholds, 30, None);
        let t = jnode.trend.as_ref().expect("trend in json node");
        assert_eq!(t.velocity_tokens_per_turn, Some(10_000));
        assert_eq!(t.projected_turns_to_recycle, Some(10));
        let json_str = serde_json::to_string(&jnode).unwrap();
        // ADR-013: trend serializes velocity + proj_turns only; no points array.
        assert!(json_str.contains("\"trend\""));
        assert!(json_str.contains("\"velocity\""));
        assert!(json_str.contains("\"proj_turns\""));
        assert!(!json_str.contains("\"points\""));
        assert!(!json_str.contains("\"velocity_tokens_per_turn\""));
        // Top-level stability: generated_at dropped (ADR-013).
        assert!(!json_str.contains("\"generated_at\""));
    }

    // ADR-013 slim --json contract: full JsonOutput shape.
    #[test]
    fn test_json_slim_contract_adr013() {
        let thresholds = Thresholds::default();
        let parent_uuid = "aaaabbbb-cccc-dddd-eeee-111122223333";
        let child_agent = "ffff0000-1111-2222-3333-444455556666";

        // no-window leaf (null fields → skipped)
        let child = SessionNode {
            session_uuid: parent_uuid.to_string(),
            agent_id: Some(child_agent.to_string()),
            project_key: "p".to_string(),
            window: None,
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
            behavior: None,
        };
        // root with window + trend (so subtree aggregates and recycle rec populate)
        let parent = SessionNode {
            session_uuid: parent_uuid.to_string(),
            agent_id: None,
            project_key: "p".to_string(),
            window: Some(WindowInfo {
                window_tokens: 190_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: vec![child],
            last_turn_at: None,
            trend: None,
            behavior: None,
        };

        let parent_si = compute_subtree(&parent, &thresholds);
        let rec = recycle_recommendation(&parent, &thresholds, &|_| false)
            .map(|r| build_json_recycle_rec(&r));
        let jroot = to_json_node(&parent, &parent_si, None, &thresholds, 30, rec);
        let output = JsonOutput {
            nodes: vec![jroot.clone()],
        };
        let json_str = serde_json::to_string(&output).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json_str).expect("parses");

        // (i) top-level is an object with `nodes` array, no generated_at (ADR-013).
        assert!(v.get("generated_at").is_none(), "no generated_at");
        assert!(v.get("nodes").unwrap().is_array(), "nodes array present");

        // (ii) top-level JsonNode keys stay stable (ADR-013: top-level unchanged).
        let node = &v["nodes"][0];
        for key in ["session_id", "project", "active", "subtree", "children"] {
            assert!(node.get(key).is_some(), "root has stable key {key}");
        }
        // root window present → window_tokens/verdict/verdict_gate/window_source/model emitted.
        assert!(node.get("window_tokens").is_some());
        assert!(node.get("verdict").is_some());
        assert_eq!(node["verdict"], "nearing");
        assert_eq!(node["verdict_gate"], "absolute_watch");

        // (iii) nested keys are the short names (ADR-013 rename map).
        let sub = &node["subtree"];
        assert!(sub.get("subtree_tokens").is_some(), "subtree_tokens");
        assert!(sub.get("worst_tokens").is_some(), "worst_tokens");
        assert!(sub.get("worst_node").is_some(), "worst_node");
        assert!(sub.get("worst_verdict").is_some(), "worst_verdict");
        assert!(
            sub.get("worst_verdict_node").is_some(),
            "worst_verdict_node"
        );
        // worst_proj / worst_proj_node / max_velocity are None here → absent (skip-null).
        assert!(
            sub.get("worst_proj").is_none(),
            "worst_proj skipped when None"
        );
        assert!(
            sub.get("worst_proj_node").is_none(),
            "worst_proj_node skipped when None"
        );
        assert!(
            sub.get("max_velocity").is_none(),
            "max_velocity skipped when None"
        );
        // verbose names absent
        assert!(sub.get("total_subtree_tokens").is_none());
        assert!(sub.get("worst_tokens_node").is_none());
        assert!(sub.get("worst_projection").is_none());
        assert!(sub.get("worst_projection_node").is_none());

        // (iv) recycle_recommendation for the over root: target + blast.
        let rec_v = node
            .get("recycle_recommendation")
            .expect("rec emitted for unhealthy root");
        assert!(
            rec_v.get("target").is_some(),
            "target (renamed from target_node_id)"
        );
        assert!(
            rec_v.get("target_node_id").is_none(),
            "old target_node_id absent"
        );
        assert_eq!(rec_v["target"], parent_uuid);
        let blast = &rec_v["blast_radius"];
        assert!(blast.is_array());
        assert!(blast[0].get("node").is_some(), "blast node renamed");
        assert!(blast[0].get("node_id").is_none(), "old node_id absent");

        // (v) trend has no points key.
        assert!(
            node.get("trend").is_none() || node["trend"].get("points").is_none(),
            "no trend.points"
        );

        // (vi) null Option fields absent on the no-window child node.
        let child_v = &node["children"][0];
        for key in [
            "model",
            "window_tokens",
            "verdict",
            "verdict_gate",
            "window_source",
            "last_turn_at",
            "trend",
            "recycle_recommendation",
        ] {
            assert!(
                child_v.get(key).is_none(),
                "child node skips null field: {key}"
            );
        }
        // child session_id + agent_id + project + active + subtree + children always present.
        assert!(child_v.get("session_id").is_some());
        assert!(child_v.get("agent_id").is_some());
        assert!(child_v.get("project").is_some());
        assert!(child_v.get("active").is_some());
        assert!(child_v.get("subtree").is_some());

        // (vii) verdict in {ok,nearing,over_recycle}|null — already checked root verdict.
        assert!(matches!(
            node["verdict"].as_str(),
            Some("ok") | Some("nearing") | Some("over_recycle")
        ));

        // (viii) no transcript/prompt content.
        assert!(!json_str.contains("prompt"));
        assert!(!json_str.contains("content"));
    }

    // TEST-005: brim --json node contract matches absolute-tokens field set.
    #[test]
    fn test_json_contract_test005() {
        let thresholds = Thresholds::default();
        let root_uuid = "00000000-1111-2222-3333-444444444444";
        let child_agent = "ffffffff-eeee-dddd-cccc-bbbbbbbbbbbb";

        // no-window child — null fields must be absent (ADR-013 skip-null).
        let no_window_child = SessionNode {
            session_uuid: root_uuid.to_string(),
            agent_id: Some(child_agent.to_string()),
            project_key: "myproject".to_string(),
            window: None,
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
            behavior: None,
        };
        // active root with known window_tokens.
        let root = SessionNode {
            session_uuid: root_uuid.to_string(),
            agent_id: None,
            project_key: "myproject".to_string(),
            window: Some(WindowInfo {
                window_tokens: 50_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: vec![no_window_child],
            last_turn_at: None,
            trend: None,
            behavior: None,
        };

        let si = compute_subtree(&root, &thresholds);
        let jroot = to_json_node(&root, &si, None, &thresholds, 30, None);
        let output = JsonOutput { nodes: vec![jroot] };
        let json_str = serde_json::to_string(&output).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json_str).expect("valid json");

        // (1) top-level: object with `nodes` array, no `generated_at` (ADR-013).
        assert!(v.is_object());
        assert!(v["nodes"].is_array());
        assert!(v.get("generated_at").is_none(), "generated_at absent");

        let root_node = &v["nodes"][0];

        // (2) stable top-level keys present on active windowed node (REQ-005, ADR-012).
        for key in [
            "session_id",
            "project",
            "model",
            "window_tokens",
            "verdict",
            "window_source",
            "active",
            "subtree",
            "children",
        ] {
            assert!(root_node.get(key).is_some(), "root missing key: {key}");
        }
        // root has no parent → parent_session_id absent.
        assert!(root_node.get("parent_session_id").is_none());

        // (3) no limit / fill_percent anywhere (ADR-011 / ADR-012).
        assert!(!json_str.contains("\"limit\""), "no limit field");
        assert!(!json_str.contains("fill_percent"), "no fill_percent field");

        // (4) verdict is one of ok / nearing / over_recycle (ADR-012).
        let verdict = root_node["verdict"].as_str().unwrap();
        assert!(
            matches!(verdict, "ok" | "nearing" | "over_recycle"),
            "verdict={verdict}"
        );

        // (5) subtree uses short names; verbose names absent (ADR-013).
        let sub = &root_node["subtree"];
        for key in [
            "subtree_tokens",
            "worst_tokens",
            "worst_node",
            "worst_verdict",
            "worst_verdict_node",
        ] {
            assert!(sub.get(key).is_some(), "subtree missing key: {key}");
        }
        for key in [
            "total_subtree_tokens",
            "worst_tokens_node",
            "worst_projection",
            "worst_projection_node",
        ] {
            assert!(sub.get(key).is_none(), "verbose subtree key present: {key}");
        }

        // (6) child carries parent_session_id matching root's session_id (REQ-003).
        let child_node = &root_node["children"][0];
        let root_session_id = root_node["session_id"].as_str().unwrap();
        assert_eq!(
            child_node
                .get("parent_session_id")
                .expect("child carries parent_session_id")
                .as_str()
                .unwrap(),
            root_session_id,
        );
        assert_eq!(child_node["agent_id"].as_str().unwrap(), child_agent);

        // no-window child: nullable fields absent (ADR-013 skip-null).
        for key in [
            "model",
            "window_tokens",
            "verdict",
            "verdict_gate",
            "window_source",
            "last_turn_at",
            "trend",
            "recycle_recommendation",
        ] {
            assert!(
                child_node.get(key).is_none(),
                "null field present on no-window child: {key}"
            );
        }
        // always-present child fields.
        for key in ["session_id", "project", "active", "subtree", "children"] {
            assert!(child_node.get(key).is_some(), "child missing key: {key}");
        }

        // (7) no transcript / prompt content (CODERULES r11).
        assert!(!json_str.contains("prompt"), "no prompt in output");
        assert!(!json_str.contains("content"), "no content in output");
    }
}
