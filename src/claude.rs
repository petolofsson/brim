use crate::{
    model::{SessionNode, WindowInfo, WindowSource},
    parser::{home_dir, read_tail},
    provider::Provider,
    window::compute_window_info,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
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

    fn load_sessions(&self) -> Vec<SessionNode> {
        discover_sessions(&self.projects_dir())
    }
}

/// Scan the tail of a JSONL transcript and return the last-turn WindowInfo and its timestamp.
fn parse_last_turn(path: &Path) -> (Option<WindowInfo>, Option<DateTime<Utc>>) {
    let text = match read_tail(path) {
        Ok(t) => t,
        Err(_) => return (None, None),
    };
    let mut result: Option<WindowInfo> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }

        // Parse timestamp locally; only bind to last_ts on the window turn (B1).
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

        if let Some(ts) = ts_opt {
            last_ts = Some(ts);
        }
        let model = msg
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        result = Some(compute_window_info(
            input,
            cache_read,
            cache_create,
            &model,
            WindowSource::LastTurn,
        ));
    }

    (result, last_ts)
}

fn agent_id_from_stem(stem: &str) -> Option<String> {
    stem.strip_prefix("agent-").map(|s| s.to_string())
}

/// Discover parent sessions and sub-agents in a single encoded-cwd project directory.
pub fn discover_project(project_dir: &Path) -> Vec<SessionNode> {
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
    let mut file_count: usize = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if path.is_file() && name.ends_with(".jsonl") {
            if file_count >= MAX_FILES_PER_PROJECT {
                continue;
            }
            file_count += 1;
            let uuid = name.trim_end_matches(".jsonl").to_string();
            let (window, last_turn_at) = parse_last_turn(&path);
            parents.push(SessionNode {
                session_uuid: uuid,
                agent_id: None,
                project_key: project_key.clone(),
                window,
                children: Vec::new(),
                last_turn_at,
            });
        } else if path.is_dir() {
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
                let (window, last_turn_at) = parse_last_turn(&sub_path);
                children.push(SessionNode {
                    session_uuid: parent_uuid.clone(),
                    agent_id,
                    project_key: project_key.clone(),
                    window,
                    children: Vec::new(),
                    last_turn_at,
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

pub(crate) fn discover_sessions(projects_dir: &Path) -> Vec<SessionNode> {
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
            let project_sessions = discover_project(&path);
            let remaining = MAX_PARENT_SESSIONS - sessions.len();
            sessions.extend(project_sessions.into_iter().take(remaining));
        }
    }
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // saturating_add clamps window_tokens to u64::MAX; fill must be bounded to 100.
        assert_eq!(info.fill_percent, 100);
    }

    // TEST-001: window-fill math from the last-turn oracle (142000 → 71%), bounded [0,100].
    #[test]
    fn test_window_fill_math_oracle() {
        let info = compute_window_info(
            7_000,
            130_000,
            5_000,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.window_tokens, 142_000);
        assert_eq!(info.fill_percent, 71);
    }

    #[test]
    fn test_window_fill_bounded_at_100() {
        let info = compute_window_info(
            200_000,
            100_000,
            50_000,
            "claude-sonnet-4-6",
            WindowSource::LastTurn,
        );
        assert_eq!(info.fill_percent, 100);
    }

    #[test]
    fn test_window_limit_1m_model() {
        // 500k/1M = 50% — verifies the [1m] model limit is applied correctly
        let info =
            compute_window_info(500_000, 0, 0, "claude-opus-4-8[1m]", WindowSource::LastTurn);
        assert_eq!(info.fill_percent, 50);
        assert_eq!(info.window_tokens, 500_000);
    }

    #[test]
    fn test_context_limit_stored() {
        let info = compute_window_info(100_000, 0, 0, "claude-sonnet-4-6", WindowSource::LastTurn);
        assert_eq!(info.context_limit, 200_000);
        let info_1m =
            compute_window_info(100_000, 0, 0, "claude-opus-4-8[1m]", WindowSource::LastTurn);
        assert_eq!(info_1m.context_limit, 1_000_000);
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
        let sessions = discover_project(&tmp);
        assert_eq!(sessions.len(), 1);
        let w = sessions[0].window.as_ref().unwrap();
        assert_eq!(w.window_tokens, 142_000);
        assert_eq!(w.fill_percent, 71);
        let _ = fs::remove_dir_all(&tmp);
    }

    // TEST-002: tree assembly — parent + two sub-agents, childless session, independent fills.
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

        let sessions = discover_project(&tmp);
        assert_eq!(sessions.len(), 1, "one parent session expected");
        let parent = &sessions[0];
        assert_eq!(parent.session_uuid, parent_uuid);
        assert_eq!(parent.children.len(), 2, "two sub-agents expected");

        // Independent fills
        assert_eq!(parent.window.as_ref().unwrap().fill_percent, 25); // 50000/200000 = 25%

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
        assert_eq!(ca.window.as_ref().unwrap().fill_percent, 5); // 10000/200000 = 5%
        assert_eq!(cb.window.as_ref().unwrap().fill_percent, 10); // 20000/200000 = 10%

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

        let sessions = discover_project(&tmp);
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

        let sessions = discover_sessions(&tmp);
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

        let sessions = discover_project(&tmp);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].window.is_some());
        assert_eq!(sessions[0].window.as_ref().unwrap().fill_percent, 20); // 40000/200000 = 20%

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

        let sessions = discover_project(&tmp);
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

        let sessions = discover_project(&tmp);
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
        let sessions = discover_project(&tmp);
        assert_eq!(sessions.len(), 1);
        // Must be 10:00 (the window turn), not 11:00 (zero-usage turn)
        let ts = sessions[0].last_turn_at.unwrap();
        assert_eq!(ts.to_rfc3339(), "2026-06-23T10:00:00+00:00");
        let _ = fs::remove_dir_all(&tmp);
    }
}
