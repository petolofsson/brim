---
id: REQ-008
title: opencode SQLite transcript source
status: Accepted
related_requirements:
  - REQ-002
  - REQ-005
related_adrs:
  - ADR-002
  - ADR-005
related_stories: [STORY-009]
related_tests:
  - TEST-004
---

# REQ-008 - opencode SQLite transcript source

## Requirement

* The system shall discover opencode sessions by opening, strictly read-only,
  the SQLite database at `$HOME/.local/share/opencode/opencode.db` and reading
  the `session` table. (Open via `rusqlite` with `SQLITE_OPEN_READ_ONLY`; wal
  / shm are tolerated natively — never copied or checkpointed.)
* The system shall mark the opencode provider unavailable (not error) when the
  database file is absent; availability is a cheap path check on the file only.
* The system shall compute the point-in-time last-turn window from the latest
  `part` row for the session where `json_extract(data,'$.type')='step-finish'`,
  ordered by `time_created DESC LIMIT 1` (SQL-side bound; no full part scan in
  Rust). From that part's `data.tokens`:
  * `input` maps to `input_tokens`,
  * `cache.read` maps to `cache_read_input_tokens`,
  * `cache.write` maps to `cache_creation_input_tokens`,
  * `window_tokens = input + cache.read + cache.write` (saturating add).
* The system shall fall back, when no `step-finish` part exists for a session,
  to the `session` aggregate columns `tokens_input + tokens_cache_read +
  tokens_cache_write`, and tag the resulting `WindowInfo` with
  `window_source = "aggregate"`.
* The system shall expose `window_source` (`"last_turn"` | `"aggregate"`) in the
  `--json` output so a consumer can distinguish point-in-time from cumulative
  occupancy (REQ-005 machine-readable; ADR-002 explicitly warns cumulative ≠
  window).
* The system shall resolve the project key from `project.name` for the
  session's `project_id`; if that is null or the project row is absent, it shall
  fall back to the basename of `session.directory`.
* The system shall assemble parent → child session trees using `session.parent_id`
  as the join key. Each child node's `session_uuid` is the parent's id and its
  `agent_id` is the child's own session id, mirroring claude's `SessionNode`
  convention. No children are synthesized.
* `last_turn_at` for a session shall use the latest step-finish part's
  `time`/`time_created` when present, else `session.time_updated`. The same 30-min
  default / `--active-mins` / `--all` semantics apply as for claude (REQ-006).
* Model context-window limit: `z-ai/glm-5.2` (and any opencode model id without
  the `[1m]` marker) uses the existing 200_000 default. No model→limit registry.

## Rationale

opencode stores transcripts in SQLite (not JSONL), and `part` rows of type
`step-finish` carry the same point-in-time usage signal claude's last `assistant`
turn does. The `session` table additionally holds cumulative token aggregates,
which are NOT window occupancy (ADR-002). When step-finish parts exist they win;
when they do not (pre-checkpoint sessions) the aggregate is the documented
"approximate or unavailable" case ADR-002 permits, and brim explicitly tags it
so the consumer does not mistake cumulative for point-in-time.

## Acceptance Criteria

- [ ] `cargo test opencode` passes the step-finish oracle, aggregate-fallback
      provenance, project-name resolution, step-finish-preferred-over-aggregate,
      is_available-on-missing-db, no-token-data emits null window, and
      parent_id sub-agent tree cases (TEST-004).
- [ ] `cargo test` (full suite) green — the `WindowInfo.window_source` field
      addition does not break claude tests.
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean
      on changed code.