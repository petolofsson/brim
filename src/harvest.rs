//! Recycle-label harvester — REQ-017 / STORY-013.
//! Extracts HarvestRecord ground-truth labels from loaded SessionNode data.
//! Read-only over sessions; never mutates transcripts (ADR-002 / ADR-004).

use crate::{
    model::SessionNode,
    parser::read_tail,
    verdict::{FamilyVoteInputs, Thresholds, family_vote_verdict},
    window::{find_reset_indices, sustained_cache_thrash},
};
use std::{collections::HashMap, io::Write, path::Path};

/// Minimum terminal occupancy for the earlier session in an adjacent pair to qualify
/// as a cross-file recycle candidate (N3: operator recycles ~100k-300k; 96k gives margin).
pub const HARVEST_RECYCLE_GATE_TOKENS: u64 = 96_000;

/// Ground-truth label record for one observed reset event (REQ-017, schema_version=1).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct HarvestRecord {
    pub schema_version: u8,
    pub provider: String,
    pub session_id: String,
    pub project_key: String,
    /// Index of the last pre-reset point in the loaded trend tail.
    pub reset_turn_index: usize,
    pub drop_magnitude: u64,
    pub occupancy_tokens: u64,
    /// "recycle" | "compaction" | "ambiguous"
    pub event_type: String,
    /// "in_session" | "cross_file"
    pub boundary_kind: String,
    /// [volume, speed, thrash, behavior, drift] over the pre-reset slice.
    pub family_fires: [bool; 5],
    pub compact_boundary_marker: bool,
    pub stop_reason_max_tokens_in_window: bool,
    pub n_turns_in_window: usize,
}

/// Collect recycle/compaction labels from all sessions (REQ-017).
/// Empty on any per-session error; never panics.
pub fn collect_labels(sessions: &[&SessionNode], thresholds: &Thresholds) -> Vec<HarvestRecord> {
    let mut records = Vec::new();
    records.extend(claude_labels(sessions, thresholds));
    records.extend(opencode_labels(sessions, thresholds));
    records.extend(copilot_labels(sessions, thresholds));
    records.extend(codex_labels(sessions, thresholds));
    records.extend(cross_file_labels(sessions, thresholds));
    records
}

/// Append records to a JSONL file; create if absent, never truncate (ADR-004).
pub fn append_records(path: &Path, records: &[HarvestRecord]) -> anyhow::Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    let mut writer = std::io::BufWriter::new(file);
    for record in records {
        let line = serde_json::to_string(record)?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// Discriminate between recycle, compaction, and ambiguous per §3 discrimination table.
/// Q5: no compact_boundary + no stop_reason → always "ambiguous" regardless of occupancy.
fn classify_event_type(
    compact_boundary_present: bool,
    stop_reason_max_tokens: bool,
    occupancy_tokens: u64,
    backstop: u64,
) -> &'static str {
    if compact_boundary_present {
        if stop_reason_max_tokens || occupancy_tokens >= backstop {
            "compaction"
        } else {
            "recycle"
        }
    } else if stop_reason_max_tokens {
        "compaction"
    } else {
        "ambiguous"
    }
}

/// Compute family_fires vector over a pre-reset slice of trend points.
fn family_fires_for_slice(
    node: &SessionNode,
    pre_tokens: u64,
    pre_slice: &[crate::model::TimelinePoint],
    thresholds: &Thresholds,
) -> [bool; 5] {
    let inputs = FamilyVoteInputs {
        window_tokens: pre_tokens,
        watch_tokens: thresholds.watch_tokens,
        recycle_backstop: thresholds.recycle_backstop,
        projected_turns: None,
        sustained_cache_thrash: sustained_cache_thrash(pre_slice),
        behavior: node.behavior.as_ref(),
        drift_score: node.trend.as_ref().and_then(|t| t.drift_score),
    };
    family_vote_verdict(&inputs).families
}

/// Scan a Claude JSONL tail for a compact_boundary system event (plan §3 second read).
/// Returns false on any read or parse error (fail-closed). Reads NO natural-language content.
fn scan_compact_boundary(path: &Path) -> bool {
    let text = match read_tail(path) {
        Ok(t) => t,
        Err(_) => return false,
    };
    text.lines().any(|line| {
        if line.trim().is_empty() {
            return false;
        }
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        obj.get("type").and_then(|v| v.as_str()) == Some("system")
            && obj.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary")
    })
}

/// Extract in-session reset records for one Claude session.
/// Performs a second bounded read_tail to detect compact_boundary markers (plan §3).
fn claude_in_session_records(node: &SessionNode, thresholds: &Thresholds) -> Vec<HarvestRecord> {
    let trend = match node.trend.as_ref().filter(|t| !t.points.is_empty()) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let reset_idxs = find_reset_indices(&trend.points);
    if reset_idxs.is_empty() {
        return Vec::new();
    }
    let stop_reason = node
        .behavior
        .as_ref()
        .is_some_and(|b| b.stop_reason_max_tokens);
    let compact_boundary_present = node
        .source_path
        .as_deref()
        .map(scan_compact_boundary)
        .unwrap_or(false);
    reset_idxs
        .into_iter()
        .map(|i| {
            let pre_idx = i - 1;
            let pre_tokens = trend.points[pre_idx].window_tokens;
            let post_tokens = trend.points[i].window_tokens;
            let family_fires =
                family_fires_for_slice(node, pre_tokens, &trend.points[..i], thresholds);
            HarvestRecord {
                schema_version: 1,
                provider: node.provider.clone(),
                session_id: node.session_uuid.clone(),
                project_key: node.project_key.clone(),
                reset_turn_index: pre_idx,
                drop_magnitude: pre_tokens.saturating_sub(post_tokens),
                occupancy_tokens: pre_tokens,
                event_type: classify_event_type(
                    compact_boundary_present,
                    stop_reason,
                    pre_tokens,
                    thresholds.recycle_backstop,
                )
                .to_string(),
                boundary_kind: "in_session".to_string(),
                family_fires,
                compact_boundary_marker: compact_boundary_present,
                stop_reason_max_tokens_in_window: stop_reason,
                n_turns_in_window: i,
            }
        })
        .collect()
}

/// Extract in-session reset records for one codex session.
/// Codex classification: stop_reason → compaction; no stop_reason + low occupancy → recycle;
/// no stop_reason + high occupancy → compaction (conservative).
fn codex_in_session_records(node: &SessionNode, thresholds: &Thresholds) -> Vec<HarvestRecord> {
    let trend = match node.trend.as_ref().filter(|t| !t.points.is_empty()) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let reset_idxs = find_reset_indices(&trend.points);
    if reset_idxs.is_empty() {
        return Vec::new();
    }
    let stop_reason = node
        .behavior
        .as_ref()
        .is_some_and(|b| b.stop_reason_max_tokens);
    reset_idxs
        .into_iter()
        .map(|i| {
            let pre_idx = i - 1;
            let pre_tokens = trend.points[pre_idx].window_tokens;
            let post_tokens = trend.points[i].window_tokens;
            let event_type = if stop_reason {
                "compaction"
            } else if pre_tokens < thresholds.recycle_backstop {
                "recycle"
            } else {
                "compaction"
            };
            let family_fires =
                family_fires_for_slice(node, pre_tokens, &trend.points[..i], thresholds);
            HarvestRecord {
                schema_version: 1,
                provider: node.provider.clone(),
                session_id: node.session_uuid.clone(),
                project_key: node.project_key.clone(),
                reset_turn_index: pre_idx,
                drop_magnitude: pre_tokens.saturating_sub(post_tokens),
                occupancy_tokens: pre_tokens,
                event_type: event_type.to_string(),
                boundary_kind: "in_session".to_string(),
                family_fires,
                compact_boundary_marker: false,
                stop_reason_max_tokens_in_window: stop_reason,
                n_turns_in_window: i,
            }
        })
        .collect()
}

/// Extract in-session reset records for one copilot session.
/// Copilot drops come from CompactionProcessor logs → all host-reported compactions.
fn copilot_in_session_records(node: &SessionNode, thresholds: &Thresholds) -> Vec<HarvestRecord> {
    let trend = match node.trend.as_ref().filter(|t| !t.points.is_empty()) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let reset_idxs = find_reset_indices(&trend.points);
    if reset_idxs.is_empty() {
        return Vec::new();
    }
    reset_idxs
        .into_iter()
        .map(|i| {
            let pre_idx = i - 1;
            let pre_tokens = trend.points[pre_idx].window_tokens;
            let post_tokens = trend.points[i].window_tokens;
            let family_fires =
                family_fires_for_slice(node, pre_tokens, &trend.points[..i], thresholds);
            HarvestRecord {
                schema_version: 1,
                provider: node.provider.clone(),
                session_id: node.session_uuid.clone(),
                project_key: node.project_key.clone(),
                reset_turn_index: pre_idx,
                drop_magnitude: pre_tokens.saturating_sub(post_tokens),
                occupancy_tokens: pre_tokens,
                event_type: "compaction".to_string(),
                boundary_kind: "in_session".to_string(),
                family_fires,
                compact_boundary_marker: false,
                stop_reason_max_tokens_in_window: false,
                n_turns_in_window: i,
            }
        })
        .collect()
}

/// Extract in-session reset records for one opencode session.
/// Rule: stop_reason → compaction; no stop_reason + drop < 50% of pre_tokens → recycle; else → ambiguous.
fn opencode_in_session_records(node: &SessionNode, thresholds: &Thresholds) -> Vec<HarvestRecord> {
    let trend = match node.trend.as_ref().filter(|t| !t.points.is_empty()) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let reset_idxs = find_reset_indices(&trend.points);
    if reset_idxs.is_empty() {
        return Vec::new();
    }
    let stop_reason = node
        .behavior
        .as_ref()
        .is_some_and(|b| b.stop_reason_max_tokens);
    reset_idxs
        .into_iter()
        .map(|i| {
            let pre_idx = i - 1;
            let pre_tokens = trend.points[pre_idx].window_tokens;
            let post_tokens = trend.points[i].window_tokens;
            let drop = pre_tokens.saturating_sub(post_tokens);
            let event_type = if stop_reason {
                "compaction"
            } else if drop < pre_tokens / 2 {
                "recycle"
            } else {
                "ambiguous"
            };
            let family_fires =
                family_fires_for_slice(node, pre_tokens, &trend.points[..i], thresholds);
            HarvestRecord {
                schema_version: 1,
                provider: node.provider.clone(),
                session_id: node.session_uuid.clone(),
                project_key: node.project_key.clone(),
                reset_turn_index: pre_idx,
                drop_magnitude: drop,
                occupancy_tokens: pre_tokens,
                event_type: event_type.to_string(),
                boundary_kind: "in_session".to_string(),
                family_fires,
                compact_boundary_marker: false,
                stop_reason_max_tokens_in_window: stop_reason,
                n_turns_in_window: i,
            }
        })
        .collect()
}

fn claude_labels(sessions: &[&SessionNode], thresholds: &Thresholds) -> Vec<HarvestRecord> {
    sessions
        .iter()
        .filter(|s| s.provider == "claude")
        .flat_map(|s| claude_in_session_records(s, thresholds))
        .collect()
}

fn opencode_labels(sessions: &[&SessionNode], thresholds: &Thresholds) -> Vec<HarvestRecord> {
    sessions
        .iter()
        .filter(|s| s.provider == "opencode")
        .flat_map(|s| opencode_in_session_records(s, thresholds))
        .collect()
}

fn copilot_labels(sessions: &[&SessionNode], thresholds: &Thresholds) -> Vec<HarvestRecord> {
    sessions
        .iter()
        .filter(|s| s.provider == "copilot")
        .flat_map(|s| copilot_in_session_records(s, thresholds))
        .collect()
}

fn codex_labels(sessions: &[&SessionNode], thresholds: &Thresholds) -> Vec<HarvestRecord> {
    sessions
        .iter()
        .filter(|s| s.provider == "codex")
        .flat_map(|s| codex_in_session_records(s, thresholds))
        .collect()
}

/// Cross-file recycle detection: adjacent sessions in the same project where the earlier
/// session ended at high occupancy (>= HARVEST_RECYCLE_GATE_TOKENS) (N3/N4).
fn cross_file_labels(sessions: &[&SessionNode], thresholds: &Thresholds) -> Vec<HarvestRecord> {
    let groups = group_by_project(sessions);
    let mut records = Vec::new();
    let mut keys: Vec<&(String, String)> = groups.keys().collect();
    keys.sort();
    for key in keys {
        let group = &groups[key];
        if group.len() < 2 {
            continue;
        }
        for pair in group.windows(2) {
            let earlier = pair[0];
            let later = pair[1];
            let Some(trend_a) = &earlier.trend else {
                continue;
            };
            let Some(last_pt) = trend_a.points.last() else {
                continue;
            };
            let terminal_occupancy = last_pt.window_tokens;
            if terminal_occupancy < HARVEST_RECYCLE_GATE_TOKENS {
                continue;
            }
            // N4: no B-side check; presence of a later session is sufficient.
            let drop_magnitude = later
                .trend
                .as_ref()
                .and_then(|t| t.points.first())
                .map(|pt| terminal_occupancy.saturating_sub(pt.window_tokens))
                .unwrap_or(0);
            let n_turns = trend_a.points.len();
            let family_fires =
                family_fires_for_slice(earlier, terminal_occupancy, &trend_a.points, thresholds);
            records.push(HarvestRecord {
                schema_version: 1,
                provider: earlier.provider.clone(),
                session_id: earlier.session_uuid.clone(),
                project_key: earlier.project_key.clone(),
                reset_turn_index: n_turns.saturating_sub(1),
                drop_magnitude,
                occupancy_tokens: terminal_occupancy,
                event_type: "recycle".to_string(),
                boundary_kind: "cross_file".to_string(),
                family_fires,
                compact_boundary_marker: false,
                stop_reason_max_tokens_in_window: false,
                n_turns_in_window: n_turns,
            });
        }
    }
    records
}

/// Group sessions by (provider, project_key); sort each group by last_turn_at ascending
/// (None sorts last), with session_uuid as a stable tiebreak (N1/N2).
fn group_by_project<'a>(
    sessions: &'a [&'a SessionNode],
) -> HashMap<(String, String), Vec<&'a SessionNode>> {
    let mut groups: HashMap<(String, String), Vec<&'a SessionNode>> = HashMap::new();
    for s in sessions {
        groups
            .entry((s.provider.clone(), s.project_key.clone()))
            .or_default()
            .push(s);
    }
    for group in groups.values_mut() {
        group.sort_by(|a, b| match (&a.last_turn_at, &b.last_turn_at) {
            (Some(ta), Some(tb)) => ta.cmp(tb).then_with(|| a.session_uuid.cmp(&b.session_uuid)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.session_uuid.cmp(&b.session_uuid),
        });
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{TimelinePoint, WindowTrend},
        verdict::Thresholds,
    };
    use chrono::{TimeZone, Utc};

    fn ts(h: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, h, 0, 0).unwrap()
    }

    fn pt(h: u32, tokens: u64) -> TimelinePoint {
        TimelinePoint {
            at: ts(h),
            window_tokens: tokens,
            cache_hit_ratio: None,
        }
    }

    fn mk_session(
        uuid: &str,
        project_key: &str,
        provider: &str,
        points: Vec<TimelinePoint>,
        last_turn_at: Option<chrono::DateTime<Utc>>,
    ) -> SessionNode {
        let trend = if points.is_empty() {
            None
        } else {
            Some(WindowTrend {
                points,
                velocity_tokens_per_turn: None,
                projected_turns_to_recycle: None,
                drift_score: None,
            })
        };
        SessionNode {
            session_uuid: uuid.to_string(),
            agent_id: None,
            project_key: project_key.to_string(),
            provider: provider.to_string(),
            window: None,
            children: Vec::new(),
            last_turn_at,
            trend,
            behavior: None,
            source_path: None,
        }
    }

    // Table-driven classify_event_type tests (§3 discrimination table, all rows incl ambiguous).
    #[test]
    fn classify_compact_and_stop_reason_is_compaction() {
        assert_eq!(
            classify_event_type(true, true, 50_000, 128_000),
            "compaction"
        );
    }

    #[test]
    fn classify_compact_no_stop_low_occupancy_is_recycle() {
        assert_eq!(
            classify_event_type(true, false, 50_000, 128_000),
            "recycle"
        );
    }

    #[test]
    fn classify_compact_no_stop_high_occupancy_is_compaction() {
        assert_eq!(
            classify_event_type(true, false, 128_000, 128_000),
            "compaction"
        );
    }

    #[test]
    fn classify_no_compact_stop_reason_is_compaction() {
        assert_eq!(
            classify_event_type(false, true, 200_000, 128_000),
            "compaction"
        );
    }

    #[test]
    fn classify_no_compact_no_stop_any_occupancy_is_ambiguous() {
        // Q5: always ambiguous regardless of occupancy
        assert_eq!(
            classify_event_type(false, false, 200_000, 128_000),
            "ambiguous"
        );
        assert_eq!(
            classify_event_type(false, false, 10_000, 128_000),
            "ambiguous"
        );
    }

    // collect_labels: synthetic in-session drop → boundary_kind=in_session.
    #[test]
    fn collect_labels_synthetic_drop() {
        let thresholds = Thresholds::default();
        // Points: 50k, 80k, 20k (reset!), 40k
        let node = mk_session(
            "sess-0001",
            "my-project",
            "claude",
            vec![pt(0, 50_000), pt(1, 80_000), pt(2, 20_000), pt(3, 40_000)],
            Some(ts(3)),
        );
        let sessions = vec![&node];
        let records = collect_labels(&sessions, &thresholds);
        // One in-session record from the drop at index 2 (80k→20k)
        assert_eq!(records.len(), 1, "one reset event expected");
        let r = &records[0];
        assert_eq!(r.boundary_kind, "in_session");
        assert_eq!(r.session_id, "sess-0001");
        assert_eq!(r.provider, "claude");
        assert_eq!(r.occupancy_tokens, 80_000);
        assert_eq!(r.drop_magnitude, 60_000);
        assert_eq!(r.reset_turn_index, 1); // last pre-reset index = i-1 = 1
        assert_eq!(r.n_turns_in_window, 2); // i=2 points before reset
        assert!(!r.compact_boundary_marker);
    }

    // cross_file_recycle_boundary: two same-project sessions, earlier at high occupancy
    // → exactly one recycle record with boundary_kind=cross_file, session_id=earlier uuid.
    #[test]
    fn cross_file_recycle_boundary() {
        let thresholds = Thresholds::default();
        let earlier = mk_session(
            "earlier-uuid",
            "shared-project",
            "claude",
            vec![pt(0, 50_000), pt(1, 100_000)], // terminal = 100k >= 96k gate
            Some(ts(1)),
        );
        let later = mk_session(
            "later-uuid",
            "shared-project",
            "claude",
            vec![pt(2, 5_000)], // fresh start
            Some(ts(2)),
        );
        let sessions: Vec<&SessionNode> = vec![&earlier, &later];
        let records = collect_labels(&sessions, &thresholds);
        let cross: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "cross_file")
            .collect();
        assert_eq!(cross.len(), 1, "exactly one cross-file record");
        let r = &cross[0];
        assert_eq!(r.event_type, "recycle");
        assert_eq!(r.session_id, "earlier-uuid");
        assert_eq!(r.occupancy_tokens, 100_000);
        assert_eq!(r.drop_magnitude, 95_000); // 100k - 5k
    }

    // cross_file: earlier terminal below gate → no cross-file record.
    #[test]
    fn cross_file_below_gate_no_record() {
        let thresholds = Thresholds::default();
        let earlier = mk_session(
            "low-occ",
            "proj",
            "claude",
            vec![pt(0, 30_000)], // terminal = 30k < 96k gate
            Some(ts(0)),
        );
        let later = mk_session(
            "later-uuid",
            "proj",
            "claude",
            vec![pt(1, 5_000)],
            Some(ts(1)),
        );
        let sessions: Vec<&SessionNode> = vec![&earlier, &later];
        let records = collect_labels(&sessions, &thresholds);
        let cross: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "cross_file")
            .collect();
        assert!(cross.is_empty(), "below-gate terminal must not yield record");
    }

    // append_records: two calls → line count doubles; each line parses as HarvestRecord.
    #[test]
    fn append_records_idempotent() {
        let path = std::env::temp_dir().join("brim_harvest_append_test.jsonl");
        // Remove any leftover from a prior run.
        let _ = std::fs::remove_file(&path);
        let thresholds = Thresholds::default();
        let node = mk_session(
            "sess-append",
            "proj",
            "claude",
            vec![pt(0, 50_000), pt(1, 80_000), pt(2, 20_000)],
            Some(ts(2)),
        );
        let sessions = vec![&node];
        let records = collect_labels(&sessions, &thresholds);
        assert!(!records.is_empty(), "need at least one record for this test");

        append_records(&path, &records).expect("first write");
        append_records(&path, &records).expect("second write");

        let content = std::fs::read_to_string(&path).expect("read file");
        let lines: Vec<_> = content.lines().collect();
        assert_eq!(
            lines.len(),
            records.len() * 2,
            "second call appends, not overwrites"
        );
        for line in &lines {
            serde_json::from_str::<HarvestRecord>(line)
                .unwrap_or_else(|e| panic!("line must parse as HarvestRecord: {e}: {line}"));
        }
    }

    // Fix 1 (N1): same project_key but different providers must NEVER pair as cross-file.
    #[test]
    fn cross_file_different_provider_no_pairing() {
        let thresholds = Thresholds::default();
        let earlier = mk_session(
            "a-uuid",
            "brim",
            "claude",
            vec![pt(0, 50_000), pt(1, 100_000)], // terminal = 100k >= 96k gate
            Some(ts(1)),
        );
        let later = mk_session(
            "b-uuid",
            "brim",     // same project_key
            "opencode", // different provider
            vec![pt(2, 5_000)],
            Some(ts(2)),
        );
        let sessions: Vec<&SessionNode> = vec![&earlier, &later];
        let records = collect_labels(&sessions, &thresholds);
        let cross: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "cross_file")
            .collect();
        assert!(
            cross.is_empty(),
            "different providers must not pair as cross-file"
        );
    }

    // Fix 2: Claude tail with compact_boundary + stop_reason → compaction (exercises Yes/compaction row).
    #[test]
    fn claude_compact_boundary_with_stop_reason_is_compaction() {
        let path = std::env::temp_dir().join("brim_test_compact_compaction.jsonl");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "{\"type\":\"system\",\"subtype\":\"compact_boundary\"}\n\
             {\"type\":\"assistant\",\"timestamp\":\"2026-01-01T01:00:00Z\",\
             \"message\":{\"model\":\"claude-sonnet-4-6\",\"stop_reason\":\"max_tokens\",\
             \"usage\":{\"input_tokens\":100000,\"cache_read_input_tokens\":0,\
             \"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
        )
        .unwrap();
        let thresholds = Thresholds::default();
        // Session: drop at index 2 (100k→20k)
        let mut node = mk_session(
            "claude-cb-comp",
            "proj",
            "claude",
            vec![pt(0, 50_000), pt(1, 100_000), pt(2, 20_000)],
            Some(ts(2)),
        );
        node.source_path = Some(path.clone());
        // Wire stop_reason via behavior signals so classify_event_type sees it
        node.behavior = Some(crate::verdict::BehaviorSignals {
            repetition_run: None,
            failure_streak: None,
            ping_pong_count: None,
            stop_reason_max_tokens: true,
        });
        let sessions = vec![&node];
        let records = collect_labels(&sessions, &thresholds);
        let in_sess: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "in_session")
            .collect();
        assert_eq!(in_sess.len(), 1);
        assert_eq!(in_sess[0].event_type, "compaction");
        assert!(in_sess[0].compact_boundary_marker, "marker must be set");
        let _ = std::fs::remove_file(&path);
    }

    // Fix 2: compact_boundary + no stop_reason + occupancy < backstop → recycle
    // (exercises the now-reachable Yes/recycle row of classify_event_type).
    #[test]
    fn claude_compact_boundary_no_stop_low_occ_is_recycle() {
        let path = std::env::temp_dir().join("brim_test_compact_recycle.jsonl");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "{\"type\":\"system\",\"subtype\":\"compact_boundary\"}\n")
            .unwrap();
        let thresholds = Thresholds::default(); // backstop = 128k
        // pre_tokens = 50k < 128k backstop; no stop_reason (behavior = None)
        let mut node = mk_session(
            "claude-cb-recycle",
            "proj2",
            "claude",
            vec![pt(0, 20_000), pt(1, 50_000), pt(2, 5_000)], // reset at i=2
            Some(ts(2)),
        );
        node.source_path = Some(path.clone());
        let sessions = vec![&node];
        let records = collect_labels(&sessions, &thresholds);
        let in_sess: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "in_session")
            .collect();
        assert_eq!(in_sess.len(), 1);
        assert_eq!(in_sess[0].event_type, "recycle");
        assert!(in_sess[0].compact_boundary_marker, "marker must be set");
        let _ = std::fs::remove_file(&path);
    }

    // Fix 4: opencode drop < 50% of pre_tokens → recycle.
    #[test]
    fn opencode_drop_below_50pct_is_recycle() {
        let thresholds = Thresholds::default();
        // pre_tokens = 80k, post_tokens = 50k → drop = 30k < 40k (50% of 80k) → recycle
        let node = mk_session(
            "oc-recycle",
            "proj",
            "opencode",
            vec![pt(0, 40_000), pt(1, 80_000), pt(2, 50_000)],
            Some(ts(2)),
        );
        let sessions = vec![&node];
        let records = collect_labels(&sessions, &thresholds);
        let in_sess: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "in_session")
            .collect();
        assert_eq!(in_sess.len(), 1);
        assert_eq!(in_sess[0].event_type, "recycle");
    }

    // Fix 4: opencode drop >= 50% of pre_tokens → ambiguous.
    #[test]
    fn opencode_drop_above_50pct_is_ambiguous() {
        let thresholds = Thresholds::default();
        // pre_tokens = 80k, post_tokens = 10k → drop = 70k > 40k (50% of 80k) → ambiguous
        let node = mk_session(
            "oc-ambiguous",
            "proj",
            "opencode",
            vec![pt(0, 40_000), pt(1, 80_000), pt(2, 10_000)],
            Some(ts(2)),
        );
        let sessions = vec![&node];
        let records = collect_labels(&sessions, &thresholds);
        let in_sess: Vec<_> = records
            .iter()
            .filter(|r| r.boundary_kind == "in_session")
            .collect();
        assert_eq!(in_sess.len(), 1);
        assert_eq!(in_sess[0].event_type, "ambiguous");
    }
}
