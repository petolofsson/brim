use crate::{
    model::{SessionNode, TimelinePoint, WindowInfo, WindowSource, WindowTrend},
    parser::{home_dir, read_tail},
    provider::Provider,
    verdict::BehaviorSignals,
    window::{TREND_TAIL_K, compute_trend, compute_window_info},
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

/// Max sub-agents listed per parent session (CODERULES r2-3).
const MAX_SUBAGENTS: usize = 64;
/// Max parent sessions enumerated across all projects (CODERULES r3 bounds).
const MAX_PARENT_SESSIONS: usize = 256;
/// Max parent-session .jsonl files parsed per project directory (CODERULES r2-3 bounds).
const MAX_FILES_PER_PROJECT: usize = 64;

pub struct ClaudeProvider {
    pub home: PathBuf,
}

impl ClaudeProvider {
    pub fn new() -> Self {
        Self { home: home_dir() }
    }

    fn projects_dir(&self) -> PathBuf {
        self.home.join(".claude").join("projects")
    }
}

impl Provider for ClaudeProvider {
    fn is_available(&self) -> bool {
        self.projects_dir().exists()
    }

    fn load_sessions(&self, backstop: u64) -> Vec<SessionNode> {
        discover_sessions(&self.projects_dir(), backstop)
    }
}

struct TurnData {
    input: u64,
    cache_read: u64,
    cache_create: u64,
    model: String,
    ts: Option<DateTime<Utc>>,
    tool_calls: Vec<(String, u64)>,
    /// tool_result error flags from human turns preceding this assistant turn.
    error_flags: Vec<bool>,
    stop_reason_max_tokens: bool,
}

/// Scan the tail of a JSONL transcript, collect the last TREND_TAIL_K assistant turns,
/// and return the last-turn WindowInfo, its timestamp, the velocity trend, and
/// behavioral degradation signals (ADR-024/ADR-025).
fn parse_transcript(
    path: &Path,
    backstop: u64,
) -> (
    Option<WindowInfo>,
    Option<DateTime<Utc>>,
    Option<WindowTrend>,
    Option<BehaviorSignals>,
) {
    let text = match read_tail(path) {
        Ok(t) => t,
        Err(_) => return (None, None, None, None),
    };

    // Collect all valid (non-zero-usage) assistant turns from the tail.
    let mut turns: Vec<TurnData> = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        let turn_type = obj.get("type").and_then(|v| v.as_str());

        if turn_type == Some("human") || turn_type == Some("user") {
            // Attribute tool_result errors to the preceding assistant turn.
            if let Some(content) = obj
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                        let is_error = block
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if let Some(last_turn) = turns.last_mut() {
                            last_turn.error_flags.push(is_error);
                        }
                    }
                }
            }
            continue;
        }

        if turn_type != Some("assistant") {
            continue;
        }

        let ts_opt = obj
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<DateTime<Utc>>().ok());

        let Some(msg) = obj.get("message") else {
            continue;
        };
        let Some(usage) = msg.get("usage") else {
            continue;
        };

        let input = usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_create = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if input == 0 && cache_read == 0 && cache_create == 0 {
            continue;
        }

        let model = msg
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let stop_reason_max_tokens =
            msg.get("stop_reason").and_then(|v| v.as_str()) == Some("max_tokens");

        // Extract tool_use blocks (structure only; input is hashed, never stored — CODERULES r11).
        let tool_calls: Vec<(String, u64)> = msg
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|block| {
                        if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                            return None;
                        }
                        let name = block.get("name").and_then(|v| v.as_str())?.to_string();
                        let input_val = block.get("input").unwrap_or(&Value::Null);
                        let canonical = serde_json::to_string(input_val).unwrap_or_default();
                        let mut hasher = DefaultHasher::new();
                        canonical.hash(&mut hasher);
                        Some((name, hasher.finish()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        turns.push(TurnData {
            input,
            cache_read,
            cache_create,
            model,
            ts: ts_opt,
            tool_calls,
            error_flags: Vec::new(),
            stop_reason_max_tokens,
        });
    }

    // Bound to last K turns (tail read — never keep unbounded growth).
    if turns.len() > TREND_TAIL_K {
        turns.drain(0..turns.len() - TREND_TAIL_K);
    }

    if turns.is_empty() {
        return (None, None, None, None);
    }

    // Last valid turn → WindowInfo. Timestamp bound to this turn only (B1).
    let Some(last) = turns.last() else {
        return (None, None, None, None);
    };
    let window_info = compute_window_info(
        last.input,
        last.cache_read,
        last.cache_create,
        &last.model,
        WindowSource::LastTurn,
    );
    let last_ts = last.ts;
    let last_model = last.model.clone();

    // Build timeline points for turns that carry a timestamp.
    let timeline_points: Vec<TimelinePoint> = turns
        .iter()
        .filter_map(|t| {
            let at = t.ts?;
            let info = compute_window_info(
                t.input,
                t.cache_read,
                t.cache_create,
                &last_model,
                WindowSource::LastTurn,
            );
            Some(TimelinePoint {
                at,
                window_tokens: info.window_tokens,
                cache_hit_ratio: info.cache_hit_ratio,
            })
        })
        .collect();

    let trend = if !timeline_points.is_empty() {
        Some(compute_trend(timeline_points, backstop))
    } else {
        None
    };

    // Collect behavior signals bounded to the last TREND_TAIL_K turns.
    let all_calls: Vec<(String, u64)> = turns
        .iter()
        .flat_map(|t| t.tool_calls.iter().cloned())
        .collect();
    let error_flags: Vec<bool> = turns
        .iter()
        .flat_map(|t| t.error_flags.iter().copied())
        .collect();
    let stop_reason_max_tokens = turns.iter().any(|t| t.stop_reason_max_tokens);
    let behavior_signals =
        BehaviorSignals::from_signals(&all_calls, &error_flags, stop_reason_max_tokens);

    (Some(window_info), last_ts, trend, behavior_signals)
}

fn agent_id_from_stem(stem: &str) -> Option<String> {
    stem.strip_prefix("agent-").map(|s| s.to_string())
}

/// Discover parent sessions and sub-agents in a single encoded-cwd project directory.
pub fn discover_project(project_dir: &Path, backstop: u64) -> Vec<SessionNode> {
    let project_key = project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.strip_prefix('-').unwrap_or(n).to_string())
        .unwrap_or_default();

    let Ok(entries) = fs::read_dir(project_dir) else {
        return Vec::new();
    };

    let mut parents: Vec<SessionNode> = Vec::new();
    let mut child_map: HashMap<String, Vec<SessionNode>> = HashMap::new();

    // Collect .jsonl files, sort newest-first by mtime so the cap retains the most-recent N
    // sessions. Unreadable mtime sorts last (oldest); filename is a stable tiebreak.
    let mut jsonl_files: Vec<(std::time::SystemTime, String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str())?.to_string();
            if path.is_file() && name.ends_with(".jsonl") {
                let mtime = path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                Some((mtime, name, path))
            } else {
                None
            }
        })
        .collect();
    jsonl_files.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    jsonl_files.truncate(MAX_FILES_PER_PROJECT);

    for (_, name, path) in jsonl_files {
        let uuid = name.trim_end_matches(".jsonl").to_string();
        let (window, last_turn_at, trend, behavior_signals) = parse_transcript(&path, backstop);
        parents.push(SessionNode {
            session_uuid: uuid,
            agent_id: None,
            project_key: project_key.clone(),
            window,
            children: Vec::new(),
            last_turn_at,
            trend,
            behavior: behavior_signals,
        });
    }

    // Second pass: scan for sub-agent directories (not subject to the file cap).
    let Ok(entries2) = fs::read_dir(project_dir) else {
        return parents;
    };
    for entry in entries2.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if path.is_dir() {
            let parent_uuid = name.clone();
            let subagents_dir = path.join("subagents");
            if !subagents_dir.is_dir() {
                continue;
            }
            let Ok(sub_entries) = fs::read_dir(&subagents_dir) else {
                continue;
            };
            let mut children: Vec<SessionNode> = Vec::new();
            for sub_entry in sub_entries.flatten() {
                if children.len() >= MAX_SUBAGENTS {
                    break;
                }
                let sub_path = sub_entry.path();
                let sub_stem = sub_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !sub_path.is_file()
                    || sub_path.extension().is_none_or(|e| e != "jsonl")
                    || !sub_stem.starts_with("agent-")
                {
                    continue;
                }
                let agent_id = agent_id_from_stem(sub_stem);
                let (window, last_turn_at, trend, behavior_signals) =
                    parse_transcript(&sub_path, backstop);
                children.push(SessionNode {
                    session_uuid: parent_uuid.clone(),
                    agent_id,
                    project_key: project_key.clone(),
                    window,
                    children: Vec::new(),
                    last_turn_at,
                    trend,
                    behavior: behavior_signals,
                });
            }
            child_map.insert(parent_uuid, children);
        }
    }

    for parent in &mut parents {
        if let Some(children) = child_map.remove(&parent.session_uuid) {
            parent.children = children;
        }
    }

    parents
}

pub(crate) fn discover_sessions(projects_dir: &Path, backstop: u64) -> Vec<SessionNode> {
    let Ok(entries) = fs::read_dir(projects_dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<SessionNode> = Vec::new();
    for entry in entries.flatten() {
        if sessions.len() >= MAX_PARENT_SESSIONS {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            let project_sessions = discover_project(&path, backstop);
            let remaining = MAX_PARENT_SESSIONS - sessions.len();
            sessions.extend(project_sessions.into_iter().take(remaining));
        }
    }
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::ABSOLUTE_RECYCLE_BACKSTOP;
    use std::fs;

    // B1: saturating_add prevents overflow on adversarial huge u64 token values.
    #[test]
    fn test_window_tokens_no_overflow_huge_values() {
        let info = compute_window_info(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.window_tokens, u64::MAX);
    }

    // TEST-001: window-fill math from the last-turn oracle (142000 tokens).
    #[test]
    fn test_window_tokens_math_oracle() {
        let info = compute_window_info(
            7_000,
            130_000,
            5_000,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.window_tokens, 142_000);
    }

    #[test]
    fn test_claude_window_source_is_last_turn() {
        // Claude's point-in-time oracle always reports LastTurn provenance (ADR-002).
        let info = compute_window_info(1_000, 0, 0, "claude-sonnet-4-6", WindowSource::LastTurn);
        assert_eq!(info.window_source, WindowSource::LastTurn);
    }

    #[test]
    fn test_earlier_turns_ignored() {
        // Write a transcript with an early turn (high usage) and a later turn (low usage).
        // brim must report the LAST turn only.
        let tmp = std::env::temp_dir().join("brim_test_last_turn");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let uuid = "11112222-3333-4444-5555-666677778888";
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":180000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
            "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":7000,\"cache_read_input_tokens\":130000,\"cache_creation_input_tokens\":5000,\"output_tokens\":200}}}\n",
        );
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        let w = sessions[0].window.as_ref().unwrap();
        assert_eq!(w.window_tokens, 142_000);
        let _ = fs::remove_dir_all(&tmp);
    }

    // TEST-002: tree assembly — parent + two sub-agents, childless session, independent tokens.
    #[test]
    fn test_tree_assembly() {
        let tmp = std::env::temp_dir().join("brim_test_tree");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let parent_uuid = "aaaabbbb-cccc-dddd-eeee-111122223333";
        let parent_jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":50000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n";
        fs::write(tmp.join(format!("{parent_uuid}.jsonl")), parent_jsonl).unwrap();

        let subagents_dir = tmp.join(parent_uuid).join("subagents");
        fs::create_dir_all(&subagents_dir).unwrap();

        let agent_a = "ffff0000-1111-2222-3333-444455556666";
        let agent_a_jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":50}}}\n";
        fs::write(
            subagents_dir.join(format!("agent-{agent_a}.jsonl")),
            agent_a_jsonl,
        )
        .unwrap();

        let agent_b = "77778888-9999-aaaa-bbbb-ccccddddeeee";
        let agent_b_jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":20000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":50}}}\n";
        fs::write(
            subagents_dir.join(format!("agent-{agent_b}.jsonl")),
            agent_b_jsonl,
        )
        .unwrap();

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1, "one parent session expected");
        let parent = &sessions[0];
        assert_eq!(parent.session_uuid, parent_uuid);
        assert_eq!(parent.children.len(), 2, "two sub-agents expected");

        assert_eq!(parent.window.as_ref().unwrap().window_tokens, 50_000);

        let ca = parent
            .children
            .iter()
            .find(|c| c.agent_id.as_deref() == Some(agent_a))
            .expect("agent A");
        let cb = parent
            .children
            .iter()
            .find(|c| c.agent_id.as_deref() == Some(agent_b))
            .expect("agent B");
        assert_eq!(ca.window.as_ref().unwrap().window_tokens, 10_000);
        assert_eq!(cb.window.as_ref().unwrap().window_tokens, 20_000);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_childless_session_no_error() {
        let tmp = std::env::temp_dir().join("brim_test_childless");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let uuid = "deadbeef-dead-beef-dead-beefdeadbeef";
        let jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":1000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":10}}}\n";
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].children.len(), 0);

        let _ = fs::remove_dir_all(&tmp);
    }

    // Same UUID under two different project dirs must produce two rows with distinct project keys.
    #[test]
    fn test_same_uuid_different_projects() {
        let tmp = std::env::temp_dir().join("brim_test_multi_project");
        let _ = fs::remove_dir_all(&tmp);

        let uuid = "12345678-1234-1234-1234-123456789abc";
        let jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":50}}}\n";

        let project_a = tmp.join("-home-pol-code-gitcake");
        fs::create_dir_all(&project_a).unwrap();
        fs::write(project_a.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let project_b = tmp.join("-home-pol-code-git-task");
        fs::create_dir_all(&project_b).unwrap();
        fs::write(project_b.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let sessions = discover_sessions(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 2, "both sessions must appear (no dedup)");

        let keys: std::collections::HashSet<&str> =
            sessions.iter().map(|s| s.project_key.as_str()).collect();
        assert!(
            keys.contains("home-pol-code-gitcake"),
            "project key A missing"
        );
        assert!(
            keys.contains("home-pol-code-git-task"),
            "project key B missing"
        );
        assert!(
            sessions.iter().all(|s| s.session_uuid == uuid),
            "all rows must share the same UUID"
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_malformed_lines_skipped() {
        let tmp = std::env::temp_dir().join("brim_test_malformed");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let uuid = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        let jsonl = concat!(
            "not valid json\n",
            "{\"type\":\"human\",\"message\":\"hi\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":40000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":10}}}\n",
        );
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].window.is_some());
        assert_eq!(sessions[0].window.as_ref().unwrap().window_tokens, 40_000);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_timestamp_extracted_from_transcript() {
        let tmp = std::env::temp_dir().join("brim_test_timestamp");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let jsonl = "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":50000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n";
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].last_turn_at.is_some());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_missing_timestamp_no_panic() {
        let tmp = std::env::temp_dir().join("brim_test_no_ts");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let uuid = "11111111-2222-3333-4444-555555555555";
        let jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":50}}}\n";
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].last_turn_at.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    // B1: final assistant turn has a later timestamp but zero usage.
    // last_turn_at must be the timestamp of the window turn, not the zero-usage turn.
    #[test]
    fn test_last_turn_ts_bound_to_window_turn_not_zero_usage_turn() {
        let tmp = std::env::temp_dir().join("brim_test_b1_ts");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let uuid = "b1b1b1b1-b1b1-b1b1-b1b1-b1b1b1b1b1b1";
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":50000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T11:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":0}}}\n",
        );
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        // Must be 10:00 (the window turn), not 11:00 (zero-usage turn)
        let ts = sessions[0].last_turn_at.unwrap();
        assert_eq!(ts.to_rfc3339(), "2026-06-23T10:00:00+00:00");
        let _ = fs::remove_dir_all(&tmp);
    }

    // Trend: two turns with timestamps → trend has 2 points; velocity computed.
    // velocity = 70k - 50k = 20k; projection = (128k - 70k) / 20k = 2 (58k/20k=2)
    #[test]
    fn test_trend_built_from_multiple_turns() {
        let tmp = std::env::temp_dir().join("brim_test_trend");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let uuid = "cccccccc-dddd-eeee-ffff-000011112222";
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":50000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T10:01:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":70000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
        );
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        let trend = sessions[0].trend.as_ref().expect("trend present");
        assert_eq!(trend.points.len(), 2);
        assert_eq!(trend.velocity_tokens_per_turn, Some(20_000));
        assert_eq!(trend.projected_turns_to_recycle, Some(2));
        let _ = fs::remove_dir_all(&tmp);
    }

    // REQ-004: projection must use the configured backstop, not the hardcoded default.
    // Same trace as test_trend_built_from_multiple_turns (50k→70k, velocity=20k).
    // default backstop=128k → projection=(128k-70k)/20k=2
    // custom backstop=100k  → projection=(100k-70k)/20k=1
    #[test]
    fn test_projection_uses_configured_backstop() {
        let tmp = std::env::temp_dir().join("brim_test_proj_backstop");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let uuid = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":50000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-23T10:01:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":70000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
        );
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();

        let sessions_default = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        let proj_default = sessions_default[0]
            .trend
            .as_ref()
            .expect("trend")
            .projected_turns_to_recycle;
        assert_eq!(proj_default, Some(2));

        let sessions_custom = discover_project(&tmp, 100_000);
        let proj_custom = sessions_custom[0]
            .trend
            .as_ref()
            .expect("trend")
            .projected_turns_to_recycle;
        assert_eq!(proj_custom, Some(1));

        assert_ne!(
            proj_default, proj_custom,
            "projection must differ when backstop differs"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    // Trend: turns without timestamps are excluded from timeline points.
    #[test]
    fn test_trend_excludes_turns_without_timestamps() {
        let tmp = std::env::temp_dir().join("brim_test_trend_no_ts");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let uuid = "33334444-5555-6666-7777-888899990000";
        // Two turns but neither has a timestamp → no timeline points → trend = None
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":50000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
            "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":70000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n",
        );
        fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        // Window is still computed (last valid turn)
        assert!(sessions[0].window.is_some());
        // But trend is None (no timestamps → no timeline points)
        assert!(sessions[0].trend.is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    // TEST-005-ext: file cap retains newest-N sessions deterministically when count > cap.
    #[test]
    fn test_file_cap_retains_newest() {
        use std::fs::FileTimes;
        use std::time::{Duration, SystemTime};
        let tmp = std::env::temp_dir().join("brim_test_file_cap_newest");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let minimal_jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":1000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":10}}}\n";

        // Create MAX_FILES_PER_PROJECT + 5 files with staggered mtimes.
        // File index i=0 is oldest; i=total-1 is newest.
        let total = MAX_FILES_PER_PROJECT + 5;
        let base_time = SystemTime::now();
        for i in 0..total {
            let path = tmp.join(format!("{:05}.jsonl", i));
            fs::write(&path, minimal_jsonl).unwrap();
            let mtime = base_time - Duration::from_secs((total - i) as u64);
            let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.set_times(FileTimes::new().set_modified(mtime)).unwrap();
        }

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), MAX_FILES_PER_PROJECT, "cap not applied");

        // The 5 oldest files (indices 0..5) must be dropped; all newer ones retained.
        let uuids: std::collections::HashSet<_> =
            sessions.iter().map(|s| s.session_uuid.as_str()).collect();
        for i in 0..5usize {
            let name = format!("{:05}", i);
            assert!(
                !uuids.contains(name.as_str()),
                "old file {name} should have been dropped"
            );
        }
        for i in 5..total {
            let name = format!("{:05}", i);
            assert!(
                uuids.contains(name.as_str()),
                "new file {name} should have been retained"
            );
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    // Equal-mtime tiebreak: all files share the same mtime; tiebreak is ascending filename.
    // After cap, the first MAX_FILES_PER_PROJECT alphabetically are retained.
    // Calling discover_project twice must yield an identical set (total order → deterministic).
    #[test]
    fn test_file_cap_equal_mtime_tiebreak() {
        use std::fs::FileTimes;
        use std::time::{Duration, SystemTime};
        let tmp = std::env::temp_dir().join("brim_test_equal_mtime");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let minimal_jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":1000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":10}}}\n";

        let total = MAX_FILES_PER_PROJECT + 5;
        // A fixed mtime far from now — all files get exactly this value.
        let fixed_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for i in 0..total {
            let path = tmp.join(format!("{i:05}.jsonl"));
            fs::write(&path, minimal_jsonl).unwrap();
            let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.set_times(FileTimes::new().set_modified(fixed_mtime))
                .unwrap();
        }

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), MAX_FILES_PER_PROJECT, "cap not applied");

        // Tiebreak is ascending filename: 00000..00063 retained, 00064..00068 dropped.
        let uuids: std::collections::HashSet<_> =
            sessions.iter().map(|s| s.session_uuid.as_str()).collect();
        for i in 0..MAX_FILES_PER_PROJECT {
            let name = format!("{i:05}");
            assert!(
                uuids.contains(name.as_str()),
                "file {name} should be retained"
            );
        }
        for i in MAX_FILES_PER_PROJECT..total {
            let name = format!("{i:05}");
            assert!(
                !uuids.contains(name.as_str()),
                "file {name} should be dropped"
            );
        }

        // Stability: repeated call must return the same retained set.
        let sessions2 = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        let uuids2: std::collections::HashSet<_> =
            sessions2.iter().map(|s| s.session_uuid.as_str()).collect();
        assert_eq!(
            uuids, uuids2,
            "discover_project must be stable across calls"
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_behavior_failure_streak_claude() {
        let tmp = std::env::temp_dir().join("brim_test_streak");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let uuid = "33330000-0000-0000-0000-000000000001";
        // Three consecutive assistant turns, each with a tool_use, followed by
        // human turns with is_error=true tool_result blocks (3 consecutive failures)
        let lines = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]}}\n",
            "{\"type\":\"human\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t1\",\"is_error\":true,\"content\":\"error1\"}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:01:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"id\":\"t2\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]}}\n",
            "{\"type\":\"human\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t2\",\"is_error\":true,\"content\":\"error2\"}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:02:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"id\":\"t3\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]}}\n",
            "{\"type\":\"human\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t3\",\"is_error\":true,\"content\":\"error3\"}]}}\n",
        );
        std::fs::write(tmp.join(format!("{uuid}.jsonl")), lines).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        let streak = sessions[0].behavior.as_ref().and_then(|b| b.failure_streak);
        assert_eq!(
            streak,
            Some(3),
            "3 consecutive is_error=true → failure_streak==3"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // REAL-SHAPE: Claude sets is_error=true on tool_result when bash exits non-zero.
    // Content begins "Exit code N\n..." (LIVE-VERIFIED from ~/.claude transcripts).
    // No code change needed — is_error already reflects exit-code failure correctly.
    #[test]
    fn test_behavior_failure_streak_claude_real_bash_exit_shape() {
        let tmp = std::env::temp_dir().join("brim_test_streak_bash_exit");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let uuid = "33330000-0000-0000-0000-000000000010";
        // Real shape from ~/.claude/projects/**/*.jsonl: is_error=true, content="Exit code 1\n..."
        let lines = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"id\":\"u1\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo test\"}}]}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"u1\",\"is_error\":true,\"content\":\"Exit code 1\\nerror[E0...]: some compile error\"}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:01:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"id\":\"u2\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo test\"}}]}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"u2\",\"is_error\":true,\"content\":\"Exit code 101\\nerror: could not compile\"}]}}\n",
        );
        std::fs::write(tmp.join(format!("{uuid}.jsonl")), lines).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        let streak = sessions[0].behavior.as_ref().and_then(|b| b.failure_streak);
        assert_eq!(
            streak,
            Some(2),
            "real bash exit content with is_error=true → failure_streak fires"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_behavior_stop_reason_max_tokens() {
        let tmp = std::env::temp_dir().join("brim_test_stop_reason");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let uuid = "33330000-0000-0000-0000-000000000002";
        let jsonl = "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"stop_reason\":\"max_tokens\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}\n";
        std::fs::write(tmp.join(format!("{uuid}.jsonl")), jsonl).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        let smt = sessions[0]
            .behavior
            .as_ref()
            .map(|b| b.stop_reason_max_tokens)
            .unwrap_or(false);
        assert!(smt, "stop_reason=max_tokens → stop_reason_max_tokens==true");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_behavior_ping_pong_claude() {
        let tmp = std::env::temp_dir().join("brim_test_pingpong");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let uuid = "33330000-0000-0000-0000-000000000003";
        // A→B→A pattern: Bash(ls), Read(/foo), Bash(ls)
        let lines = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:00:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:01:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"path\":\"/foo\"}}]}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-06-25T10:02:00Z\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100},\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]}}\n",
        );
        std::fs::write(tmp.join(format!("{uuid}.jsonl")), lines).unwrap();
        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1);
        let pp = sessions[0]
            .behavior
            .as_ref()
            .and_then(|b| b.ping_pong_count);
        assert!(pp.is_some_and(|c| c >= 1), "A→B→A → ping_pong_count >= 1");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // UNIX_EPOCH fallback sorts oldest: a file whose mtime is UNIX_EPOCH is dropped first
    // when total > cap. This exercises the same sort position as the unwrap_or(UNIX_EPOCH)
    // fallback in discover_project for unreadable mtimes.
    // Note: forcing metadata() to fail on a readable file is not reliably portable on
    // Linux/macOS (the OS always returns metadata for an accessible file), so we set the
    // mtime explicitly to UNIX_EPOCH to cover the equivalent sort behavior.
    #[test]
    fn test_file_cap_unix_epoch_sorts_last() {
        use std::fs::FileTimes;
        use std::time::SystemTime;
        let tmp = std::env::temp_dir().join("brim_test_unix_epoch_sort");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let minimal_jsonl = "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":1000,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":10}}}\n";

        let recent_mtime = SystemTime::now();
        for i in 0..MAX_FILES_PER_PROJECT {
            let path = tmp.join(format!("recent-{i:05}.jsonl"));
            fs::write(&path, minimal_jsonl).unwrap();
            let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.set_times(FileTimes::new().set_modified(recent_mtime))
                .unwrap();
        }

        // One extra file with UNIX_EPOCH mtime — equivalent to unreadable mtime fallback.
        let epoch_path = tmp.join("epoch-00000.jsonl");
        fs::write(&epoch_path, minimal_jsonl).unwrap();
        let f = fs::OpenOptions::new()
            .write(true)
            .open(&epoch_path)
            .unwrap();
        f.set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();

        let sessions = discover_project(&tmp, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), MAX_FILES_PER_PROJECT, "cap not applied");

        let uuids: std::collections::HashSet<_> =
            sessions.iter().map(|s| s.session_uuid.as_str()).collect();
        assert!(
            !uuids.contains("epoch-00000"),
            "UNIX_EPOCH-mtime file must be dropped (sorts oldest)"
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
