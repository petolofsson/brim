//! Copilot provider — reads GitHub Copilot CLI session state at
//! `$HOME/.copilot/session-state/<uuid>/` (see REQ-002).
//!
//! Token oracle: the `session.shutdown` event's
//! `data.modelMetrics.<currentModel>.usage` block carries session-CUMULATIVE
//! totals (input, cacheRead, cacheWrite), NOT a per-turn/last-request snapshot.
//! `cacheReadTokens` re-counts the cached prefix on every turn, so feeding these
//! totals into `compute_window_info` would report cumulative spend as window
//! occupancy — exactly what ADR-002 rejects.
//!
//! **brim reports no Copilot fill % or recycle verdict by design (ADR-002).**
//! A true per-turn snapshot counter would be required to restore fill reporting.
//! Sessions are still listed with session_id, project, and last-activity timestamp.

use crate::{
    model::{SessionNode, WindowInfo},
    parser::{home_dir, read_tail},
    provider::Provider,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Max sessions enumerated (CODERULES r3).
const MAX_SESSIONS: usize = 256;

pub struct CopilotProvider {
    pub home: PathBuf,
}

impl CopilotProvider {
    pub fn new() -> Self {
        Self { home: home_dir() }
    }

    fn state_root(&self) -> PathBuf {
        self.home.join(".copilot").join("session-state")
    }

    fn log_root(&self) -> PathBuf {
        self.home.join(".copilot").join("logs")
    }
}

impl Provider for CopilotProvider {
    fn is_available(&self) -> bool {
        self.state_root().exists() || self.log_root().exists()
    }

    fn load_sessions(&self) -> Vec<SessionNode> {
        if !self.is_available() {
            return Vec::new();
        }
        let state_root = self.state_root();
        if !state_root.exists() {
            return Vec::new();
        }
        collect_sessions(&state_root)
    }
}

/// Walk `session-state/<uuid>/` (one level, flat), capped at MAX_SESSIONS.
fn collect_sessions(state_root: &Path) -> Vec<SessionNode> {
    let Ok(entries) = std::fs::read_dir(state_root) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Some(node) = parse_session_dir(&entry.path()) {
            sessions.push(node);
        }
        if sessions.len() >= MAX_SESSIONS {
            break;
        }
    }
    sessions
}

fn parse_session_dir(dir: &Path) -> Option<SessionNode> {
    // Bounded read: workspace.yaml is tiny; read_tail caps at 256 KB (no OOM).
    let workspace_path = dir.join("workspace.yaml");
    let workspace = read_tail(&workspace_path).unwrap_or_default();

    let session_id = parse_yaml_scalar(&workspace, "id")
        .or_else(|| {
            dir.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    let project_key = extract_project_key(&workspace);
    let created_ts = parse_yaml_timestamp(&workspace, "created_at");
    let updated_ts = parse_yaml_timestamp(&workspace, "updated_at");

    let events_path = dir.join("events.jsonl");
    let (window, event_ts) = if events_path.exists() {
        let tail = read_tail(&events_path).unwrap_or_default();
        extract_window_from_events(&tail)
    } else {
        (None, None)
    };

    // Prefer shutdown-event timestamp, then updated_at, then created_at.
    let last_turn_at = event_ts.or(updated_ts).or(created_ts);

    Some(SessionNode {
        session_uuid: session_id,
        agent_id: None,
        project_key,
        window,
        children: Vec::new(),
        last_turn_at,
        trend: None,
    })
}

/// Extract a single-line scalar `key: value` from YAML text.
/// Strips surrounding single or double quotes from the value.
fn parse_yaml_scalar(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Parse a YAML scalar as an RFC3339 timestamp.
fn parse_yaml_timestamp(text: &str, key: &str) -> Option<DateTime<Utc>> {
    let s = parse_yaml_scalar(text, key)?;
    chrono::DateTime::parse_from_rfc3339(&s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Try common project-path fields in workspace.yaml; return the directory basename.
fn extract_project_key(workspace: &str) -> String {
    for field in &["project_path", "directory", "workspace_path", "cwd"] {
        if let Some(val) = parse_yaml_scalar(workspace, field)
            && let Some(base) = Path::new(&val).file_name().and_then(|n| n.to_str())
        {
            return base.to_string();
        }
    }
    String::new()
}

/// Scan events for a `session.shutdown` timestamp; always returns `window = None`.
///
/// Copilot's token counters are session-cumulative accumulators — not window
/// occupancy — so no fill % is computed.  See module doc and ADR-002.
fn extract_window_from_events(tail: &str) -> (Option<WindowInfo>, Option<DateTime<Utc>>) {
    let mut last_shutdown_ts: Option<DateTime<Utc>> = None;

    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("session.shutdown") {
            continue;
        }
        if let Some(ts) = parse_event_timestamp(&v) {
            last_shutdown_ts = Some(ts);
        }
    }

    let ts = last_shutdown_ts.or_else(|| find_last_event_timestamp(tail));
    (None, ts)
}

fn parse_event_timestamp(v: &Value) -> Option<DateTime<Utc>> {
    let s = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("time").and_then(|x| x.as_str()))?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Scan from the tail end to find the last event that carries a timestamp.
fn find_last_event_timestamp(tail: &str) -> Option<DateTime<Utc>> {
    for line in tail.lines().rev().take(50) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(ts) = parse_event_timestamp(&v) {
            return Some(ts);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a synthetic `session.shutdown` event JSON line.
    fn make_shutdown_event(
        input: u64,
        cache_read: u64,
        cache_write: u64,
        model: &str,
        ts: &str,
    ) -> String {
        let usage = serde_json::json!({
            "inputTokens": input,
            "outputTokens": 0u64,
            "cacheReadTokens": cache_read,
            "cacheWriteTokens": cache_write,
            "reasoningTokens": 0u64,
        });
        let mut model_metrics = serde_json::Map::new();
        model_metrics.insert(model.to_string(), serde_json::json!({"usage": usage}));
        let model_metrics_val = serde_json::Value::Object(model_metrics);
        serde_json::json!({
            "type": "session.shutdown",
            "timestamp": ts,
            "data": {
                "currentModel": model,
                "modelMetrics": model_metrics_val,
            }
        })
        .to_string()
    }

    // TEST-003 (a): shutdown event => window = None (cumulative totals, ADR-002).
    // Session is still discoverable via the returned timestamp.
    #[test]
    fn test_copilot_shutdown_window_none() {
        let line = make_shutdown_event(20_000, 5_000, 3_000, "gpt-4o", "2024-06-01T10:00:00Z");
        let (window, ts) = extract_window_from_events(&line);
        assert!(window.is_none(), "no fill % for cumulative Copilot totals");
        assert!(ts.is_some(), "shutdown timestamp still extracted");
    }

    // Shutdown with all-zero usage => window still None, timestamp extracted.
    #[test]
    fn test_copilot_all_zero_usage_window_none() {
        let line = make_shutdown_event(0, 0, 0, "gpt-4o", "2024-06-01T10:00:00Z");
        let (window, ts) = extract_window_from_events(&line);
        assert!(window.is_none());
        assert!(ts.is_some());
    }

    // No shutdown event => window None, timestamp from last event in tail.
    #[test]
    fn test_copilot_no_shutdown_window_none() {
        let tail = r#"{"type":"session.start","timestamp":"2024-06-01T09:00:00Z","data":{}}
{"type":"session.model_change","timestamp":"2024-06-01T09:05:00Z","data":{"newModel":"gpt-4o"}}"#;
        let (window, ts) = extract_window_from_events(tail);
        assert!(window.is_none());
        assert!(ts.is_some(), "timestamp from last non-shutdown event");
    }

    // TEST-003 (b): malformed JSONL line is skipped, valid shutdown still sets ts.
    #[test]
    fn test_copilot_malformed_line_skipped() {
        let good = make_shutdown_event(40_000, 0, 0, "gpt-4o", "2024-06-01T10:00:00Z");
        let tail = format!("{{not valid json\n{good}\n");
        let (window, ts) = extract_window_from_events(&tail);
        assert!(window.is_none());
        assert!(
            ts.is_some(),
            "valid shutdown timestamp extracted despite earlier bad line"
        );
    }

    // TEST-003 (c): both dirs absent => is_available()==false.
    #[test]
    fn test_copilot_absent_dirs_not_available() {
        let p = CopilotProvider {
            home: PathBuf::from("/nonexistent/zzz_brim_copilot_test"),
        };
        assert!(!p.is_available());
        assert!(p.load_sessions().is_empty());
    }

    // Round-trip: write a synthetic session dir, parse end-to-end via parse_session_dir.
    // window = None (ADR-002); session_uuid, project_key, last_turn_at all present.
    #[test]
    fn test_copilot_parse_session_dir_round_trip() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_rt_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        std::fs::write(
            tmp.join("workspace.yaml"),
            "id: copilot-session-abc\nproject_path: /home/user/code/myproject\ncreated_at: 2024-06-01T09:00:00Z\nupdated_at: 2024-06-01T10:00:00Z\n",
        )
        .unwrap();

        let mut f = std::fs::File::create(tmp.join("events.jsonl")).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::json!({"type": "session.start", "data": {"sessionId": "copilot-session-abc"}, "timestamp": "2024-06-01T09:00:00Z"})
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            make_shutdown_event(30_000, 10_000, 5_000, "gpt-4o", "2024-06-01T10:00:00Z")
        )
        .unwrap();
        drop(f);

        let node = parse_session_dir(&tmp).expect("node");
        assert_eq!(node.session_uuid, "copilot-session-abc");
        assert_eq!(node.project_key, "myproject");
        assert!(node.agent_id.is_none());
        assert!(node.children.is_empty());
        assert!(
            node.window.is_none(),
            "no fill % — cumulative tokens, ADR-002"
        );
        assert!(node.last_turn_at.is_some(), "shutdown timestamp propagated");

        std::fs::remove_dir_all(&tmp).ok();
    }

    // N4 (d): build a temp ~/.copilot tree, assert load_sessions finds the session
    // as a flat node with agent_id=None. A non-directory entry must be ignored.
    #[test]
    fn test_copilot_collect_sessions_directory_walk() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_walk_{}", std::process::id()));
        let state_root = tmp.join(".copilot").join("session-state");
        let session_dir = state_root.join("walk-session-uuid");
        std::fs::create_dir_all(&session_dir).unwrap();

        std::fs::write(session_dir.join("workspace.yaml"), "id: walk-session-1\n").unwrap();

        let mut f = std::fs::File::create(session_dir.join("events.jsonl")).unwrap();
        writeln!(
            f,
            "{}",
            make_shutdown_event(10_000, 0, 0, "gpt-4o", "2024-06-01T10:00:00Z")
        )
        .unwrap();
        drop(f);

        // Non-directory entry in session-state must be skipped.
        std::fs::write(state_root.join("ignore_me.txt"), "not a dir").unwrap();

        let provider = CopilotProvider { home: tmp.clone() };
        assert!(provider.is_available());
        let sessions = provider.load_sessions();
        assert_eq!(sessions.len(), 1, "exactly one session discovered");
        assert_eq!(sessions[0].session_uuid, "walk-session-1");
        assert!(sessions[0].agent_id.is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
