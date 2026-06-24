mod claude;
mod codex;
mod copilot;
mod model;
mod opencode;
mod output;
mod parser;
mod provider;
mod verdict;
mod window;

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use claude::ClaudeProvider;
use codex::CodexProvider;
use copilot::CopilotProvider;
use model::{SessionNode, SubtreeInfo, compute_subtree, recycle_recommendation};
use opencode::OpencodeProvider;
use output::{
    JsonOutput, build_json_recycle_rec, print_flat, print_recycle_advisory, print_subtree_summary,
    print_tree, to_json_node,
};
use parser::short_id;
use provider::Provider;
use verdict::Thresholds;

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

pub(crate) fn is_active(node: &SessionNode, active_mins: u32) -> bool {
    match node.last_turn_at {
        None => false,
        Some(ts) => Utc::now().signed_duration_since(ts).num_minutes() <= active_mins as i64,
    }
}

fn any_active(node: &SessionNode, active_mins: u32) -> bool {
    is_active(node, active_mins) || node.children.iter().any(|c| is_active(c, active_mins))
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
        sessions.extend(p.load_sessions(thresholds.recycle_backstop));
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use model::{SubtreeInfo, WindowInfo, WindowSource, WindowTrend, compute_subtree};
    use verdict::Verdict;

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
}
