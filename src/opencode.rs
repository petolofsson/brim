//! opencode provider — reads the SQLite transcript DB at
//! `$HOME/.local/share/opencode/opencode.db` (see REQ-008 / ADR-005).
//!
//! Last-turn oracle: prefers `session_message` rows where `type='step-finish'`
//! (new opencode schema, native `type`+`seq` columns); falls back to the old
//! `part` table query (`json_extract(data,'$.type')='step-finish'`) for older
//! DBs. Step-finish `data.tokens` carries
//! `{ total?, input, output?, cache: { read, write } }`.
//! Window occupancy: prefer `tokens.total` when present; else sum
//! `input + output + cache.read + cache.write`.
//!
//! If no `step-finish` row exists for a session (pre-checkpoint), brim falls
//! back to the `session` aggregate token columns and tags the node with
//! `window_source = "aggregate"` (ADR-002's "approximate or unavailable" case).

use crate::{
    model::{SessionNode, TimelinePoint, WindowInfo, WindowSource, WindowTrend},
    parser::home_dir,
    provider::Provider,
    window::{TREND_TAIL_K, compute_trend, compute_window_info},
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use std::path::PathBuf;

/// Max sessions enumerated (CODERULES r3).
const MAX_SESSIONS: usize = 256;
/// Max children joined per parent (CODERULES r2-3).
const MAX_CHILDREN: usize = 64;

pub struct OpencodeProvider {
    pub home: PathBuf,
}

impl OpencodeProvider {
    pub fn new() -> Self {
        Self { home: home_dir() }
    }

    fn db_path(&self) -> PathBuf {
        self.home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db")
    }

    /// Open the opencode SQLite DB read-only (CODERULES r11). WAL is tolerated
    /// natively — rusqlite reads through `-wal`/`-shm` without checkpoint.
    fn open(&self) -> Result<Connection> {
        let path = self.db_path();
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // Best-effort: enforce read-only at the pragma layer too. Ignore errors
        // (some sqlite builds reject query_only on read-only handles).
        let _ = conn.pragma_update(None, "query_only", true);
        Ok(conn)
    }

    fn load_sessions_inner(&self, backstop: u64) -> Vec<SessionNode> {
        let conn = match self.open() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        discover_sessions(&conn, backstop)
    }
}

impl Provider for OpencodeProvider {
    fn is_available(&self) -> bool {
        self.db_path().exists()
    }

    fn load_sessions(&self, backstop: u64) -> Vec<SessionNode> {
        self.load_sessions_inner(backstop)
    }
}

/// One row of the session table with the columns brim needs.
struct SessionRow {
    id: String,
    parent_id: Option<String>,
    /// opencode `session.agent` (agent type). Currently unused for output; kept
    /// for future agent-type reporting (FEATURE-001 § future providers).
    #[allow(dead_code)]
    agent: Option<String>,
    model_json: Option<String>,
    directory: Option<String>,
    project_id: Option<String>,
    tokens_input: i64,
    tokens_cache_read: i64,
    tokens_cache_write: i64,
    tokens_output: i64,
    time_updated_ms: i64,
}

/// Parse the opencode `session.model` JSON for the model `.id`.
/// `{"id":"z-ai/glm-5.2","providerID":"llmbase"}` → `"z-ai/glm-5.2"`.
/// Falls back to the raw string if it isn't JSON, or "" if null.
fn parse_model_id(model_json: Option<&str>) -> String {
    let Some(s) = model_json else {
        return String::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(s) else {
        return s.to_string();
    };
    v.get("id")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string()
}

/// Resolve the project key: prefer `project.name`, else `session.directory` basename.
fn project_key(conn: &Connection, project_id: Option<&str>, directory: Option<&str>) -> String {
    if let Some(pid) = project_id {
        let name: Option<String> = conn
            .query_row(
                "SELECT name FROM project WHERE id = ?1",
                params![pid],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        if let Some(n) = name {
            return n;
        }
    }
    directory
        .and_then(|d| {
            std::path::Path::new(d)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// Discover all opencode sessions and assemble parent → child trees via `parent_id`.
///
/// For each session:
/// 1. Query last TREND_TAIL_K step-finish parts (point-in-time oracle, ADR-005/ADR-006).
/// 2. If found, build WindowInfo + WindowTrend from their `data.tokens` (LastTurn).
/// 3. Else fall back to `session` aggregate columns (Aggregate), trend = None.
pub fn discover_sessions(conn: &Connection, backstop: u64) -> Vec<SessionNode> {
    let mut stmt = match conn.prepare(
        "SELECT id, parent_id, agent, model, directory, project_id,
                tokens_input, tokens_cache_read, tokens_cache_write, tokens_output, time_updated
         FROM session
         ORDER BY time_updated DESC
         LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt
        .query_map(params![MAX_SESSIONS as i64], |r| {
            Ok(SessionRow {
                id: r.get::<_, String>(0)?,
                parent_id: r.get::<_, Option<String>>(1)?,
                agent: r.get::<_, Option<String>>(2)?,
                model_json: r.get::<_, Option<String>>(3)?,
                directory: r.get::<_, Option<String>>(4)?,
                project_id: r.get::<_, Option<String>>(5)?,
                tokens_input: r.get::<_, i64>(6)?,
                tokens_cache_read: r.get::<_, i64>(7)?,
                tokens_cache_write: r.get::<_, i64>(8)?,
                tokens_output: r.get::<_, i64>(9)?,
                time_updated_ms: r.get::<_, i64>(10)?,
            })
        })
        .ok();

    let Some(rows) = rows else {
        return Vec::new();
    };

    let mut raw: Vec<SessionRow> = Vec::new();
    for row in rows.flatten() {
        if raw.len() >= MAX_SESSIONS {
            break;
        }
        raw.push(row);
    }

    // First pass: build a node per row, recording window + project key + ts.
    // Parent/child links resolved in a second pass by parent_id.
    let mut nodes: Vec<(SessionRow, SessionNode)> = Vec::with_capacity(raw.len());
    for row in raw {
        let model = parse_model_id(row.model_json.as_deref());
        let pkey = project_key(conn, row.project_id.as_deref(), row.directory.as_deref());
        let (window, last_turn_at, trend) = step_finish_oracle(conn, &row.id, &model, backstop)
            .unwrap_or_else(|| {
                (
                    aggregate_window(&row, &model),
                    ts_from_ms(row.time_updated_ms),
                    None,
                )
            });
        nodes.push((
            row,
            SessionNode {
                session_uuid: String::new(), // filled in second pass
                agent_id: None,
                project_key: pkey,
                window,
                children: Vec::new(),
                last_turn_at,
                trend,
                behavior: None, // opencode SQLite source does not carry tool_use parts in the queried part types; Behavior family cannot currently fire for this provider.
            },
        ));
    }

    // First pass id assignment.
    for (row, node) in nodes.iter_mut() {
        node.session_uuid = row.id.clone();
    }
    let id_of: std::collections::HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, (r, _))| (r.id.clone(), i))
        .collect();

    // Build child buckets and mark which indices became children of an in-set
    // parent (those are not emitted as roots). Per claude's SessionNode
    // convention: a child's `session_uuid` is the parent's id and its
    // `agent_id` is the child's own session id (the sub-agent identifier).
    let mut child_buckets: Vec<Vec<SessionNode>> = vec![Vec::new(); nodes.len()];
    let mut is_child = vec![false; nodes.len()];
    for (i, (row, node)) in nodes.iter().enumerate() {
        if let Some(pid) = &row.parent_id
            && let Some(&pidx) = id_of.get(pid)
            && child_buckets[pidx].len() < MAX_CHILDREN
        {
            let parent_id = pid.clone();
            let child_id = row.id.clone();
            let mut child = node.clone();
            child.session_uuid = parent_id;
            child.agent_id = Some(child_id);
            child_buckets[pidx].push(child);
            is_child[i] = true;
        }
    }

    // Roots = nodes that are not children of an in-set parent.
    let mut roots: Vec<SessionNode> = Vec::new();
    for (i, (_, mut node)) in nodes.into_iter().enumerate() {
        if is_child[i] {
            continue;
        }
        node.children = std::mem::take(&mut child_buckets[i]);
        roots.push(node);
    }
    roots
}

/// Query the last TREND_TAIL_K step-finish parts for a session and build the
/// window + trend.  Returns None when no step-finish parts exist (triggers
/// aggregate fallback in the caller).
///
/// Occupancy: prefer `tokens.total` when present; else sum
/// `input + output + cache.read + cache.write`.
fn step_finish_oracle(
    conn: &Connection,
    session_id: &str,
    model: &str,
    backstop: u64,
) -> StepFinishResult {
    let raw = fetch_step_finish_rows(conn, session_id);

    if raw.is_empty() {
        return None;
    }

    // Reverse DESC → chronological order (oldest first).
    let rows: Vec<_> = raw.into_iter().rev().collect();

    let mut timeline_points: Vec<TimelinePoint> = Vec::new();
    let mut last_window: Option<WindowInfo> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;

    for (data_opt, row_ts_ms) in &rows {
        let data = match data_opt {
            Some(d) if !d.is_empty() => d.as_str(),
            _ => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };

        let tokens = v.get("tokens");
        let total_opt = tokens.and_then(|t| t.get("total")).and_then(|x| x.as_u64());
        let input = tokens
            .and_then(|t| t.get("input"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let output = tokens
            .and_then(|t| t.get("output"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cache = tokens.and_then(|t| t.get("cache"));
        let cache_read = cache
            .and_then(|c| c.get("read"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cache_write = cache
            .and_then(|c| c.get("write"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);

        // Prefer tokens.total; else sum all active-token contributors.
        let window_tokens = total_opt.unwrap_or_else(|| {
            input
                .saturating_add(output)
                .saturating_add(cache_read)
                .saturating_add(cache_write)
        });

        let time_ms = v.get("time").and_then(|t| t.as_i64()).or(Some(*row_ts_ms));
        let ts = ts_from_ms_option(time_ms);

        if window_tokens == 0 {
            if let Some(t) = ts {
                last_ts = Some(t);
            }
            continue;
        }

        let cache_hit_ratio = if cache_read > 0 || cache_write > 0 {
            Some((cache_read as f32 / window_tokens as f32).clamp(0.0, 1.0))
        } else {
            None
        };

        let info = WindowInfo {
            window_tokens,
            model: model.to_string(),
            window_source: WindowSource::LastTurn,
            cache_hit_ratio,
        };

        if let Some(at) = ts {
            timeline_points.push(TimelinePoint {
                at,
                window_tokens: info.window_tokens,
                cache_hit_ratio: info.cache_hit_ratio,
            });
            last_ts = Some(at);
        }

        last_window = Some(info);
    }

    let trend = if !timeline_points.is_empty() {
        Some(compute_trend(timeline_points, backstop))
    } else {
        None
    };

    Some((last_window, last_ts, trend))
}

/// Fetch step-finish rows for a session: try `session_message` (new schema),
/// fall back to old `part` table. Both queries return DESC; caller reverses.
fn fetch_step_finish_rows(conn: &Connection, session_id: &str) -> Vec<(Option<String>, i64)> {
    if let Some(rows) = try_session_message(conn, session_id)
        && !rows.is_empty()
    {
        return rows;
    }
    try_part(conn, session_id).unwrap_or_default()
}

/// Query `session_message` for step-finish rows (new opencode schema).
/// Returns None when the table is absent (prepare fails → treated as "not available").
fn try_session_message(conn: &Connection, session_id: &str) -> Option<Vec<(Option<String>, i64)>> {
    let mut stmt = conn
        .prepare(
            "SELECT data, time_created FROM session_message
             WHERE session_id = ?1 AND type = 'step-finish'
             ORDER BY seq DESC LIMIT ?2",
        )
        .ok()?;
    let rows: Vec<_> = stmt
        .query_map(params![session_id, TREND_TAIL_K as i64], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    Some(rows)
}

/// Query old `part` table for step-finish rows (older opencode schema).
/// Returns None when the table is absent.
fn try_part(conn: &Connection, session_id: &str) -> Option<Vec<(Option<String>, i64)>> {
    let mut stmt = conn
        .prepare(
            "SELECT data, time_created FROM part
             WHERE session_id = ?1
               AND json_extract(data, '$.type') = 'step-finish'
             ORDER BY time_created DESC LIMIT ?2",
        )
        .ok()?;
    let rows: Vec<_> = stmt
        .query_map(params![session_id, TREND_TAIL_K as i64], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    Some(rows)
}

/// Fallback: build an Aggregate WindowInfo from the session's cumulative columns.
fn aggregate_window(row: &SessionRow, model: &str) -> Option<WindowInfo> {
    let input = row.tokens_input.max(0) as u64;
    let cache_read = row.tokens_cache_read.max(0) as u64;
    let cache_write = row.tokens_cache_write.max(0) as u64;
    let output = row.tokens_output.max(0) as u64;
    if input == 0 && cache_read == 0 && cache_write == 0 && output == 0 {
        return None;
    }
    let mut w = compute_window_info(
        input,
        cache_read,
        cache_write,
        model,
        WindowSource::Aggregate,
    );
    w.window_tokens = w.window_tokens.saturating_add(output);
    Some(w)
}

/// Return type for `step_finish_oracle`: window, timestamp, trend.
type StepFinishResult = Option<(
    Option<WindowInfo>,
    Option<DateTime<Utc>>,
    Option<WindowTrend>,
)>;

fn ts_from_ms_option(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(ts_from_ms)
}

fn ts_from_ms(ms: i64) -> Option<DateTime<Utc>> {
    if ms <= 0 {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::ABSOLUTE_RECYCLE_BACKSTOP;
    use rusqlite::Connection;

    /// Open an in-memory sqlite db with the opencode schema seed the tests need.
    /// Includes both `session_message` (new) and `part` (old) tables.
    fn seed_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                agent TEXT,
                model TEXT,
                directory TEXT,
                project_id TEXT,
                tokens_input INTEGER DEFAULT 0,
                tokens_cache_read INTEGER DEFAULT 0,
                tokens_cache_write INTEGER DEFAULT 0,
                tokens_output INTEGER DEFAULT 0,
                tokens_reasoning INTEGER DEFAULT 0,
                cost REAL DEFAULT 0,
                time_created INTEGER DEFAULT 0,
                time_updated INTEGER DEFAULT 0
            );
            CREATE TABLE session_message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                type TEXT,
                seq INTEGER,
                time_created INTEGER DEFAULT 0,
                time_updated INTEGER DEFAULT 0,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                type TEXT,
                time_created INTEGER DEFAULT 0,
                data TEXT
            );
            CREATE TABLE project (
                id TEXT PRIMARY KEY,
                name TEXT,
                worktree TEXT
            );",
        )
        .unwrap();
        conn
    }

    /// Like seed_db() but without session_message — simulates old opencode schema.
    fn seed_db_old_schema() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                agent TEXT,
                model TEXT,
                directory TEXT,
                project_id TEXT,
                tokens_input INTEGER DEFAULT 0,
                tokens_cache_read INTEGER DEFAULT 0,
                tokens_cache_write INTEGER DEFAULT 0,
                tokens_output INTEGER DEFAULT 0,
                tokens_reasoning INTEGER DEFAULT 0,
                cost REAL DEFAULT 0,
                time_created INTEGER DEFAULT 0,
                time_updated INTEGER DEFAULT 0
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                type TEXT,
                time_created INTEGER DEFAULT 0,
                data TEXT
            );
            CREATE TABLE project (
                id TEXT PRIMARY KEY,
                name TEXT,
                worktree TEXT
            );",
        )
        .unwrap();
        conn
    }

    fn model_json(id: &str) -> String {
        format!("{{\"id\":\"{id}\",\"providerID\":\"llmbase\"}}")
    }

    // TEST-004 case 1: step-finish oracle — prefers tokens.total=46826.
    #[test]
    fn test_opencode_step_finish_window() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES (?1, ?2, ?3, NULL, 1719000000000)",
            params![
                "ses_alpha",
                model_json("z-ai/glm-5.2"),
                "/home/pol/code/brim"
            ],
        )
        .unwrap();
        let part_data = serde_json::json!({
            "type": "step-finish",
            "time": 1719000000000_i64,
            "tokens": {
                "total": 46826u64,
                "input": 106u64,
                "output": 0u64,
                "reasoning": 0u64,
                "cache": { "write": 0u64, "read": 46720u64 }
            }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('p1','ses_alpha','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();

        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(nodes.len(), 1);
        let w = nodes[0].window.as_ref().expect("window present");
        assert_eq!(w.window_tokens, 46_826);
        assert_eq!(w.window_source, WindowSource::LastTurn);
        assert!(nodes[0].last_turn_at.is_some());
    }

    // TEST-004 case 2: no step-finish → aggregate fallback, window_source=Aggregate.
    #[test]
    fn test_opencode_aggregate_fallback() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id,
                                  tokens_input, tokens_cache_read, tokens_cache_write,
                                  tokens_output, time_updated)
             VALUES (?1, ?2, ?3, NULL, 5000, 30000, 0, 2000, 1719000000000)",
            params![
                "ses_beta",
                model_json("z-ai/glm-5.2"),
                "/home/pol/code/brim"
            ],
        )
        .unwrap();
        // A non-step-finish part — must not be picked up by the oracle.
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('p2','ses_beta','text',1719000000000,'{\"type\":\"text\"}')",
            [],
        )
        .unwrap();

        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(nodes.len(), 1);
        let w = nodes[0].window.as_ref().expect("window");
        // 5000 + 30000 + 0 (cache_write) + 2000 (output) = 37000
        assert_eq!(w.window_tokens, 37_000);
        assert_eq!(w.window_source, WindowSource::Aggregate);
        // Aggregate fallback → no trend
        assert!(nodes[0].trend.is_none());
    }

    // Aggregate path includes tokens_output in occupancy.
    #[test]
    fn test_opencode_aggregate_output_counted() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id,
                                  tokens_input, tokens_cache_read, tokens_cache_write,
                                  tokens_output, time_updated)
             VALUES (?1, ?2, ?3, NULL, 1000, 0, 0, 500, 1719000000001)",
            params![
                "ses_gamma",
                model_json("z-ai/glm-5.2"),
                "/home/pol/code/brim"
            ],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(nodes.len(), 1);
        let w = nodes[0].window.as_ref().expect("window");
        // input=1000 + output=500 = 1500
        assert_eq!(w.window_tokens, 1_500);
        assert_eq!(w.window_source, WindowSource::Aggregate);
    }

    // TEST-004 case 3: parent_id sub-agent tree.
    #[test]
    fn test_opencode_parent_id_subagent_tree() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, parent_id, model, directory, project_id, time_updated)
             VALUES ('parent_ses', NULL, ?1, '/home/pol/code/brim', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, parent_id, agent, model, directory, project_id, time_updated)
             VALUES ('child_ses', 'parent_ses', 'explore', ?1, '/home/pol/code/brim', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();

        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(
            nodes.len(),
            1,
            "one root (parent) returned; child is nested"
        );
        let parent = &nodes[0];
        assert_eq!(parent.session_uuid, "parent_ses");
        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].session_uuid, "parent_ses");
        assert_eq!(
            parent.children[0].agent_id.as_deref(),
            Some("child_ses"),
            "child's own session id is its agent_id",
        );
    }

    // TEST-004 case 4: project.name resolution, falling back to directory basename.
    #[test]
    fn test_opencode_project_key_prefers_project_name() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO project (id, name, worktree) VALUES ('pid1','brim','/home/pol/code/brim')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_gamma', ?1, '/home/pol/code/brim', 'pid1', 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(nodes[0].project_key, "brim");

        // Now a session with no project row — must fall back to directory basename.
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_delta', ?1, '/home/pol/code/other', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        let nodes2 = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let delta = nodes2
            .iter()
            .find(|n| n.session_uuid == "ses_delta")
            .unwrap();
        assert_eq!(delta.project_key, "other");
    }

    #[test]
    fn test_opencode_step_finish_preferred_over_aggregate() {
        let conn = seed_db();
        // Aggregate columns would give a big window, but step-finish must win.
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id,
                                  tokens_input, tokens_cache_read, tokens_cache_write,
                                  time_updated)
             VALUES ('ses_eps', ?1, '/x', NULL, 99999, 99999, 99999, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        // No total → falls back to input+output+cache.read+cache.write = 106+0+46720+0 = 46826
        let part_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": { "input": 106u64, "cache": { "read": 46720u64, "write": 0u64 } }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('p3','ses_eps','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let w = nodes[0].window.as_ref().unwrap();
        assert_eq!(
            w.window_tokens, 46_826,
            "step-finish must override aggregate"
        );
        assert_eq!(w.window_source, WindowSource::LastTurn);
    }

    #[test]
    fn test_opencode_is_available_nonexistent_db() {
        let p = OpencodeProvider {
            home: PathBuf::from("/nonexistent/zzz"),
        };
        assert!(!p.is_available());
        // load_sessions must not panic when the db is missing.
        assert!(p.load_sessions(ABSOLUTE_RECYCLE_BACKSTOP).is_empty());
    }

    #[test]
    fn test_opencode_no_token_data_emits_null_window() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_zeta', ?1, '/x', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].window.is_none());
    }

    // ADR-006: two step-finish parts → trend with velocity.
    // velocity = 70k - 50k = 20k; projection to 128k backstop = (128k-70k)/20k = 2
    #[test]
    fn test_opencode_trend_from_multiple_step_finish() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_trend', ?1, '/x', NULL, 1719000002000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();

        // Part 1: 50k tokens (no total → sum = 50000)
        let p1 = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": { "input": 50000u64, "cache": { "read": 0u64, "write": 0u64 } }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('tp1','ses_trend','step-finish',1719000000000,?1)",
            params![p1],
        )
        .unwrap();

        // Part 2: 70k tokens (no total → sum = 70000)
        let p2 = serde_json::json!({
            "type": "step-finish", "time": 1719000002000_i64,
            "tokens": { "input": 70000u64, "cache": { "read": 0u64, "write": 0u64 } }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('tp2','ses_trend','step-finish',1719000002000,?1)",
            params![p2],
        )
        .unwrap();

        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        assert_eq!(nodes.len(), 1);
        let trend = nodes[0].trend.as_ref().expect("trend present");
        assert_eq!(trend.points.len(), 2);
        assert_eq!(trend.velocity_tokens_per_turn, Some(20_000));
        assert_eq!(trend.projected_turns_to_recycle, Some(2));
    }

    // Occupancy normalization: tokens.total preferred over individual fields.
    #[test]
    fn test_opencode_total_preferred_over_sum() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_total', ?1, '/x', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        // total=46826 should win over input(106)+output(0)+cache.read(46720)+cache.write(0)=46826
        // (same value here, but total takes precedence by code path)
        let part_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": {
                "total": 46826u64,
                "input": 106u64,
                "output": 0u64,
                "cache": { "read": 46720u64, "write": 0u64 }
            }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('pt1','ses_total','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let w = nodes[0].window.as_ref().unwrap();
        assert_eq!(w.window_tokens, 46_826);
    }

    // Occupancy normalization: without total, output tokens are included.
    #[test]
    fn test_opencode_output_included_when_no_total() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_out', ?1, '/x', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        // No total → window = input(100) + output(50) + cache.read(0) + cache.write(0) = 150
        let part_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": {
                "input": 100u64,
                "output": 50u64,
                "cache": { "read": 0u64, "write": 0u64 }
            }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('po1','ses_out','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let w = nodes[0].window.as_ref().unwrap();
        assert_eq!(w.window_tokens, 150);
    }

    // session_message preferred over part when both have step-finish rows.
    #[test]
    fn test_session_message_preferred_over_part() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_sm', ?1, '/x', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        // session_message row: 30000 tokens
        let sm_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": { "total": 30000u64 }
        })
        .to_string();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, seq, time_created, data)
             VALUES ('sm1','ses_sm','step-finish',1,1719000000000,?1)",
            params![sm_data],
        )
        .unwrap();
        // part row with different token count (99999) — must NOT win
        let part_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": { "total": 99999u64 }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('pm1','ses_sm','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let w = nodes[0].window.as_ref().unwrap();
        assert_eq!(
            w.window_tokens, 30_000,
            "session_message must win over part"
        );
        assert_eq!(w.window_source, WindowSource::LastTurn);
    }

    // Falls back to part table when session_message is absent (old opencode schema).
    #[test]
    fn test_falls_back_to_part_when_no_session_message() {
        let conn = seed_db_old_schema();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_old', ?1, '/x', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        let part_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": { "total": 55000u64 }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('po_old','ses_old','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let w = nodes[0].window.as_ref().unwrap();
        assert_eq!(w.window_tokens, 55_000, "part fallback must fire");
        assert_eq!(w.window_source, WindowSource::LastTurn);
    }

    // session_message table present but has no step-finish rows for this session
    // → must fall through to part.
    #[test]
    fn test_falls_back_to_part_when_session_message_empty() {
        let conn = seed_db();
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_sm_empty', ?1, '/x', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        // No session_message rows for this session; part has a step-finish row.
        let part_data = serde_json::json!({
            "type": "step-finish", "time": 1719000000000_i64,
            "tokens": { "total": 42000u64 }
        })
        .to_string();
        conn.execute(
            "INSERT INTO part (id, session_id, type, time_created, data)
             VALUES ('pe1','ses_sm_empty','step-finish',1719000000000,?1)",
            params![part_data],
        )
        .unwrap();
        let nodes = discover_sessions(&conn, ABSOLUTE_RECYCLE_BACKSTOP);
        let w = nodes[0].window.as_ref().unwrap();
        assert_eq!(
            w.window_tokens, 42_000,
            "part must be read when session_message table is present but empty for session"
        );
        assert_eq!(w.window_source, WindowSource::LastTurn);
    }
}
