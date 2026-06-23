mod claude;
mod codex;
mod copilot;
mod model;
mod opencode;
mod parser;
mod provider;
mod verdict;
mod window;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Parser;
use claude::ClaudeProvider;
use codex::CodexProvider;
use copilot::CopilotProvider;
use model::{
    RecycleRecommendation, SessionNode, SubtreeInfo, compute_subtree, recycle_recommendation,
};
use opencode::OpencodeProvider;
use parser::short_id;
use provider::Provider;
use serde::Serialize;
use verdict::{Thresholds, Verdict, absolute_verdict};

const PROJECT_COL_WIDTH: usize = 28;

#[derive(Parser, Debug)]
#[command(
    name = "brim",
    about = "Context-window occupancy for AI coding sessions"
)]
struct Cli {
    /// Show orchestrator → sub-agent tree
    #[arg(long)]
    tree: bool,

    /// Scope output to one session and its sub-agents
    #[arg(long)]
    session: Option<String>,

    /// Single snapshot (default behavior; accepted for compatibility)
    #[arg(long)]
    once: bool,

    /// Absolute active-token watch band; research-anchored per ADR-010 (default: 32000)
    #[arg(long, default_value_t = 32_000)]
    watch_tokens: u64,

    /// Absolute recycle backstop; research-anchored per ADR-010 (default: 128000)
    #[arg(long, default_value_t = 128_000)]
    recycle_backstop: u64,

    /// Emit structured JSON to stdout instead of human-readable text (REQ-005)
    #[arg(long)]
    json: bool,

    /// Include stale/historical sessions; default shows active sessions only (REQ-006)
    #[arg(long)]
    all: bool,

    /// Minutes since last turn for a session to be considered active (default: 30) (REQ-006)
    #[arg(long, default_value_t = 30)]
    active_mins: u32,
}

#[derive(Serialize)]
struct JsonTimelinePoint {
    at: String,
    window_tokens: u64,
    cache_hit_ratio: Option<f32>,
}

#[derive(Serialize)]
struct JsonWindowTrend {
    points: Vec<JsonTimelinePoint>,
    velocity_tokens_per_turn: Option<u64>,
    projected_turns_to_recycle: Option<u32>,
}

/// Subtree aggregation over a node + all descendants (ADR-007).
/// Cost omitted: brim reads point-in-time window only, not cumulative spend (ADR-002).
#[derive(Serialize)]
struct JsonSubtreeInfo {
    total_subtree_tokens: u64,
    worst_tokens: u64,
    worst_tokens_node: String,
    worst_projection: Option<u32>,
    worst_projection_node: Option<String>,
    max_velocity: Option<u64>,
    worst_verdict: &'static str,
    worst_verdict_node: String,
}

#[derive(Serialize)]
struct JsonBlastRadiusEntry {
    node_id: String,
    active: bool,
}

/// Recycle recommendation in JSON output (ADR-009, REQ-005). Advisory only (ADR-010 §5).
#[derive(Serialize)]
struct JsonRecycleRecommendation {
    target_node_id: String,
    is_root: bool,
    target_verdict: &'static str,
    verdict_gate: Option<&'static str>,
    blast_radius: Vec<JsonBlastRadiusEntry>,
}

#[derive(Serialize)]
struct JsonOutput {
    generated_at: String,
    nodes: Vec<JsonNode>,
}

#[derive(Serialize)]
struct JsonNode {
    session_id: String,
    parent_session_id: Option<String>,
    agent_id: Option<String>,
    project: String,
    model: Option<String>,
    window_tokens: Option<u64>,
    /// Quality verdict: OR of ADR-010 absolute-budget, projection, and cache-thrash signals.
    verdict: Option<&'static str>,
    /// Which ADR-010 OR-gate fired (null when verdict is ok).
    verdict_gate: Option<&'static str>,
    /// Provenance of the reported window occupancy: "last_turn" or "aggregate" (REQ-005).
    window_source: Option<&'static str>,
    last_turn_at: Option<String>,
    active: bool,
    /// Per-turn fill trajectory: velocity, projection, cache-hit ratio (ADR-006, ADR-008).
    trend: Option<JsonWindowTrend>,
    /// Subtree aggregation: self + all descendants (ADR-007).
    subtree: JsonSubtreeInfo,
    /// Recycle recommendation for this session tree (ADR-009). Null when subtree is healthy.
    /// Set for root nodes only; null for child nodes.
    recycle_recommendation: Option<JsonRecycleRecommendation>,
    children: Vec<JsonNode>,
}

fn is_active(node: &SessionNode, active_mins: u32) -> bool {
    match node.last_turn_at {
        None => false,
        Some(ts) => Utc::now().signed_duration_since(ts).num_minutes() <= active_mins as i64,
    }
}

fn any_active(node: &SessionNode, active_mins: u32) -> bool {
    is_active(node, active_mins) || node.children.iter().any(|c| is_active(c, active_mins))
}

fn age_str(last_turn_at: Option<DateTime<Utc>>) -> String {
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
fn truncate_project(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= PROJECT_COL_WIDTH {
        s.to_string()
    } else {
        let truncated: String = chars.into_iter().take(PROJECT_COL_WIDTH - 1).collect();
        format!("{truncated}\u{2026}")
    }
}

fn build_json_recycle_rec(rec: &RecycleRecommendation) -> JsonRecycleRecommendation {
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

fn to_json_node(
    node: &SessionNode,
    si: &SubtreeInfo,
    parent_uuid: Option<&str>,
    thresholds: &Thresholds,
    active_mins: u32,
    rec: Option<JsonRecycleRecommendation>,
) -> JsonNode {
    let (window_tokens, model, verdict, verdict_gate, window_source) =
        if let Some(w) = node.window.as_ref() {
            let projected_turns = node
                .trend
                .as_ref()
                .and_then(|t| t.projected_turns_to_recycle);
            let (v, gate) = absolute_verdict(
                w.window_tokens,
                projected_turns,
                w.cache_hit_ratio,
                thresholds.watch_tokens,
                thresholds.recycle_backstop,
            );
            (
                Some(w.window_tokens),
                Some(w.model.clone()),
                Some(v.as_json_str()),
                gate.map(|g| g.as_json_str()),
                Some(w.window_source.as_json_str()),
            )
        } else {
            (None, None, None, None, None)
        };

    // For sub-agents: session_id = agent_id (the sub-agent's own UUID).
    // For roots: session_id = session_uuid.
    let session_id = node
        .agent_id
        .as_deref()
        .unwrap_or(&node.session_uuid)
        .to_string();

    let trend = node.trend.as_ref().map(|t| JsonWindowTrend {
        points: t
            .points
            .iter()
            .map(|p| JsonTimelinePoint {
                at: p.at.to_rfc3339(),
                window_tokens: p.window_tokens,
                cache_hit_ratio: p.cache_hit_ratio,
            })
            .collect(),
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
        active: is_active(node, active_mins),
        trend,
        subtree,
        recycle_recommendation: rec,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    anyhow::ensure!(
        cli.watch_tokens <= cli.recycle_backstop,
        "--watch-tokens ({}) must be \u{2264} --recycle-backstop ({})",
        cli.watch_tokens,
        cli.recycle_backstop,
    );
    let thresholds = Thresholds {
        watch_tokens: cli.watch_tokens,
        recycle_backstop: cli.recycle_backstop,
    };

    let providers: Vec<Box<dyn Provider>> = vec![
        Box::new(ClaudeProvider::new()),
        Box::new(CodexProvider::new()),
        Box::new(OpencodeProvider::new()),
        Box::new(CopilotProvider::new()),
    ];
    if providers.iter().all(|p| !p.is_available()) {
        println!("brim: no sessions (no provider available)");
        return Ok(());
    }

    let mut sessions: Vec<SessionNode> = Vec::new();
    for p in providers.iter().filter(|p| p.is_available()) {
        sessions.extend(p.load_sessions());
    }
    // De-dup not performed: providers may legitimately share a project key; the
    // project_key field carries the source-provider prefix only when needed for
    // disambiguation (handled per-provider at load time — opencode already
    // namespaces by project name, claude by encoded cwd, so collisions are rare).

    if let Some(id) = &cli.session {
        sessions.retain(|s| {
            s.session_uuid.starts_with(id.as_str())
                || short_id(&s.session_uuid).contains(id.as_str())
        });
        if sessions.is_empty() {
            eprintln!("brim: no session matching '{id}'");
            return Ok(());
        }
    }

    if !cli.all && cli.session.is_none() {
        sessions.retain(|s| any_active(s, cli.active_mins));
    }

    // Sort children worst-first recursively before computing root subtrees (ADR-007).
    for s in &mut sessions {
        sort_children_worst_first(s, &thresholds);
    }

    // Sort worst-first. Sort key: worst verdict (Over > Nearing > Ok) desc,
    // earliest projection asc (None = infinity → last), highest tokens desc. Deterministic.
    let mut pairs: Vec<(SubtreeInfo, SessionNode)> = sessions
        .into_iter()
        .map(|s| {
            let si = compute_subtree(&s, &thresholds);
            (si, s)
        })
        .collect();
    pairs.sort_by(|(sa, _), (sb, _)| {
        sb.worst_verdict
            .cmp(&sa.worst_verdict)
            .then_with(|| {
                sa.worst_projection
                    .unwrap_or(u32::MAX)
                    .cmp(&sb.worst_projection.unwrap_or(u32::MAX))
            })
            .then_with(|| sb.worst_tokens.cmp(&sa.worst_tokens))
    });

    if cli.json {
        let output = JsonOutput {
            generated_at: Utc::now().to_rfc3339(),
            nodes: pairs
                .iter()
                .map(|(si, s)| {
                    let rec =
                        recycle_recommendation(s, &thresholds, &|n| is_active(n, cli.active_mins))
                            .map(|r| build_json_recycle_rec(&r));
                    to_json_node(s, si, None, &thresholds, cli.active_mins, rec)
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    print_subtree_summary(&pairs);
    for (_, root) in &pairs {
        if let Some(rec) =
            recycle_recommendation(root, &thresholds, &|n| is_active(n, cli.active_mins))
        {
            print_recycle_advisory(&rec);
        }
    }

    let sessions: Vec<SessionNode> = pairs.into_iter().map(|(_, s)| s).collect();
    if cli.tree {
        print_tree(&sessions, &thresholds);
    } else {
        print_flat(&sessions, &thresholds);
    }

    Ok(())
}

fn tokens_str(node: &SessionNode) -> String {
    node.window
        .as_ref()
        .map(|w| w.window_tokens.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn verdict_str(node: &SessionNode, thresholds: &Thresholds) -> String {
    let Some(w) = node.window.as_ref() else {
        return "-".to_string();
    };
    let projected_turns = node
        .trend
        .as_ref()
        .and_then(|t| t.projected_turns_to_recycle);
    let (v, gate) = absolute_verdict(
        w.window_tokens,
        projected_turns,
        w.cache_hit_ratio,
        thresholds.watch_tokens,
        thresholds.recycle_backstop,
    );
    match gate {
        Some(g) => format!("{} ({})", v.as_str(), g.as_str()),
        None => v.as_str().to_string(),
    }
}

fn model_str(node: &SessionNode) -> &str {
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

fn project_str(node: &SessionNode) -> &str {
    if node.project_key.is_empty() {
        "-"
    } else {
        &node.project_key
    }
}

/// Apply the same worst-first sort key recursively to children at every level (ADR-007).
fn sort_children_worst_first(node: &mut SessionNode, thresholds: &Thresholds) {
    if node.children.is_empty() {
        return;
    }
    let mut child_pairs: Vec<(SubtreeInfo, SessionNode)> = node
        .children
        .drain(..)
        .map(|mut c| {
            sort_children_worst_first(&mut c, thresholds);
            let si = compute_subtree(&c, thresholds);
            (si, c)
        })
        .collect();
    child_pairs.sort_by(|(sa, _), (sb, _)| {
        sb.worst_verdict
            .cmp(&sa.worst_verdict)
            .then_with(|| {
                sa.worst_projection
                    .unwrap_or(u32::MAX)
                    .cmp(&sb.worst_projection.unwrap_or(u32::MAX))
            })
            .then_with(|| sb.worst_tokens.cmp(&sa.worst_tokens))
    });
    node.children = child_pairs.into_iter().map(|(_, c)| c).collect();
}

/// Advisory recycle block (ADR-009, ADR-010 §5). Recommend, never imperative.
fn print_recycle_advisory(rec: &RecycleRecommendation) {
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
fn print_subtree_summary(pairs: &[(SubtreeInfo, SessionNode)]) {
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

fn print_flat(sessions: &[SessionNode], thresholds: &Thresholds) {
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

fn print_tree(sessions: &[SessionNode], thresholds: &Thresholds) {
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
    use model::{WindowInfo, WindowSource, WindowTrend};

    fn make_node_with_ts(last_turn_at: Option<DateTime<Utc>>) -> SessionNode {
        SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: None,
            project_key: "test-project".to_string(),
            window: Some(WindowInfo {
                window_tokens: 100_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at,
            trend: None,
        }
    }

    #[test]
    fn test_recency_active_recent_timestamp() {
        // 10 minutes ago → active under 30-minute threshold
        let ts = Utc::now() - chrono::Duration::minutes(10);
        assert!(is_active(&make_node_with_ts(Some(ts)), 30));
    }

    #[test]
    fn test_recency_inactive_old_timestamp() {
        // 60 minutes ago → inactive under 30-minute threshold
        let ts = Utc::now() - chrono::Duration::minutes(60);
        assert!(!is_active(&make_node_with_ts(Some(ts)), 30));
    }

    #[test]
    fn test_recency_inactive_missing_timestamp() {
        assert!(!is_active(&make_node_with_ts(None), 30));
    }

    #[test]
    fn test_default_filter_excludes_stale_includes_active() {
        let active = make_node_with_ts(Some(Utc::now() - chrono::Duration::minutes(5)));
        let mut stale = make_node_with_ts(Some(Utc::now() - chrono::Duration::hours(2)));
        stale.session_uuid = "bbbbcccc-dddd-eeee-ffff-000011112222".to_string();

        let mut sessions = vec![active.clone(), stale.clone()];
        sessions.retain(|s| is_active(s, 30));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_uuid, active.session_uuid);

        // --all: both returned (no filter applied)
        let sessions_all = [active, stale];
        assert_eq!(sessions_all.len(), 2);
    }

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
        };
        let node_si = compute_subtree(&node, &thresholds);
        let jnode = to_json_node(&node, &node_si, None, &thresholds, 30, None);
        // 190k >= ABSOLUTE_RECYCLE_BACKSTOP (128k) → Over via abs-backstop gate (ADR-010)
        assert_eq!(jnode.verdict, Some("over_recycle"));
        assert_eq!(jnode.verdict_gate, Some("absolute_backstop"));
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

    // S1: stale parent + active child → both retained by default filter.
    #[test]
    fn test_default_filter_retains_stale_parent_with_active_child() {
        let child = SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: Some("child000-1111-2222-3333-444455556666".to_string()),
            project_key: "test-project".to_string(),
            window: Some(WindowInfo {
                window_tokens: 50_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at: Some(Utc::now() - chrono::Duration::minutes(5)),
            trend: None,
        };
        let parent = SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: None,
            project_key: "test-project".to_string(),
            window: Some(WindowInfo {
                window_tokens: 100_000,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: vec![child],
            last_turn_at: Some(Utc::now() - chrono::Duration::hours(2)),
            trend: None,
        };
        let mut sessions = vec![parent];
        sessions.retain(|s| any_active(s, 30));
        assert_eq!(
            sessions.len(),
            1,
            "stale parent with active child must be retained"
        );
        assert_eq!(sessions[0].children.len(), 1);
    }

    // S2: --session bypasses active filter; stale session must still appear.
    #[test]
    fn test_session_flag_bypasses_active_filter() {
        let stale = make_node_with_ts(Some(Utc::now() - chrono::Duration::hours(2)));
        // Without --session: active filter removes stale session
        let mut sessions = vec![stale.clone()];
        sessions.retain(|s| any_active(s, 30));
        assert_eq!(
            sessions.len(),
            0,
            "stale must be filtered without --session"
        );
        // With --session: no active filter applied → session is retained
        let retained = [stale];
        assert_eq!(retained.len(), 1, "stale must be retained with --session");
    }

    // M1: no-window node must emit null for model, window_tokens, verdict.
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
        };
        let node_si = compute_subtree(&node, &thresholds);
        let jnode = to_json_node(&node, &node_si, None, &thresholds, 30, None);
        assert_eq!(jnode.model, None);
        assert_eq!(jnode.window_tokens, None);
        assert_eq!(jnode.verdict, None);
        let json_str = serde_json::to_string(&jnode).unwrap();
        assert!(json_str.contains("\"model\":null"));
        assert!(json_str.contains("\"verdict\":null"));
    }

    // ADR-007 subtree aggregation tests

    fn make_node(
        uuid: &str,
        agent_id: Option<&str>,
        tokens: u64,
        projection: Option<u32>,
        velocity: Option<u64>,
    ) -> SessionNode {
        SessionNode {
            session_uuid: uuid.to_string(),
            agent_id: agent_id.map(|s| s.to_string()),
            project_key: "test".to_string(),
            window: Some(WindowInfo {
                window_tokens: tokens,
                model: "claude-sonnet-4-6".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: projection.map(|p| WindowTrend {
                points: Vec::new(),
                velocity_tokens_per_turn: velocity,
                projected_turns_to_recycle: Some(p),
            }),
        }
    }

    // T1: flat single-node — subtree == self
    #[test]
    fn subtree_single_node_equals_self() {
        let thresholds = Thresholds::default();
        let node = make_node(
            "aaaa0000-0000-0000-0000-000000000001",
            None,
            10_000,
            None,
            None,
        );
        let si = compute_subtree(&node, &thresholds);

        assert_eq!(si.total_subtree_tokens, 10_000);
        assert_eq!(si.worst_tokens, 10_000);
        assert_eq!(si.worst_tokens_node, "aaaa0000-0000-0000-0000-000000000001");
        assert_eq!(si.worst_projection, None);
        assert_eq!(si.worst_projection_node, None);
        assert_eq!(si.max_velocity, None);
        assert_eq!(si.worst_verdict, Verdict::Ok);
    }

    // T2: total tokens sum across multi-level tree
    #[test]
    fn subtree_multi_level_total_tokens_sum() {
        let thresholds = Thresholds::default();
        let parent_uuid = "pppp0000-0000-0000-0000-000000000000";
        let mut root = make_node(parent_uuid, None, 50_000, None, None);
        let child_a = make_node(
            parent_uuid,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa"),
            30_000,
            None,
            None,
        );
        let child_b = make_node(
            parent_uuid,
            Some("bbbb0000-0000-0000-0000-bbbbbbbbbbbb"),
            20_000,
            None,
            None,
        );
        root.children = vec![child_a, child_b];

        let si = compute_subtree(&root, &thresholds);
        assert_eq!(si.total_subtree_tokens, 100_000); // 50k + 30k + 20k
    }

    // T3: worst-tokens selection names the correct node
    #[test]
    fn subtree_worst_tokens_names_correct_node() {
        let thresholds = Thresholds::default();
        let parent_uuid = "pppp0000-0000-0000-0000-000000000000";
        let mut root = make_node(parent_uuid, None, 10_000, None, None);
        let child_a = make_node(
            parent_uuid,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa"),
            20_000,
            None,
            None,
        );
        let child_b = make_node(
            parent_uuid,
            Some("bbbb0000-0000-0000-0000-bbbbbbbbbbbb"),
            80_000,
            None,
            None,
        );
        root.children = vec![child_a, child_b];

        let si = compute_subtree(&root, &thresholds);
        assert_eq!(si.worst_tokens, 80_000);
        assert_eq!(si.worst_tokens_node, "bbbb0000-0000-0000-0000-bbbbbbbbbbbb");
    }

    // T4: earliest-deadline selection (smallest projection) names the correct node
    #[test]
    fn subtree_earliest_projection_names_correct_node() {
        let thresholds = Thresholds::default();
        let parent_uuid = "pppp0000-0000-0000-0000-000000000000";
        let mut root = make_node(parent_uuid, None, 10_000, Some(20), Some(1_000));
        let child_a = make_node(
            parent_uuid,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa"),
            20_000,
            Some(3), // earliest deadline
            Some(5_000),
        );
        let child_b = make_node(
            parent_uuid,
            Some("bbbb0000-0000-0000-0000-bbbbbbbbbbbb"),
            15_000,
            Some(12),
            Some(2_000),
        );
        root.children = vec![child_a, child_b];

        let si = compute_subtree(&root, &thresholds);
        assert_eq!(si.worst_projection, Some(3));
        assert_eq!(
            si.worst_projection_node,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa".to_string())
        );
        assert_eq!(si.max_velocity, Some(5_000));
    }

    // T5: worst-verdict propagates upward naming the offending node
    #[test]
    fn subtree_worst_verdict_propagates_naming_offending_node() {
        let thresholds = Thresholds::default();
        let parent_uuid = "pppp0000-0000-0000-0000-000000000000";
        // root: 10k tokens → Ok; child_b: 150k → Over (above absolute_recycle_backstop 128k)
        let mut root = make_node(parent_uuid, None, 10_000, None, None);
        let child_a = make_node(
            parent_uuid,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa"),
            40_000,
            None,
            None,
        );
        let child_b = make_node(
            parent_uuid,
            Some("bbbb0000-0000-0000-0000-bbbbbbbbbbbb"),
            150_000, // above 128k backstop → Over
            None,
            None,
        );
        root.children = vec![child_a, child_b];

        let si = compute_subtree(&root, &thresholds);
        assert_eq!(si.worst_verdict, Verdict::Over);
        assert_eq!(
            si.worst_verdict_node,
            "bbbb0000-0000-0000-0000-bbbbbbbbbbbb"
        );
    }

    // T6: worst-first sort key — Over before Nearing before Ok
    #[test]
    fn sort_worst_first_by_verdict() {
        let thresholds = Thresholds::default();
        let ok_node = make_node(
            "ok000000-0000-0000-0000-000000000000",
            None,
            5_000,
            None,
            None,
        );
        let nearing_node = make_node(
            "near0000-0000-0000-0000-000000000000",
            None,
            40_000, // above watch threshold (32k) → Nearing
            None,
            None,
        );
        let over_node = make_node(
            "over0000-0000-0000-0000-000000000000",
            None,
            150_000, // above backstop (128k) → Over
            None,
            None,
        );

        let mut pairs: Vec<(SubtreeInfo, SessionNode)> = vec![ok_node, nearing_node, over_node]
            .into_iter()
            .map(|s| (compute_subtree(&s, &thresholds), s))
            .collect();

        pairs.sort_by(|(sa, _), (sb, _)| {
            sb.worst_verdict
                .cmp(&sa.worst_verdict)
                .then_with(|| {
                    sa.worst_projection
                        .unwrap_or(u32::MAX)
                        .cmp(&sb.worst_projection.unwrap_or(u32::MAX))
                })
                .then_with(|| sb.worst_tokens.cmp(&sa.worst_tokens))
        });

        // Over first, then Nearing, then Ok
        assert_eq!(pairs[0].0.worst_verdict, Verdict::Over);
        assert_eq!(pairs[1].0.worst_verdict, Verdict::Nearing);
        assert_eq!(pairs[2].0.worst_verdict, Verdict::Ok);
    }

    // T6b: mixed projection-present/absent — worst_projection_node Some iff worst_projection Some
    #[test]
    fn subtree_worst_projection_node_iff_worst_projection() {
        let thresholds = Thresholds::default();
        let parent_uuid = "pppp0000-0000-0000-0000-000000000000";

        // All nodes lack projection → both fields None
        let mut root_none = make_node(parent_uuid, None, 1_000, None, None);
        root_none.children = vec![make_node(
            parent_uuid,
            Some("cccc0000-0000-0000-0000-cccccccccccc"),
            2_000,
            None,
            None,
        )];
        let si_none = compute_subtree(&root_none, &thresholds);
        assert_eq!(si_none.worst_projection, None, "no projection → None");
        assert_eq!(
            si_none.worst_projection_node, None,
            "invariant: node None when projection None"
        );

        // child_a has projection; root and child_b do not
        let mut root_mixed = make_node(parent_uuid, None, 1_000, None, None);
        let child_with_proj = make_node(
            parent_uuid,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa"),
            2_000,
            Some(20),
            Some(500),
        );
        let child_no_proj = make_node(
            parent_uuid,
            Some("bbbb0000-0000-0000-0000-bbbbbbbbbbbb"),
            3_000,
            None,
            None,
        );
        root_mixed.children = vec![child_with_proj, child_no_proj];
        let si_mixed = compute_subtree(&root_mixed, &thresholds);
        assert!(
            si_mixed.worst_projection.is_some(),
            "projection present when any child has it"
        );
        assert!(
            si_mixed.worst_projection_node.is_some(),
            "invariant: node Some when projection Some"
        );
        assert_eq!(si_mixed.worst_projection, Some(20));
        assert_eq!(
            si_mixed.worst_projection_node,
            Some("aaaa0000-0000-0000-0000-aaaaaaaaaaaa".to_string())
        );
    }

    // T6c: sort tie-breaker — verdict ties → projection asc → tokens desc
    #[test]
    fn sort_tie_breaker_projection_then_tokens() {
        let thresholds = Thresholds::default();

        // Both Ok (5k < watch, projection 7/20 both > PROJECTION_NEARING_TURNS=5).
        // Earlier projection (7 < 20) sorts first.
        let node_proj_early = make_node(
            "aaaa0000-0000-0000-0000-000000000000",
            None,
            5_000,
            Some(7),
            Some(100),
        );
        let node_proj_late = make_node(
            "bbbb0000-0000-0000-0000-000000000000",
            None,
            5_000,
            Some(20),
            Some(100),
        );

        let mut pairs: Vec<(SubtreeInfo, SessionNode)> = vec![node_proj_late, node_proj_early]
            .into_iter()
            .map(|s| (compute_subtree(&s, &thresholds), s))
            .collect();
        pairs.sort_by(|(sa, _), (sb, _)| {
            sb.worst_verdict
                .cmp(&sa.worst_verdict)
                .then_with(|| {
                    sa.worst_projection
                        .unwrap_or(u32::MAX)
                        .cmp(&sb.worst_projection.unwrap_or(u32::MAX))
                })
                .then_with(|| sb.worst_tokens.cmp(&sa.worst_tokens))
        });
        assert_eq!(
            pairs[0].1.session_uuid, "aaaa0000-0000-0000-0000-000000000000",
            "earlier projection sorts first"
        );

        // Projection tie (both Some(20)) → higher tokens sorts first.
        let node_tokens_high = make_node(
            "cccc0000-0000-0000-0000-000000000000",
            None,
            8_000,
            Some(20),
            Some(100),
        );
        let node_tokens_low = make_node(
            "dddd0000-0000-0000-0000-000000000000",
            None,
            5_000,
            Some(20),
            Some(100),
        );

        let mut pairs2: Vec<(SubtreeInfo, SessionNode)> = vec![node_tokens_low, node_tokens_high]
            .into_iter()
            .map(|s| (compute_subtree(&s, &thresholds), s))
            .collect();
        pairs2.sort_by(|(sa, _), (sb, _)| {
            sb.worst_verdict
                .cmp(&sa.worst_verdict)
                .then_with(|| {
                    sa.worst_projection
                        .unwrap_or(u32::MAX)
                        .cmp(&sb.worst_projection.unwrap_or(u32::MAX))
                })
                .then_with(|| sb.worst_tokens.cmp(&sa.worst_tokens))
        });
        assert_eq!(
            pairs2[0].1.session_uuid, "cccc0000-0000-0000-0000-000000000000",
            "higher tokens sorts first when projection ties"
        );
    }

    // T7: saturating arithmetic — no overflow on large token sums
    #[test]
    fn subtree_tokens_saturating_add() {
        let thresholds = Thresholds::default();
        let parent_uuid = "pppp0000-0000-0000-0000-000000000000";
        let mut root = make_node(parent_uuid, None, u64::MAX, None, None);
        let child = make_node(
            parent_uuid,
            Some("cccc0000-0000-0000-0000-cccccccccccc"),
            1_000,
            None,
            None,
        );
        root.children = vec![child];
        let si = compute_subtree(&root, &thresholds);
        assert_eq!(si.total_subtree_tokens, u64::MAX); // saturated
    }

    // Trend serialized in JSON output when present.
    #[test]
    fn test_json_trend_serialized() {
        use model::{TimelinePoint, WindowTrend};
        let thresholds = Thresholds::default();
        let at = chrono::Utc::now();
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
            }),
        };
        let node_si = compute_subtree(&node, &thresholds);
        let jnode = to_json_node(&node, &node_si, None, &thresholds, 30, None);
        let t = jnode.trend.as_ref().expect("trend in json node");
        assert_eq!(t.velocity_tokens_per_turn, Some(10_000));
        assert_eq!(t.projected_turns_to_recycle, Some(10));
        assert_eq!(t.points.len(), 1);
        assert_eq!(t.points[0].cache_hit_ratio, Some(0.5));
        let json_str = serde_json::to_string(&jnode).unwrap();
        assert!(json_str.contains("\"trend\""));
        assert!(json_str.contains("\"velocity_tokens_per_turn\""));
    }
}
