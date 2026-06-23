---
id: TEST-004
title: opencode last-turn from step-finish
status: Draft
related_requirements:
  - FEATURE-002
  - REQ-008
related_adrs:
  - ADR-005
related_stories: []
related_tests: []
---

# TEST-004 - opencode last-turn from step-finish

## Test Case

Covered by `src/opencode.rs::tests`. Cases:

1. **Step-finish oracle.** Seed an in-memory opencode schema; insert one
   `session` and one `part` row of `type=step-finish` whose `data.tokens` is
   `{ input: 106, cache: { read: 46720, write: 0 } }` and `data.time` matches
   the part row's `time_created`.
2. **Aggregate fallback.** Same schema; a `session` row with
   `tokens_input=5000, tokens_cache_read=30000, tokens_cache_write=0` and a
   non-step-finish `part` (must not be picked up by the oracle).
3. **Step-finish preferred over aggregate.** A session with large aggregate
   columns but a small step-finish part — the step-finish window must win.
4. **parent_id sub-agent tree.** Two `session` rows where the second's
   `parent_id` is the first's id; the discover output is one root with one
   nested child whose `session_uuid` is the parent's id and whose `agent_id` is
   the child's own id (claude's `SessionNode` convention).
5. **Project key resolution.** A `project` row with `name='brim'` referenced by
   the session's `project_id` yields `project_key='brim'`; a session with a NULL
   `project_id` and `directory='/home/pol/code/other'` falls back to the
   directory basename `'other'`.
6. **Provider availability on a missing db.** `OpencodeProvider` with a
   non-existent home reports `is_available() == false` and
   `load_sessions().is_empty()` without panicking.
7. **No token data emits a null window.** A session with neither step-finish
   parts nor non-zero aggregate columns emits `window: None`.

## Expected Result

For case 1: `window_tokens = 46826` (106 + 46720 + 0, saturating add),
`fill_percent = round(46826 / 200000 * 100) = 23` (bounded [0,100]),
`context_limit = 200000`, `window_source = LastTurn`, and `last_turn_at` is
populated from the part's `time_created`.

For case 2: `window_tokens = 35000` (5000 + 30000 + 0),
`fill_percent = round(35000 / 200000 * 100) = 18`,
`window_source = Aggregate` — provenance distinguishes cumulative from
point-in-time (ADR-002).

For case 3: `window_tokens = 46826` and `window_source = LastTurn` — step-finish
overrides the larger aggregate.

For case 4: one root node with `session_uuid = parent_ses` and one child whose
`session_uuid = parent_ses`, `agent_id = Some("child_ses")`. No synthesized
children.

For cases 5–7: project keys and provider behavior exactly as stated; the
null-token case emits `window: None` in the JSON (REQ-005 null fields).

All cases: `cargo test opencode` green; the full `cargo test` suite remains
green (the `WindowInfo.window_source` field addition must not break claude tests).