use chrono::{DateTime, Utc};

use crate::verdict::{Thresholds, Verdict, VerdictGate, absolute_verdict};
use crate::window::sustained_cache_thrash;

/// Provenance of the reported window occupancy (REQ-005 machine-readable).
/// `LastTurn` = computed from the latest point-in-time usage record.
/// `Aggregate` = fell back to session-aggregate columns (no point-in-time record).
/// `ProcessLog` = extracted from `CompactionProcessor` lines in process-<pid>.log (REQ-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSource {
    LastTurn,
    Aggregate,
    ProcessLog,
}

impl WindowSource {
    pub fn as_json_str(self) -> &'static str {
        match self {
            WindowSource::LastTurn => "last_turn",
            WindowSource::Aggregate => "aggregate",
            WindowSource::ProcessLog => "process_log",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub window_tokens: u64,
    pub model: String,
    pub window_source: WindowSource,
    /// cache_read / window_tokens, bounded [0,1]. None when no cache split (ADR-008).
    pub cache_hit_ratio: Option<f32>,
}

/// One point in a session's fill trajectory (ADR-006, REQ-007).
/// ADR-013: not serialized to JSON; the trend JSON shape carries only
/// `velocity` + `proj_turns`. Retained internally for the verdict projection.
#[derive(Debug, Clone)]
pub struct TimelinePoint {
    /// Timestamp of the turn. ADR-013: not consumed by the verdict path; kept
    /// for future timeline-emitting flags and for build-time ordering.
    #[allow(dead_code)]
    pub at: DateTime<Utc>,
    pub window_tokens: u64,
    /// Per-turn cache hit ratio; consumed by window::sustained_cache_thrash for the verdict path.
    pub cache_hit_ratio: Option<f32>,
}

/// Growth trend derived from a bounded tail read of the last K assistant turns (ADR-006).
#[derive(Debug, Clone)]
pub struct WindowTrend {
    /// Per-turn trajectory tail; drives velocity/projection (ADR-006) and
    /// sustained_cache_thrash (new ADR). Not serialized to JSON (ADR-013).
    pub points: Vec<TimelinePoint>,
    pub velocity_tokens_per_turn: Option<u64>,
    pub projected_turns_to_recycle: Option<u32>,
}

/// Aggregated health across a node and all its descendants (ADR-007).
/// Cost aggregation omitted: brim reads point-in-time window, not cumulative spend (ADR-002).
/// Per-metric rules preserved — metrics are NOT blended into a single scalar.
#[derive(Debug, Clone)]
pub struct SubtreeInfo {
    /// Saturating sum of window_tokens across the subtree.
    pub total_subtree_tokens: u64,
    /// Highest window_tokens of any single node in the subtree.
    pub worst_tokens: u64,
    /// Node identifier (agent_id if sub-agent, else session_uuid) of the node with highest tokens.
    pub worst_tokens_node: String,
    /// Smallest projected_turns_to_recycle in the subtree (earliest deadline; worst growth).
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

    let (self_tokens, self_verdict) = if let Some(w) = &node.window {
        let trend = node.trend.as_ref();
        let projected_turns = trend.and_then(|t| t.projected_turns_to_recycle);
        let thrash = trend.is_some_and(|t| sustained_cache_thrash(&t.points));
        let (v, _) = absolute_verdict(
            w.window_tokens,
            projected_turns,
            thrash,
            thresholds.watch_tokens,
            thresholds.recycle_backstop,
        );
        (w.window_tokens, v)
    } else {
        (0, Verdict::Ok)
    };

    let self_projection = node
        .trend
        .as_ref()
        .and_then(|t| t.projected_turns_to_recycle);
    let self_velocity = node.trend.as_ref().and_then(|t| t.velocity_tokens_per_turn);

    let mut total_subtree_tokens = self_tokens;
    let mut worst_tokens = self_tokens;
    let mut worst_tokens_node = node_id.clone();
    let mut worst_projection = self_projection;
    let mut worst_projection_node = self_projection.map(|_| node_id.clone());
    let mut max_velocity = self_velocity;
    let mut worst_verdict = self_verdict;
    let mut worst_verdict_node = node_id;

    for child in &node.children {
        let ci = compute_subtree(child, thresholds);

        total_subtree_tokens = total_subtree_tokens.saturating_add(ci.total_subtree_tokens);

        if ci.worst_tokens > worst_tokens {
            worst_tokens = ci.worst_tokens;
            worst_tokens_node = ci.worst_tokens_node;
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
        worst_tokens,
        worst_tokens_node,
        worst_projection,
        worst_projection_node,
        max_velocity,
        worst_verdict,
        worst_verdict_node,
    }
}

/// One entry in a recycle blast radius — a descendant of the recycle target (ADR-009).
#[derive(Debug, Clone)]
pub struct BlastRadiusEntry {
    pub node_id: String,
    /// True when the node's last-turn timestamp is within the active threshold (REQ-006).
    /// Set by the caller's is_active_fn to avoid threading wall-clock into model logic.
    pub active: bool,
}

/// Recycle recommendation for a session tree (ADR-009).
/// Advisory only — brim never acts; the user decides (ADR-010 §5).
#[derive(Debug, Clone)]
pub struct RecycleRecommendation {
    /// Node id of the recommended recycle target (agent_id if sub-agent, else session_uuid).
    pub target_node_id: String,
    /// True when the target is the root of the session tree.
    /// Recycling the root restarts the whole operation — flag distinctly.
    pub is_root: bool,
    /// Self-verdict of the target node (the gate that makes it explanatory).
    pub target_verdict: Verdict,
    /// Which ADR-010 OR-gate fired on the target's own window. None only if window absent.
    pub verdict_gate: Option<VerdictGate>,
    /// Descendants of the target, each marked active (REQ-006) by the caller.
    /// Empty when the target is a leaf (recyclable independently of its parent).
    pub blast_radius: Vec<BlastRadiusEntry>,
}

/// Compute a recycle recommendation for a session tree (ADR-009).
///
/// Returns `None` when every node in the tree is self-healthy (worst self-verdict == Ok).
///
/// Target selection rules (deterministic, clock-independent):
///   1. Find worst self-verdict across all nodes (Over > Nearing > Ok).
///   2. Among nodes whose OWN self-verdict equals that worst, pick the DEEPEST.
///
/// Tie-break (documented for auditability): projection ASC (None = ∞, no deadline),
/// tokens DESC (larger occupancy is more urgent), node_id ASC (lexicographic, final arbiter).
///
/// `is_active_fn` is injected for blast-radius active marking; it may be clock-dependent.
/// The target selection itself is pure (verdict + tree only) and unit-testable without a clock.
pub fn recycle_recommendation(
    root: &SessionNode,
    thresholds: &Thresholds,
    is_active_fn: &impl Fn(&SessionNode) -> bool,
) -> Option<RecycleRecommendation> {
    let mut worst = Verdict::Ok;
    scan_worst_self_verdict(root, thresholds, &mut worst);
    if worst == Verdict::Ok {
        return None;
    }

    // Walk tree to find deepest node whose own verdict == worst (tie-break documented above).
    let mut best: Option<(u32, u32, u64, String)> = None; // (depth, proj, tokens, node_id)
    find_target_node(root, thresholds, worst, 0, &mut best);
    let (_, _, _, target_id) = best?;

    let root_id = node_id_str(root).to_string();
    let is_root = target_id == root_id;

    let (target_verdict, verdict_gate) = find_node(root, &target_id)
        .map(|n| self_verdict(n, thresholds))
        .unwrap_or((worst, None));

    let mut blast_radius = Vec::new();
    if let Some(target_node) = find_node(root, &target_id) {
        collect_descendants(target_node, is_active_fn, &mut blast_radius);
    }

    Some(RecycleRecommendation {
        target_node_id: target_id,
        is_root,
        target_verdict,
        verdict_gate,
        blast_radius,
    })
}

fn node_id_str(node: &SessionNode) -> &str {
    node.agent_id.as_deref().unwrap_or(&node.session_uuid)
}

fn self_verdict(node: &SessionNode, thresholds: &Thresholds) -> (Verdict, Option<VerdictGate>) {
    if let Some(w) = &node.window {
        let trend = node.trend.as_ref();
        let projected_turns = trend.and_then(|t| t.projected_turns_to_recycle);
        let thrash = trend.is_some_and(|t| sustained_cache_thrash(&t.points));
        absolute_verdict(
            w.window_tokens,
            projected_turns,
            thrash,
            thresholds.watch_tokens,
            thresholds.recycle_backstop,
        )
    } else {
        (Verdict::Ok, None)
    }
}

fn scan_worst_self_verdict(node: &SessionNode, thresholds: &Thresholds, worst: &mut Verdict) {
    let (v, _) = self_verdict(node, thresholds);
    if v > *worst {
        *worst = v;
    }
    for child in &node.children {
        scan_worst_self_verdict(child, thresholds, worst);
    }
}

fn find_target_node(
    node: &SessionNode,
    thresholds: &Thresholds,
    target_verdict: Verdict,
    depth: u32,
    best: &mut Option<(u32, u32, u64, String)>,
) {
    let (v, _) = self_verdict(node, thresholds);
    if v == target_verdict {
        let proj = node
            .trend
            .as_ref()
            .and_then(|t| t.projected_turns_to_recycle)
            .unwrap_or(u32::MAX);
        let tokens = node.window.as_ref().map(|w| w.window_tokens).unwrap_or(0);
        let id = node_id_str(node).to_string();
        let is_better = match best.as_ref() {
            None => true,
            Some((bd, bp, bt, bi)) => {
                depth > *bd
                    || (depth == *bd && proj < *bp)
                    || (depth == *bd && proj == *bp && tokens > *bt)
                    || (depth == *bd && proj == *bp && tokens == *bt && id < *bi)
            }
        };
        if is_better {
            *best = Some((depth, proj, tokens, id));
        }
    }
    for child in &node.children {
        find_target_node(child, thresholds, target_verdict, depth + 1, best);
    }
}

fn find_node<'a>(node: &'a SessionNode, id: &str) -> Option<&'a SessionNode> {
    if node_id_str(node) == id {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_node(child, id) {
            return Some(found);
        }
    }
    None
}

fn collect_descendants(
    node: &SessionNode,
    is_active_fn: &impl Fn(&SessionNode) -> bool,
    out: &mut Vec<BlastRadiusEntry>,
) {
    for child in &node.children {
        out.push(BlastRadiusEntry {
            node_id: node_id_str(child).to_string(),
            active: is_active_fn(child),
        });
        collect_descendants(child, is_active_fn, out);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::{Thresholds, Verdict};

    // Token landmarks: Ok < 32_000, Nearing >= 32_000, Over >= 128_000 (Thresholds::default()).
    const OK_TOKENS: u64 = 5_000;
    const OVER_TOKENS: u64 = 150_000;

    fn mk(uuid: &str, agent_id: Option<&str>, tokens: u64) -> SessionNode {
        SessionNode {
            session_uuid: uuid.to_string(),
            agent_id: agent_id.map(str::to_string),
            project_key: "test".to_string(),
            window: Some(WindowInfo {
                window_tokens: tokens,
                model: "m".to_string(),
                window_source: WindowSource::LastTurn,
                cache_hit_ratio: None,
            }),
            children: Vec::new(),
            last_turn_at: None,
            trend: None,
        }
    }

    // R1: degraded leaf selected over unhealthy parent (leaf is deeper + explanatory).
    #[test]
    fn recycle_degraded_leaf_over_unhealthy_parent() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let leaf_id = "leaf0000-0000-0000-0000-000000000000";

        let mut root = mk(root_uuid, None, 40_000); // Nearing
        let leaf = mk(root_uuid, Some(leaf_id), OVER_TOKENS); // Over — explanatory
        root.children = vec![leaf];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(
            rec.target_node_id, leaf_id,
            "leaf is deeper and explains Over"
        );
        assert_eq!(rec.target_verdict, Verdict::Over);
        assert!(!rec.is_root);
        assert!(rec.blast_radius.is_empty(), "leaf has no descendants");
    }

    // R2: intermediate selected when ITS OWN self-metrics are the cause.
    #[test]
    fn recycle_intermediate_when_own_cause() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let mid_id = "mid00000-0000-0000-0000-000000000000";
        let leaf_id = "leaf0000-0000-0000-0000-000000000000";

        let mut root = mk(root_uuid, None, OK_TOKENS); // Ok — not explanatory
        let leaf = mk(root_uuid, Some(leaf_id), OK_TOKENS); // Ok
        let mut mid = mk(root_uuid, Some(mid_id), OVER_TOKENS); // Over — explanatory
        mid.children = vec![leaf];
        root.children = vec![mid];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(
            rec.target_node_id, mid_id,
            "intermediate is Over while leaf is Ok"
        );
        assert_eq!(rec.target_verdict, Verdict::Over);
        assert!(!rec.is_root);
        // blast radius = leaf
        assert_eq!(rec.blast_radius.len(), 1);
        assert_eq!(rec.blast_radius[0].node_id, leaf_id);
    }

    // R3: root-case flagged distinctly when root is the target.
    #[test]
    fn recycle_root_case_flagged() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let child_id = "chld0000-0000-0000-0000-000000000000";

        let mut root = mk(root_uuid, None, OVER_TOKENS); // Over — root IS the target
        let child = mk(root_uuid, Some(child_id), OK_TOKENS); // Ok
        root.children = vec![child];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(rec.target_node_id, root_uuid);
        assert!(rec.is_root, "root-case must be flagged");
        assert_eq!(rec.target_verdict, Verdict::Over);
        // blast radius includes child
        assert_eq!(rec.blast_radius.len(), 1);
        assert_eq!(rec.blast_radius[0].node_id, child_id);
    }

    // R4: blast radius lists correct descendants with correct active markers.
    #[test]
    fn recycle_blast_radius_descendants_and_active_markers() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let target_id = "tgt00000-0000-0000-0000-000000000000";
        let active_id = "actv0000-0000-0000-0000-000000000000";
        let inactive_id = "inac0000-0000-0000-0000-000000000000";

        let active_child = mk(root_uuid, Some(active_id), OK_TOKENS);
        let inactive_child = mk(root_uuid, Some(inactive_id), OK_TOKENS);
        let mut target = mk(root_uuid, Some(target_id), OVER_TOKENS);
        target.children = vec![active_child, inactive_child];

        let mut root = mk(root_uuid, None, OK_TOKENS);
        root.children = vec![target];

        // is_active_fn: active_id is active, inactive_id is not.
        let is_active = |n: &SessionNode| n.agent_id.as_deref() == Some(active_id);
        let rec = recycle_recommendation(&root, &thresholds, &is_active)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(rec.target_node_id, target_id);
        assert_eq!(rec.blast_radius.len(), 2);
        let active_entry = rec
            .blast_radius
            .iter()
            .find(|e| e.node_id == active_id)
            .unwrap();
        let inactive_entry = rec
            .blast_radius
            .iter()
            .find(|e| e.node_id == inactive_id)
            .unwrap();
        assert!(active_entry.active, "active_id must be marked active");
        assert!(
            !inactive_entry.active,
            "inactive_id must be marked inactive"
        );
    }

    // R5: healthy subtree yields None.
    #[test]
    fn recycle_healthy_subtree_yields_none() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let child_id = "chld0000-0000-0000-0000-000000000000";

        let mut root = mk(root_uuid, None, OK_TOKENS);
        let child = mk(root_uuid, Some(child_id), OK_TOKENS);
        root.children = vec![child];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false);
        assert!(rec.is_none(), "healthy subtree must yield None");
    }

    // R6: flat single-node provider → target self, empty blast radius, is_root true.
    #[test]
    fn recycle_flat_single_node_self_empty_blast() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let root = mk(root_uuid, None, OVER_TOKENS);

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("over node must yield recommendation");

        assert_eq!(rec.target_node_id, root_uuid);
        assert!(rec.is_root);
        assert!(
            rec.blast_radius.is_empty(),
            "leaf/flat node has no descendants"
        );
    }

    // R7: parent and child share the same worst self-verdict (both Over) — depth wins.
    #[test]
    fn recycle_depth_wins_when_parent_and_child_both_over() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let child_id = "chld0000-0000-0000-0000-000000000000";

        let mut root = mk(root_uuid, None, OVER_TOKENS); // Over at depth 0
        let child = mk(root_uuid, Some(child_id), OVER_TOKENS); // Over at depth 1 — deeper
        root.children = vec![child];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(
            rec.target_node_id, child_id,
            "child is deeper and shares Over verdict — depth wins"
        );
        assert_eq!(rec.target_verdict, Verdict::Over);
        assert!(!rec.is_root);
        assert!(rec.blast_radius.is_empty(), "child has no descendants");
    }

    // R8a: projection-ASC tie-break — sibling with smaller projection wins over sibling with None (∞).
    #[test]
    fn recycle_tiebreak_projection_asc() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let no_proj_id = "noproj00-0000-0000-0000-000000000000";
        let soon_id = "soon0000-0000-0000-0000-000000000000";

        // Both Over at depth 1; no_proj_id processed first (proj=None→u32::MAX), soon_id has proj=2.
        let no_proj = mk(root_uuid, Some(no_proj_id), OVER_TOKENS);
        let mut soon = mk(root_uuid, Some(soon_id), OVER_TOKENS);
        soon.trend = Some(WindowTrend {
            points: vec![],
            velocity_tokens_per_turn: None,
            projected_turns_to_recycle: Some(2),
        });

        let mut root = mk(root_uuid, None, OK_TOKENS);
        root.children = vec![no_proj, soon];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(
            rec.target_node_id, soon_id,
            "projection-ASC: proj=2 < MAX (no deadline is treated as ∞)"
        );
    }

    // R8b: tokens-DESC tie-break — sibling with higher window_tokens wins when depth and projection are equal.
    #[test]
    fn recycle_tiebreak_tokens_desc() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let lo_id = "lo-fill0-0000-0000-0000-000000000000";
        let hi_id = "hi-fill0-0000-0000-0000-000000000000";

        // Both Over, no trend (proj equal at u32::MAX); lo_id=150k tokens, hi_id=160k tokens.
        let lo = mk(root_uuid, Some(lo_id), OVER_TOKENS); // 150k tokens
        let hi = mk(root_uuid, Some(hi_id), 160_000); // 160k tokens, still Over

        let mut root = mk(root_uuid, None, OK_TOKENS);
        root.children = vec![lo, hi]; // lo processed first

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(
            rec.target_node_id, hi_id,
            "tokens-DESC: 160k > 150k at equal depth and projection"
        );
    }

    // R8c: node_id-ASC final arbiter — lexicographically smaller id wins when all else is equal.
    #[test]
    fn recycle_tiebreak_node_id_asc() {
        let thresholds = Thresholds::default();
        let root_uuid = "root0000-0000-0000-0000-000000000000";
        let later_id = "bbb00000-0000-0000-0000-000000000000"; // processed first
        let earlier_id = "aaa00000-0000-0000-0000-000000000000"; // smaller lexicographically

        // Both Over, same tokens, no trend (equal proj).
        let later = mk(root_uuid, Some(later_id), OVER_TOKENS);
        let earlier = mk(root_uuid, Some(earlier_id), OVER_TOKENS);

        let mut root = mk(root_uuid, None, OK_TOKENS);
        root.children = vec![later, earlier];

        let rec = recycle_recommendation(&root, &thresholds, &|_| false)
            .expect("unhealthy subtree must yield recommendation");

        assert_eq!(
            rec.target_node_id, earlier_id,
            "node_id-ASC: 'aaa...' < 'bbb...'"
        );
    }
}
