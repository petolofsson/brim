//! opencode provider — reads the SQLite transcript DB at
//! `$HOME/.local/share/opencode/opencode.db` (see REQ-008 / ADR-005).
//!
//! Last-turn oracle: the latest `part` row with
//! `json_extract(data,'$.type')='step-finish'`; its `data.tokens` carries
//! `{ input, cache: { read, write } }` — the opencode analogue of claude's
//! `input_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens`.
//!
//! If no `step-finish` part exists for a session (pre-checkpoint), brim falls
//! back to the `session` aggregate token columns and tags the node with
//! `window_source = "aggregate"` (ADR-002's "approximate or unavailable" case).

use crate::{
    model::{SessionNode, WindowInfo, WindowSource},
    parser::home_dir,
    provider::Provider,
    window::compute_window_info,
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

    fn load_sessions_inner(&self) -> Vec<SessionNode> {
        let conn = match self.open() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        discover_sessions(&conn)
    }
}

impl Provider for OpencodeProvider {
    fn is_available(&self) -> bool {
        self.db_path().exists()
    }

    fn load_sessions(&self) -> Vec<SessionNode> {
        self.load_sessions_inner()
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
/// 1. Look up the latest `step-finish` part (point-in-time oracle, ADR-005).
/// 2. If found, compute `WindowInfo` from its `data.tokens` (LastTurn).
/// 3. Else fall back to `session` aggregate columns (Aggregate).
pub fn discover_sessions(conn: &Connection) -> Vec<SessionNode> {
    let mut stmt = match conn.prepare(
        "SELECT id, parent_id, agent, model, directory, project_id,
                tokens_input, tokens_cache_read, tokens_cache_write, time_updated
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
                time_updated_ms: r.get::<_, i64>(9)?,
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
        let (window, last_turn_at) = latest_step_finish_window(conn, &row.id, &model)
            .unwrap_or_else(|| {
                (
                    aggregate_window(&row, &model),
                    ts_from_ms(row.time_updated_ms),
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

/// Latest `step-finish` part for a session → `(WindowInfo, last_turn_at)`.
/// Returns `None` if no step-finish part exists or it has no token data.
fn latest_step_finish_window(
    conn: &Connection,
    session_id: &str,
    model: &str,
) -> Option<(Option<WindowInfo>, Option<DateTime<Utc>>)> {
    let data = conn
        .query_row(
            "SELECT data FROM part
             WHERE session_id = ?1
               AND json_extract(data, '$.type') = 'step-finish'
             ORDER BY time_created DESC
             LIMIT 1",
            params![session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()?;

    let v: Value = serde_json::from_str(&data).ok()?;
    let tokens = v.get("tokens")?;
    let input = tokens.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
    let cache = tokens.get("cache")?;
    let cache_read = cache.get("read").and_then(|x| x.as_u64()).unwrap_or(0);
    let cache_write = cache.get("write").and_then(|x| x.as_u64()).unwrap_or(0);

    // Time: prefer `time` in the part data, else the part row's time_created.
    // We reuse the part `time_created` via a second query only if needed; the
    // spec says the part row's time_created is authoritative, but the data JSON
    // also carries a `time` field. Use data.time first, fall back to row column.
    let time_ms = v.get("time").and_then(|t| t.as_i64()).or_else(|| {
        conn.query_row(
            "SELECT time_created FROM part
                 WHERE session_id = ?1
                   AND json_extract(data, '$.type') = 'step-finish'
                 ORDER BY time_created DESC LIMIT 1",
            params![session_id],
            |r| r.get::<_, i64>(0),
        )
        .ok()
    });

    if input == 0 && cache_read == 0 && cache_write == 0 {
        return Some((None, ts_from_ms_option(time_ms)));
    }

    let info = compute_window_info(
        input,
        cache_read,
        cache_write,
        model,
        WindowSource::LastTurn,
    );
    Some((Some(info), ts_from_ms_option(time_ms)))
}

/// Fallback: build an Aggregate WindowInfo from the session's cumulative columns.
fn aggregate_window(row: &SessionRow, model: &str) -> Option<WindowInfo> {
    let input = row.tokens_input.max(0) as u64;
    let cache_read = row.tokens_cache_read.max(0) as u64;
    let cache_write = row.tokens_cache_write.max(0) as u64;
    if input == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }
    Some(compute_window_info(
        input,
        cache_read,
        cache_write,
        model,
        WindowSource::Aggregate,
    ))
}

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
    use rusqlite::Connection;

    /// Open an in-memory sqlite db with the opencode schema seed the tests need.
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

    // TEST-004 case 1: step-finish oracle — input=106, cache.read=46720, cache.write=0
    // → window_tokens=46826, fill=round(46826/200000*100)=23.
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

        let nodes = discover_sessions(&conn);
        assert_eq!(nodes.len(), 1);
        let w = nodes[0].window.as_ref().expect("window present");
        assert_eq!(w.window_tokens, 46_826);
        assert_eq!(w.fill_percent, 23);
        assert_eq!(w.context_limit, 200_000);
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
                                  time_updated)
             VALUES (?1, ?2, ?3, NULL, 5000, 30000, 0, 1719000000000)",
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

        let nodes = discover_sessions(&conn);
        assert_eq!(nodes.len(), 1);
        let w = nodes[0].window.as_ref().expect("window");
        // 5000 + 30000 + 0 = 35000 → round(35000/200000*100) = round(17.5) = 18
        assert_eq!(w.window_tokens, 35_000);
        assert_eq!(w.fill_percent, 18);
        assert_eq!(w.window_source, WindowSource::Aggregate);
    }

    // TEST-004 case 3: parent_id sub-agent tree (current install is empty, but
    // the join must work structurally when opencode starts spawning sub-agents).
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

        let nodes = discover_sessions(&conn);
        assert_eq!(
            nodes.len(),
            1,
            "one root (parent) returned; child is nested"
        );
        let parent = &nodes[0];
        assert_eq!(parent.session_uuid, "parent_ses");
        assert_eq!(parent.children.len(), 1);
        // Claude SessionNode convention: child.session_uuid = parent id, child.agent_id = child's own id.
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
        let nodes = discover_sessions(&conn);
        assert_eq!(nodes[0].project_key, "brim");

        // Now a session with no project row — must fall back to directory basename.
        conn.execute(
            "INSERT INTO session (id, model, directory, project_id, time_updated)
             VALUES ('ses_delta', ?1, '/home/pol/code/other', NULL, 1719000000000)",
            params![model_json("z-ai/glm-5.2")],
        )
        .unwrap();
        let nodes2 = discover_sessions(&conn);
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
        let nodes = discover_sessions(&conn);
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
        assert!(p.load_sessions().is_empty());
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
        let nodes = discover_sessions(&conn);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].window.is_none());
    }
}
