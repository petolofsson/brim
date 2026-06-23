//! codex provider — reads JSONL session transcripts at
//! `$HOME/.codex/sessions/YYYY/MM/DD/*.jsonl` (see REQ-002).
//!
//! Last-turn oracle: the final two `payload.type == "token_count"` lines from
//! the file tail; their delta gives last-turn tokens:
//! `input_tokens` (includes cached), `cached_input_tokens`, `cache_creation_input_tokens`.
//! Pure input = `input_tokens − cached_input_tokens`; that plus the two cache
//! fields feeds `compute_window_info`.
//!
//! Single-event sessions (first and only turn) use absolute values tagged
//! `WindowSource::Aggregate`. Zero-event sessions emit no window.
//!
//! Every session is rendered FLAT (no sub-agent tree, `agent_id = None`)
//! per REQ-002 / REQ-003 / FEATURE-001.

use crate::{
    model::{SessionNode, WindowInfo, WindowSource},
    parser::{home_dir, read_tail},
    provider::Provider,
    window::compute_window_info,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Max sessions enumerated (CODERULES r3).
const MAX_SESSIONS: usize = 256;

pub struct CodexProvider {
    pub home: PathBuf,
}

impl CodexProvider {
    pub fn new() -> Self {
        Self { home: home_dir() }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.home.join(".codex").join("sessions")
    }
}

impl Provider for CodexProvider {
    fn is_available(&self) -> bool {
        self.sessions_dir().exists()
    }

    fn load_sessions(&self) -> Vec<SessionNode> {
        if !self.is_available() {
            return Vec::new();
        }
        collect_sessions(&self.sessions_dir())
    }
}

/// Walk `sessions_dir/YYYY/MM/DD/*.jsonl` (depth 3, capped at MAX_SESSIONS).
fn collect_sessions(sessions_dir: &Path) -> Vec<SessionNode> {
    let mut files: Vec<PathBuf> = Vec::new();
    let Ok(years) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    'outer: for year_entry in years.flatten() {
        let Ok(months) = std::fs::read_dir(year_entry.path()) else {
            continue;
        };
        for month_entry in months.flatten() {
            let Ok(days) = std::fs::read_dir(month_entry.path()) else {
                continue;
            };
            for day_entry in days.flatten() {
                let Ok(sessions) = std::fs::read_dir(day_entry.path()) else {
                    continue;
                };
                for session_entry in sessions.flatten() {
                    let p = session_entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        files.push(p);
                        if files.len() >= MAX_SESSIONS {
                            break 'outer;
                        }
                    }
                }
            }
        }
    }
    files.iter().filter_map(|f| parse_session_file(f)).collect()
}

fn parse_session_file(path: &Path) -> Option<SessionNode> {
    // Known limitations for >256KB sessions: the final turn-span may be
    // truncated; cwd and the full-span token header live before the tail window.
    let tail = read_tail(path).ok()?;
    let session_id = extract_session_id(path, &tail);
    let model = extract_model(&tail);
    let project_key = extract_project_key(&tail);
    let (window, last_turn_at) = extract_window(&tail, &model);
    Some(SessionNode {
        session_uuid: session_id,
        agent_id: None,
        project_key,
        window,
        children: Vec::new(),
        last_turn_at,
    })
}

fn extract_session_id(path: &Path, tail: &str) -> String {
    for line in tail.lines().rev().take(50) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(id) = session_id_from_value(&v) {
            return id.to_string();
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn session_id_from_value(v: &Value) -> Option<&str> {
    v.get("session_id")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("sessionId").and_then(|x| x.as_str()))
        .or_else(|| {
            v.get("payload")
                .and_then(|p| p.get("session_id"))
                .and_then(|x| x.as_str())
        })
        .or_else(|| {
            v.get("payload")
                .and_then(|p| p.get("sessionId"))
                .and_then(|x| x.as_str())
        })
}

fn extract_model(tail: &str) -> String {
    for line in tail.lines().rev().take(100) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let m = v
            .get("model")
            .and_then(|x| x.as_str())
            .or_else(|| {
                v.get("payload")
                    .and_then(|p| p.get("model"))
                    .and_then(|x| x.as_str())
            })
            .or_else(|| {
                v.get("payload")
                    .and_then(|p| p.get("response"))
                    .and_then(|r| r.get("model"))
                    .and_then(|x| x.as_str())
            });
        if let Some(m) = m
            && !m.is_empty()
            && m != "-"
        {
            return m.to_string();
        }
    }
    String::new()
}

/// Try `cwd`, `project_path`, `directory` in early lines for a directory basename.
fn extract_project_key(tail: &str) -> String {
    for line in tail.lines().take(20) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for field in &["cwd", "project_path", "directory"] {
            if let Some(dir) = v.get(field).and_then(|x| x.as_str())
                && let Some(base) = Path::new(dir).file_name().and_then(|n| n.to_str())
            {
                return base.to_string();
            }
        }
    }
    String::new()
}

/// Token counts parsed from a `total_token_usage` object.
#[derive(Default, Clone)]
struct TokenCounts {
    /// Total input including cached hits (Codex JSONL semantics).
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_creation_input_tokens: u64,
}

impl TokenCounts {
    fn all_zero(&self) -> bool {
        self.input_tokens == 0
            && self.cached_input_tokens == 0
            && self.cache_creation_input_tokens == 0
    }
}

fn parse_token_counts(v: &Value) -> TokenCounts {
    TokenCounts {
        input_tokens: v.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        cached_input_tokens: v
            .get("cached_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: v
            .get("cache_creation_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
    }
}

/// Extract last-turn window from the tail's `token_count` events.
///
/// With ≥2 events: delta of last two → `WindowSource::LastTurn`.
/// With exactly 1: absolute values → `WindowSource::Aggregate`.
/// With 0: `(None, None)`.
fn extract_window(tail: &str, model: &str) -> (Option<WindowInfo>, Option<DateTime<Utc>>) {
    let mut events: Vec<(TokenCounts, Option<DateTime<Utc>>)> = Vec::new();
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(payload) = v.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        let Some(total) = payload.get("info").and_then(|i| i.get("total_token_usage")) else {
            continue;
        };
        events.push((parse_token_counts(total), parse_timestamp(&v)));
    }

    if events.is_empty() {
        return (None, None);
    }

    let (counts, ts, source) = if events.len() >= 2 {
        let (last, last_ts) = events.last().unwrap();
        let (prev, _) = &events[events.len() - 2];
        let delta = TokenCounts {
            input_tokens: last.input_tokens.saturating_sub(prev.input_tokens),
            cached_input_tokens: last
                .cached_input_tokens
                .saturating_sub(prev.cached_input_tokens),
            cache_creation_input_tokens: last
                .cache_creation_input_tokens
                .saturating_sub(prev.cache_creation_input_tokens),
        };
        if delta.all_zero() && !last.all_zero() {
            // Stalled turn: delta is zero but session is live; degrade to absolute.
            (last.clone(), *last_ts, WindowSource::Aggregate)
        } else {
            (delta, *last_ts, WindowSource::LastTurn)
        }
    } else {
        let (counts, ts) = &events[0];
        (counts.clone(), *ts, WindowSource::Aggregate)
    };

    if counts.all_zero() {
        return (None, ts);
    }

    // `input_tokens` includes cached hits; subtract to get pure uncached input.
    // window_tokens = pure_input + cache_read + cache_create
    //               = (input_tokens − cached) + cached + cache_create
    //               = input_tokens + cache_create
    let pure_input = counts
        .input_tokens
        .saturating_sub(counts.cached_input_tokens);
    let info = compute_window_info(
        pure_input,
        counts.cached_input_tokens,
        counts.cache_creation_input_tokens,
        model,
        source,
    );
    (Some(info), ts)
}

fn parse_timestamp(v: &Value) -> Option<DateTime<Utc>> {
    let s = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("time").and_then(|x| x.as_str()))
        .or_else(|| v.get("created_at").and_then(|x| x.as_str()))
        .or_else(|| {
            v.get("payload")
                .and_then(|p| p.get("timestamp"))
                .and_then(|x| x.as_str())
        })?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WindowSource;
    use std::io::Write;

    fn make_token_count_line(input: u64, cached: u64, cache_create: u64, ts: &str) -> String {
        serde_json::json!({
            "timestamp": ts,
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "output_tokens": 0u64,
                        "cached_input_tokens": cached,
                        "cache_creation_input_tokens": cache_create
                    }
                }
            }
        })
        .to_string()
    }

    // TEST-003 (a): two token_count events → last-turn delta → correct WindowInfo.
    // Turn 1 cumulative: input=1200, cached=800, cache_create=800.
    // Turn 2 cumulative: input=1400, cached=800, cache_create=800.
    // Delta: input_delta=200, cached_delta=0, cache_create_delta=0.
    // pure_input=200, window_tokens=200+0+0=200.
    // fill = round(200/200000*100) = 0 (rounds to 0 for <0.5%).
    // Use bigger numbers to get a non-zero fill:
    // Turn 1: input=100000, cached=80000, cache_create=10000.
    // Turn 2: input=120000, cached=80000, cache_create=10000.
    // Delta: input_delta=20000, cached_delta=0, cache_create_delta=0.
    // pure_input=20000, window_tokens=20000.
    // fill=round(20000/200000*100)=10.
    #[test]
    fn test_codex_last_turn_delta_window() {
        let line1 = make_token_count_line(100_000, 80_000, 10_000, "2024-06-01T10:00:00Z");
        let line2 = make_token_count_line(120_000, 80_000, 10_000, "2024-06-01T10:01:00Z");
        let tail = format!("{line1}\n{line2}\n");

        let (window, ts) = extract_window(&tail, "codex");
        let w = window.expect("window present");
        // pure_input = 120000-80000 - (100000-80000) = 40000 - 20000 = 20000? no...
        // delta.input_tokens = 120000-100000=20000, delta.cached=0, delta.cache_create=0
        // pure_input = 20000 - 0 = 20000
        // window_tokens = 20000 + 0 + 0 = 20000
        assert_eq!(w.window_tokens, 20_000);
        assert_eq!(w.fill_percent, 10);
        assert_eq!(w.window_source, WindowSource::LastTurn);
        assert!(ts.is_some());
    }

    // TEST-003 (a) alt: single event → Aggregate, uses absolute values.
    // input=50000, cached=30000, cache_create=5000.
    // pure_input=50000-30000=20000, window=20000+30000+5000=55000.
    // fill=round(55000/200000*100)=round(27.5)=28.
    #[test]
    fn test_codex_single_event_aggregate() {
        let line = make_token_count_line(50_000, 30_000, 5_000, "2024-06-01T10:00:00Z");
        let (window, _) = extract_window(&line, "codex");
        let w = window.expect("window present");
        assert_eq!(w.window_tokens, 55_000);
        assert_eq!(w.fill_percent, 28);
        assert_eq!(w.window_source, WindowSource::Aggregate);
    }

    // TEST-003 (b): malformed JSONL line is skipped, valid line still parsed.
    #[test]
    fn test_codex_malformed_line_skipped() {
        let good = make_token_count_line(40_000, 0, 0, "2024-06-01T10:00:00Z");
        let tail = format!("{{not valid json\n{good}\n");
        let (window, _) = extract_window(&tail, "codex");
        // malformed line silently skipped; good line parsed → Aggregate (1 event)
        let w = window.expect("window present");
        assert_eq!(w.window_tokens, 40_000);
        assert_eq!(w.window_source, WindowSource::Aggregate);
    }

    // TEST-003 (c): absent sessions dir → is_available() == false.
    #[test]
    fn test_codex_absent_dir_not_available() {
        let p = CodexProvider {
            home: PathBuf::from("/nonexistent/zzz_brim_codex_test"),
        };
        assert!(!p.is_available());
        assert!(p.load_sessions().is_empty());
    }

    // Round-trip: write a synthetic JSONL to a temp path and parse it end-to-end.
    #[test]
    fn test_codex_parse_session_file_round_trip() {
        let file_path = std::env::temp_dir().join("brim_codex_test_session.jsonl");
        let mut f = std::fs::File::create(&file_path).unwrap();
        // Session metadata line.
        let meta = serde_json::json!({
            "session_id": "abc-session-1",
            "model": "gpt-4o",
            "cwd": "/home/user/code/myproject"
        });
        writeln!(f, "{meta}").unwrap();
        // First turn.
        writeln!(
            f,
            "{}",
            make_token_count_line(10_000, 0, 8_000, "2024-06-01T10:00:00Z")
        )
        .unwrap();
        // Second turn: adds 5000 new input, reads 8000 from cache, no new cache.
        writeln!(
            f,
            "{}",
            make_token_count_line(23_000, 8_000, 8_000, "2024-06-01T10:01:00Z")
        )
        .unwrap();
        drop(f);

        let node = parse_session_file(&file_path).expect("node");
        assert_eq!(node.session_uuid, "abc-session-1");
        assert_eq!(node.project_key, "myproject");
        assert!(node.agent_id.is_none());
        assert!(node.children.is_empty());
        // Delta: input_delta=13000, cached_delta=8000, cache_create_delta=0.
        // pure_input=13000-8000=5000, window=5000+8000+0=13000.
        // fill=round(13000/200000*100)=round(6.5)=7.
        let w = node.window.expect("window present");
        assert_eq!(w.window_tokens, 13_000);
        assert_eq!(w.fill_percent, 7);
        assert_eq!(w.window_source, WindowSource::LastTurn);
    }

    // N4 (a): collect_sessions / directory walk — builds a temp YYYY/MM/DD tree,
    // writes one valid .jsonl session + one non-.jsonl file (must be ignored),
    // and asserts load_sessions returns exactly the one flat node.
    #[test]
    fn test_codex_collect_sessions_directory_walk() {
        let tmp = std::env::temp_dir().join(format!("brim_codex_walk_{}", std::process::id()));
        let day_dir = tmp
            .join(".codex")
            .join("sessions")
            .join("2024")
            .join("06")
            .join("01");
        std::fs::create_dir_all(&day_dir).unwrap();

        let jsonl_path = day_dir.join("walk-session.jsonl");
        let mut f = std::fs::File::create(&jsonl_path).unwrap();
        writeln!(f, "{}", serde_json::json!({"session_id": "walk-session-1"})).unwrap();
        writeln!(
            f,
            "{}",
            make_token_count_line(10_000, 0, 0, "2024-06-01T10:00:00Z")
        )
        .unwrap();
        drop(f);

        std::fs::write(day_dir.join("ignore_me.txt"), "not jsonl").unwrap();

        let provider = CodexProvider { home: tmp.clone() };
        assert!(provider.is_available());
        let sessions = provider.load_sessions();
        assert_eq!(sessions.len(), 1, "exactly one session discovered");
        assert_eq!(sessions[0].session_uuid, "walk-session-1");
        assert!(sessions[0].agent_id.is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }

    // N4 (b): >=3 token_count events — window must equal delta of the LAST two,
    // not the first two or the full cumulative span.
    #[test]
    fn test_codex_three_events_delta_of_last_two() {
        // Cumulative events; last-turn delta = event3 - event2 = 1000, not 4000.
        let line1 = make_token_count_line(1_000, 0, 0, "2024-06-01T10:00:00Z");
        let line2 = make_token_count_line(5_000, 0, 0, "2024-06-01T10:01:00Z");
        let line3 = make_token_count_line(6_000, 0, 0, "2024-06-01T10:02:00Z");
        let tail = format!("{line1}\n{line2}\n{line3}\n");

        let (window, _) = extract_window(&tail, "codex");
        let w = window.expect("window present");
        assert_eq!(w.window_tokens, 1_000);
        assert_eq!(w.window_source, WindowSource::LastTurn);
    }
}
