//! Copilot provider — reads GitHub Copilot CLI session state at
//! `$HOME/.copilot/session-state/<uuid>/` (REQ-002, REQ-009).
//!
//! Point-in-time window occupancy: `CompactionProcessor: Utilization` lines in
//! `~/.copilot/logs/process-<epochMs>-<pid>.log`.  Session↔log linkage: the
//! session dir holds `inuse.<pid>.lock`; the pid matches the log filename.
//! Sessions without a live lock (ended) report window=None gracefully.
//!
//! Events.jsonl carries no point-in-time occupancy (confirmed VERIFIED-LIVE).

use crate::{
    model::{SessionNode, TimelinePoint, WindowInfo, WindowSource, WindowTrend},
    parser::{home_dir, read_tail},
    provider::Provider,
    window::{TREND_TAIL_K, compute_trend},
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
        self.state_root().exists()
    }

    fn load_sessions(&self, backstop: u64) -> Vec<SessionNode> {
        if !self.is_available() {
            return Vec::new();
        }
        collect_sessions(&self.state_root(), &self.log_root(), backstop)
    }
}

/// Walk `session-state/<uuid>/` (one level, flat), capped at MAX_SESSIONS.
fn collect_sessions(state_root: &Path, log_root: &Path, backstop: u64) -> Vec<SessionNode> {
    let Ok(entries) = std::fs::read_dir(state_root) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Some(node) = parse_session_dir(&entry.path(), log_root, backstop) {
            sessions.push(node);
        }
        if sessions.len() >= MAX_SESSIONS {
            break;
        }
    }
    sessions
}

fn parse_session_dir(dir: &Path, log_root: &Path, backstop: u64) -> Option<SessionNode> {
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

    let event_ts = if dir.join("events.jsonl").exists() {
        let tail = read_tail(&dir.join("events.jsonl")).unwrap_or_default();
        extract_event_timestamp(&tail)
    } else {
        None
    };

    // Point-in-time occupancy from process log (REQ-009).
    let (window, trend) = process_log_occupancy(dir, log_root, backstop)
        .map(|(w, t)| (Some(w), Some(t)))
        .unwrap_or((None, None));

    let last_turn_at = event_ts.or(updated_ts).or(created_ts);

    Some(SessionNode {
        session_uuid: session_id,
        agent_id: None,
        project_key,
        window,
        children: Vec::new(),
        last_turn_at,
        trend,
    })
}

/// Returns window + trend from the process log; None when lock or log is missing/malformed.
fn process_log_occupancy(
    session_dir: &Path,
    log_root: &Path,
    backstop: u64,
) -> Option<(WindowInfo, WindowTrend)> {
    let pid = find_pid_from_lock(session_dir)?;
    let log_path = find_process_log(log_root, pid)?;
    let tail = read_tail(&log_path).ok()?;
    let mut points = extract_compaction_points(&tail);
    if points.is_empty() {
        return None;
    }
    if points.len() > TREND_TAIL_K {
        points.drain(0..points.len() - TREND_TAIL_K);
    }
    let latest = points.last()?.window_tokens;
    let trend = compute_trend(points, backstop);
    let window = WindowInfo {
        window_tokens: latest,
        model: "copilot".to_string(),
        window_source: WindowSource::ProcessLog,
        cache_hit_ratio: None,
    };
    Some((window, trend))
}

/// Extract the pid from `inuse.<pid>.lock` in the session dir.
fn find_pid_from_lock(dir: &Path) -> Option<u64> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if let Some(rest) = s.strip_prefix("inuse.")
            && let Some(pid_str) = rest.strip_suffix(".lock")
            && let Ok(pid) = pid_str.parse::<u64>()
        {
            return Some(pid);
        }
    }
    None
}

/// Find the newest `process-<epochMs>-<pid>.log` in log_root for the given pid.
fn find_process_log(log_root: &Path, pid: u64) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(log_root) else {
        return None;
    };
    let suffix = format!("-{pid}.log");
    let mut best: Option<(String, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.starts_with("process-") && s.ends_with(suffix.as_str()) {
            let candidate = s.into_owned();
            let is_newer = best.as_ref().is_none_or(|(b, _)| candidate > *b);
            if is_newer {
                best = Some((candidate, entry.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Parse CompactionProcessor Utilization lines from a log tail.
/// Only lines containing the marker are inspected; all other log content is ignored.
fn extract_compaction_points(tail: &str) -> Vec<TimelinePoint> {
    const MARKER: &str = "CompactionProcessor: Utilization ";
    let mut points = Vec::new();
    for line in tail.lines() {
        if !line.contains(MARKER) {
            continue;
        }
        if let Some((at, window_tokens)) = parse_compaction_line(line) {
            points.push(TimelinePoint {
                at,
                window_tokens,
                cache_hit_ratio: None,
            });
        }
    }
    points
}

/// Parse a single CompactionProcessor Utilization line.
/// Format: `<TS> [INFO] CompactionProcessor: Utilization <PCT>% (<USED>/<LIMIT> tokens)...`
/// Extracts ONLY <TS> and <USED>; ignores PCT, LIMIT, THRESH (absolute-only, ADR-011).
fn parse_compaction_line(line: &str) -> Option<(DateTime<Utc>, u64)> {
    let space = line.find(' ')?;
    let at = chrono::DateTime::parse_from_rfc3339(&line[..space])
        .ok()?
        .with_timezone(&Utc);

    // Advance past "% (" to reach "<USED>/...".
    let pct_open = line.find("% (")?;
    let after_open = &line[pct_open + 3..]; // skip "% ("
    let slash = after_open.find('/')?;
    let used: u64 = after_open[..slash].trim().parse().ok()?;

    Some((at, used))
}

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

fn parse_yaml_timestamp(text: &str, key: &str) -> Option<DateTime<Utc>> {
    let s = parse_yaml_scalar(text, key)?;
    chrono::DateTime::parse_from_rfc3339(&s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Return the directory basename of `cwd` from workspace.yaml (VERIFIED-LIVE field).
fn extract_project_key(workspace: &str) -> String {
    if let Some(val) = parse_yaml_scalar(workspace, "cwd")
        && let Some(base) = Path::new(&val).file_name().and_then(|n| n.to_str())
    {
        return base.to_string();
    }
    String::new()
}

/// Scan events.jsonl tail for a timestamp; events carry no point-in-time occupancy.
fn extract_event_timestamp(tail: &str) -> Option<DateTime<Utc>> {
    let mut shutdown_ts: Option<DateTime<Utc>> = None;
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("session.shutdown")
            && let Some(ts) = parse_event_timestamp(&v)
        {
            shutdown_ts = Some(ts);
        }
    }
    shutdown_ts.or_else(|| find_last_event_timestamp(tail))
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
    use crate::verdict::ABSOLUTE_RECYCLE_BACKSTOP;

    // ── process-log line parsing ─────────────────────────────────────────────

    #[test]
    fn test_parse_compaction_line_ok() {
        let line = "2026-01-01T10:00:00.123Z [INFO] CompactionProcessor: Utilization 45.2% (92340/204800 tokens) below threshold 90%";
        let (at, used) = parse_compaction_line(line).expect("must parse");
        assert_eq!(used, 92340);
        assert_eq!(
            at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "2026-01-01T10:00:00.123Z"
        );
    }

    #[test]
    fn test_parse_compaction_line_limit_pct_ignored() {
        // Only USED (92340) is taken; limit (204800) and pct (45.2) are not in the output.
        let line = "2026-01-01T10:00:00.000Z [INFO] CompactionProcessor: Utilization 45.2% (92340/204800 tokens) below threshold 90%";
        let (_, used) = parse_compaction_line(line).expect("must parse");
        assert_eq!(used, 92340);
    }

    #[test]
    fn test_parse_compaction_line_malformed_skipped() {
        // Missing "% (" → None.
        assert!(parse_compaction_line("2026-01-01T10:00:00Z [INFO] no match here").is_none());
        // Bad timestamp → None.
        assert!(
            parse_compaction_line(
                "not-a-ts [INFO] CompactionProcessor: Utilization 10% (5000/50000 tokens)"
            )
            .is_none()
        );
        // Non-numeric USED → None.
        assert!(
            parse_compaction_line(
                "2026-01-01T10:00:00Z [INFO] CompactionProcessor: Utilization 10% (abc/50000 tokens)"
            )
            .is_none()
        );
    }

    #[test]
    fn test_extract_compaction_points_multi_line_trend() {
        let make = |h: u32, used: u64| {
            format!(
                "2026-01-01T{h:02}:00:00.000Z [INFO] CompactionProcessor: Utilization 10% ({used}/200000 tokens) below threshold 90%"
            )
        };
        let tail = format!(
            "{}\n{}\n{}\n",
            make(10, 10_000),
            make(11, 20_000),
            make(12, 30_000)
        );
        let points = extract_compaction_points(&tail);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].window_tokens, 10_000);
        assert_eq!(points[2].window_tokens, 30_000);

        let trend = compute_trend(points, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(trend.velocity_tokens_per_turn, Some(10_000));
        assert!(trend.projected_turns_to_recycle.is_some());
    }

    #[test]
    fn test_extract_compaction_points_non_compaction_lines_skipped() {
        let tail = "2026-01-01T10:00:00Z [INFO] SomeOtherProcessor: doing stuff\n\
                    2026-01-01T10:01:00.000Z [INFO] CompactionProcessor: Utilization 5% (5000/100000 tokens) below threshold 90%\n\
                    2026-01-01T10:02:00Z [ERROR] Something failed\n";
        let points = extract_compaction_points(tail);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].window_tokens, 5000);
    }

    // ── lock / log linkage ───────────────────────────────────────────────────

    #[test]
    fn test_process_log_occupancy_missing_lock_returns_none() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_nolock_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let log_root = tmp.join("logs");
        std::fs::create_dir_all(&log_root).unwrap();

        assert!(process_log_occupancy(&tmp, &log_root, ABSOLUTE_RECYCLE_BACKSTOP).is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_process_log_occupancy_missing_log_returns_none() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_nolog_{}", std::process::id()));
        let session_dir = tmp.join("session");
        let log_root = tmp.join("logs");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(&log_root).unwrap();

        std::fs::write(session_dir.join("inuse.9999.lock"), "").unwrap();
        // No matching process-*.log in log_root.
        assert!(
            process_log_occupancy(&session_dir, &log_root, ABSOLUTE_RECYCLE_BACKSTOP).is_none()
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    // ── extract_project_key ──────────────────────────────────────────────────

    #[test]
    fn test_extract_project_key_cwd() {
        let workspace =
            "id: abc\ncwd: /home/user/code/myproject\ncreated_at: 2026-01-01T00:00:00Z\n";
        assert_eq!(extract_project_key(workspace), "myproject");
    }

    #[test]
    fn test_extract_project_key_dead_keys_ignored() {
        // project_path / directory / workspace_path no longer matched (VERIFIED-LIVE: only cwd exists).
        let workspace = "project_path: /home/user/dead\ndirectory: /also/dead\n";
        assert_eq!(extract_project_key(workspace), "");
    }

    // ── events → timestamp; no window ───────────────────────────────────────

    #[test]
    fn test_copilot_events_window_none_shutdown_ts() {
        // events.jsonl shutdown event → timestamp extracted; window comes from process log only.
        let line = r#"{"type":"session.shutdown","timestamp":"2024-06-01T10:00:00Z","data":{}}"#;
        let ts = extract_event_timestamp(line);
        assert!(ts.is_some(), "shutdown timestamp extracted from events");
    }

    #[test]
    fn test_copilot_no_shutdown_event_ts_from_last_event() {
        let tail = "{\"type\":\"session.start\",\"timestamp\":\"2024-06-01T09:00:00Z\",\"data\":{}}\n\
                    {\"type\":\"session.model_change\",\"timestamp\":\"2024-06-01T09:05:00Z\",\"data\":{\"newModel\":\"gpt-4o\"}}";
        let ts = extract_event_timestamp(tail);
        assert!(ts.is_some(), "timestamp from last non-shutdown event");
    }

    #[test]
    fn test_copilot_malformed_event_line_skipped() {
        let good = r#"{"type":"session.shutdown","timestamp":"2024-06-01T10:00:00Z","data":{}}"#;
        let tail = format!("{{not valid json\n{good}\n");
        let ts = extract_event_timestamp(&tail);
        assert!(
            ts.is_some(),
            "valid shutdown ts extracted despite bad preceding line"
        );
    }

    // ── provider-level ───────────────────────────────────────────────────────

    #[test]
    fn test_copilot_absent_dirs_not_available() {
        let p = CopilotProvider {
            home: PathBuf::from("/nonexistent/zzz_brim_copilot_test"),
        };
        assert!(!p.is_available());
        assert!(p.load_sessions(ABSOLUTE_RECYCLE_BACKSTOP).is_empty());
    }

    #[test]
    fn test_copilot_logs_only_not_available() {
        let tmp =
            std::env::temp_dir().join(format!("brim_copilot_logsonly_{}", std::process::id()));
        let log_root = tmp.join(".copilot").join("logs");
        std::fs::create_dir_all(&log_root).unwrap();
        // session-state absent → not available despite logs/ existing
        let p = CopilotProvider { home: tmp.clone() };
        assert!(!p.is_available());
        assert!(p.load_sessions(ABSOLUTE_RECYCLE_BACKSTOP).is_empty());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_copilot_state_root_only_is_available() {
        let tmp =
            std::env::temp_dir().join(format!("brim_copilot_stateonly_{}", std::process::id()));
        let state_root = tmp.join(".copilot").join("session-state");
        std::fs::create_dir_all(&state_root).unwrap();
        // logs/ absent — must not prevent availability
        let p = CopilotProvider { home: tmp.clone() };
        assert!(p.is_available());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_copilot_parse_session_dir_round_trip() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_rt_{}", std::process::id()));
        let log_root = tmp.join("logs");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir_all(&log_root).unwrap();

        std::fs::write(
            tmp.join("workspace.yaml"),
            "id: copilot-session-abc\ncwd: /home/user/code/myproject\ncreated_at: 2024-06-01T09:00:00Z\nupdated_at: 2024-06-01T10:00:00Z\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("events.jsonl"),
            "{\"type\":\"session.start\",\"timestamp\":\"2024-06-01T09:00:00Z\",\"data\":{}}\n",
        )
        .unwrap();

        // No inuse.*.lock → window=None from process log path.
        let node = parse_session_dir(&tmp, &log_root, ABSOLUTE_RECYCLE_BACKSTOP).expect("node");
        assert_eq!(node.session_uuid, "copilot-session-abc");
        assert_eq!(node.project_key, "myproject");
        assert!(node.agent_id.is_none());
        assert!(node.children.is_empty());
        assert!(node.window.is_none(), "no lock → window=None");
        assert!(node.last_turn_at.is_some(), "timestamp propagated");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_copilot_collect_sessions_directory_walk() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_walk_{}", std::process::id()));
        let state_root = tmp.join(".copilot").join("session-state");
        let log_root = tmp.join(".copilot").join("logs");
        let session_dir = state_root.join("walk-session-uuid");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(&log_root).unwrap();

        std::fs::write(session_dir.join("workspace.yaml"), "id: walk-session-1\n").unwrap();
        std::fs::write(
            session_dir.join("events.jsonl"),
            "{\"type\":\"session.shutdown\",\"timestamp\":\"2024-06-01T10:00:00Z\",\"data\":{}}\n",
        )
        .unwrap();

        // Non-directory entry in session-state must be skipped.
        std::fs::write(state_root.join("ignore_me.txt"), "not a dir").unwrap();

        let provider = CopilotProvider { home: tmp.clone() };
        assert!(provider.is_available());
        let sessions = provider.load_sessions(ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(sessions.len(), 1, "exactly one session discovered");
        assert_eq!(sessions[0].session_uuid, "walk-session-1");
        assert!(sessions[0].agent_id.is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }

    // End-to-end: process log → window + trend.
    #[test]
    fn test_process_log_produces_window_and_trend() {
        let tmp = std::env::temp_dir().join(format!("brim_copilot_e2e_{}", std::process::id()));
        let session_dir = tmp.join("session");
        let log_root = tmp.join("logs");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(&log_root).unwrap();

        let pid: u64 = 54321;
        std::fs::write(session_dir.join(format!("inuse.{pid}.lock")), "").unwrap();

        let log_content = "\
2026-01-01T10:00:00.000Z [INFO] CompactionProcessor: Utilization 10% (10000/100000 tokens) below threshold 90%\n\
2026-01-01T10:01:00.000Z [INFO] CompactionProcessor: Utilization 20% (20000/100000 tokens) below threshold 90%\n\
2026-01-01T10:02:00.000Z [INFO] CompactionProcessor: Utilization 30% (30000/100000 tokens) below threshold 90%\n";
        std::fs::write(
            log_root.join(format!("process-1234567890000-{pid}.log")),
            log_content,
        )
        .unwrap();

        std::fs::write(
            session_dir.join("workspace.yaml"),
            "id: e2e-session\ncwd: /some/project\n",
        )
        .unwrap();

        let node =
            parse_session_dir(&session_dir, &log_root, ABSOLUTE_RECYCLE_BACKSTOP).expect("node");
        assert_eq!(node.session_uuid, "e2e-session");
        assert_eq!(node.project_key, "project");

        let w = node.window.expect("window from process log");
        assert_eq!(w.window_tokens, 30_000);
        assert_eq!(w.window_source, WindowSource::ProcessLog);
        assert!(w.cache_hit_ratio.is_none());

        let t = node.trend.expect("trend from process log");
        assert_eq!(t.velocity_tokens_per_turn, Some(10_000));
        assert!(t.projected_turns_to_recycle.is_some());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
