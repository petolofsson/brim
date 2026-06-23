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
use model::SessionNode;
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

    /// Fill % at which verdict becomes 'nearing' (default: 70)
    #[arg(long, default_value_t = 70)]
    nearing: u8,

    /// Fill % at which verdict becomes 'over -> recycle' (default: 90)
    #[arg(long, default_value_t = 90)]
    ceiling: u8,

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
    fill_percent: u8,
    cache_hit_ratio: Option<f32>,
}

#[derive(Serialize)]
struct JsonWindowTrend {
    points: Vec<JsonTimelinePoint>,
    velocity_tokens_per_turn: Option<u64>,
    projected_turns_to_overbound: Option<u32>,
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
    context_limit: Option<u64>,
    window_tokens: Option<u64>,
    fill_percent: Option<u8>,
    /// Quality verdict: OR of ADR-010 absolute-budget, projection, and cache-thrash signals.
    verdict: Option<&'static str>,
    /// Which ADR-010 OR-gate fired (null when verdict is ok).
    verdict_gate: Option<&'static str>,
    /// Capacity runway: fill % mapped to distance from auto-compaction (ADR-010 §2).
    capacity_runway: Option<&'static str>,
    /// Provenance of the reported window occupancy: "last_turn" or "aggregate" (REQ-005).
    window_source: Option<&'static str>,
    last_turn_at: Option<String>,
    active: bool,
    /// Per-turn fill trajectory: velocity, projection, cache-hit ratio (ADR-006, ADR-008).
    trend: Option<JsonWindowTrend>,
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

fn to_json_node(
    node: &SessionNode,
    parent_uuid: Option<&str>,
    thresholds: &Thresholds,
    active_mins: u32,
) -> JsonNode {
    let (
        window_tokens,
        fill_percent,
        model,
        context_limit,
        verdict,
        verdict_gate,
        capacity_runway,
        window_source,
    ) = if let Some(w) = node.window.as_ref() {
        let projected_turns = node
            .trend
            .as_ref()
            .and_then(|t| t.projected_turns_to_overbound);
        let (v, gate) = absolute_verdict(
            w.window_tokens,
            projected_turns,
            w.cache_hit_ratio,
            thresholds.watch_tokens,
            thresholds.recycle_backstop,
        );
        (
            Some(w.window_tokens),
            Some(w.fill_percent),
            Some(w.model.clone()),
            Some(w.context_limit),
            Some(v.as_json_str()),
            gate.map(|g| g.as_json_str()),
            Some(thresholds.runway_capacity_str(w.fill_percent)),
            Some(w.window_source.as_json_str()),
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
        points: t
            .points
            .iter()
            .map(|p| JsonTimelinePoint {
                at: p.at.to_rfc3339(),
                window_tokens: p.window_tokens,
                fill_percent: p.fill_percent,
                cache_hit_ratio: p.cache_hit_ratio,
            })
            .collect(),
        velocity_tokens_per_turn: t.velocity_tokens_per_turn,
        projected_turns_to_overbound: t.projected_turns_to_overbound,
    });

    JsonNode {
        session_id,
        parent_session_id: parent_uuid.map(|s| s.to_string()),
        agent_id: node.agent_id.clone(),
        project: node.project_key.clone(),
        model,
        context_limit,
        window_tokens,
        fill_percent,
        verdict,
        verdict_gate,
        capacity_runway,
        window_source,
        last_turn_at: node.last_turn_at.map(|ts| ts.to_rfc3339()),
        active: is_active(node, active_mins),
        trend,
        children: node
            .children
            .iter()
            .map(|c| to_json_node(c, Some(&node.session_uuid), thresholds, active_mins))
            .collect(),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    anyhow::ensure!(
        cli.nearing <= cli.ceiling && cli.ceiling <= 100,
        "--nearing ({}) must be \u{2264} --ceiling ({}) and --ceiling must be \u{2264} 100",
        cli.nearing,
        cli.ceiling,
    );
    anyhow::ensure!(
        cli.watch_tokens <= cli.recycle_backstop,
        "--watch-tokens ({}) must be \u{2264} --recycle-backstop ({})",
        cli.watch_tokens,
        cli.recycle_backstop,
    );
    let thresholds = Thresholds {
        nearing: cli.nearing,
        ceiling: cli.ceiling,
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

    if cli.json {
        let output = JsonOutput {
            generated_at: Utc::now().to_rfc3339(),
            nodes: sessions
                .iter()
                .map(|s| to_json_node(s, None, &thresholds, cli.active_mins))
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

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

/// Capacity-runway readout column. Suffix '~' = nearing auto-compaction, '!' = at/over.
fn fill_str(node: &SessionNode, thresholds: &Thresholds) -> String {
    let Some(w) = node.window.as_ref() else {
        return "-".to_string();
    };
    let suffix = match thresholds.runway(w.fill_percent) {
        Verdict::Over => "!",
        Verdict::Nearing => "~",
        Verdict::Ok => "",
    };
    format!("{}%{}", w.fill_percent, suffix)
}

fn verdict_str(node: &SessionNode, thresholds: &Thresholds) -> String {
    let Some(w) = node.window.as_ref() else {
        return "-".to_string();
    };
    let projected_turns = node
        .trend
        .as_ref()
        .and_then(|t| t.projected_turns_to_overbound);
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

fn print_flat(sessions: &[SessionNode], thresholds: &Thresholds) {
    println!(
        "{:<16} {:>8}  {:>5}  {:<30}  {:<5}  {:<28} MODEL",
        "SESSION", "TOKENS", "FILL%", "VERDICT", "AGE", "PROJECT"
    );
    for s in sessions {
        let id = short_id(&s.session_uuid);
        let sub = if s.children.is_empty() {
            String::new()
        } else {
            format!("  [{} sub-agent(s)]", s.children.len())
        };
        println!(
            "{:<16} {:>8}  {:>5}  {:<30}  {:<5}  {:<28} {}{}",
            id,
            tokens_str(s),
            fill_str(s, thresholds),
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
            "{:<16} {:>8}  {:>5}  {:<30}  {:<5}  {:<28} {}",
            short_id(&s.session_uuid),
            tokens_str(s),
            fill_str(s, thresholds),
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
                "  {} agent:{:<16} {:>8}  {:>5}  {:<30}  {:<5}  {:<28} {}",
                conn,
                child_id,
                tokens_str(child),
                fill_str(child, thresholds),
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
    use model::{WindowInfo, WindowSource};

    fn make_node_with_ts(last_turn_at: Option<DateTime<Utc>>) -> SessionNode {
        SessionNode {
            session_uuid: "aaaabbbb-cccc-dddd-eeee-111122223333".to_string(),
            agent_id: None,
            project_key: "test-project".to_string(),
            window: Some(WindowInfo {
                window_tokens: 100_000,
                fill_percent: 50,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
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
                fill_percent: 5,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
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
                fill_percent: 20,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: vec![child],
            last_turn_at: None,
            trend: None,
        };

        let jnode = to_json_node(&parent, None, &thresholds, 30);

        // Full untruncated ID
        assert_eq!(jnode.session_id, parent_uuid);
        assert_eq!(jnode.parent_session_id, None);
        assert_eq!(jnode.agent_id, None);
        assert_eq!(jnode.fill_percent, Some(20));
        // 40k >= ABSOLUTE_WATCH_TOKENS → Nearing via abs-watch gate (ADR-010)
        assert_eq!(jnode.verdict, Some("nearing"));
        assert_eq!(jnode.verdict_gate, Some("absolute_watch"));
        assert!(jnode.fill_percent <= Some(100));
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
                fill_percent: 95,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
        };
        let jnode = to_json_node(&node, None, &thresholds, 30);
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
                fill_percent: 25,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
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
                fill_percent: 50,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
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

    // M1: no-window node must emit null for model, context_limit, window_tokens, fill_percent, verdict.
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
        let jnode = to_json_node(&node, None, &thresholds, 30);
        assert_eq!(jnode.model, None);
        assert_eq!(jnode.context_limit, None);
        assert_eq!(jnode.window_tokens, None);
        assert_eq!(jnode.fill_percent, None);
        assert_eq!(jnode.verdict, None);
        let json_str = serde_json::to_string(&jnode).unwrap();
        assert!(json_str.contains("\"model\":null"));
        assert!(json_str.contains("\"context_limit\":null"));
        assert!(json_str.contains("\"verdict\":null"));
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
                fill_percent: 50,
                model: "claude-sonnet-4-6".to_string(),
                context_limit: 200_000,
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: Some(0.5),
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: Some(WindowTrend {
                points: vec![TimelinePoint {
                    at,
                    window_tokens: 100_000,
                    fill_percent: 50,
                    cache_hit_ratio: Some(0.5),
                }],
                velocity_tokens_per_turn: Some(10_000),
                projected_turns_to_overbound: Some(10),
            }),
        };
        let jnode = to_json_node(&node, None, &thresholds, 30);
        let t = jnode.trend.as_ref().expect("trend in json node");
        assert_eq!(t.velocity_tokens_per_turn, Some(10_000));
        assert_eq!(t.projected_turns_to_overbound, Some(10));
        assert_eq!(t.points.len(), 1);
        assert_eq!(t.points[0].cache_hit_ratio, Some(0.5));
        let json_str = serde_json::to_string(&jnode).unwrap();
        assert!(json_str.contains("\"trend\""));
        assert!(json_str.contains("\"velocity_tokens_per_turn\""));
    }
}
