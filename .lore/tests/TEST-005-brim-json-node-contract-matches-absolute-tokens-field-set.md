---
id: TEST-005
title: brim --json node contract matches absolute-tokens field set
status: Accepted
related_requirements:
  - REQ-005
  - FEATURE-001
related_adrs:
  - ADR-012
  - ADR-013
  - ADR-002
related_stories: []
related_tests: []
---

# TEST-005 - brim --json node contract matches absolute-tokens field set

## Test Case

A Rust test (separate delegation) runs `brim --json` against a fixture
transcript set and asserts the parsed JSON node contract. The fixture should
include at least one active session with a known `window_tokens` and one
session with no window (null fields), plus a parent -> child pair.

## Expected Result

1. `brim --json` output parses as JSON: a single object with a `nodes` array
   and NO `generated_at` key (ADR-013 dropped it; the consumer uses its own
   clock).
2. Each node carries the stable top-level keys (ADR-012 / ADR-013):
   `session_id`, `parent_session_id`, `agent_id`, `project`, `model`,
   `window_tokens`, `verdict`, `verdict_gate`, `window_source`, `last_turn_at`,
   `active`, `trend`, `subtree`, `recycle_recommendation`, `children`. Null
   `Option` fields are ABSENT from the serialized JSON (ADR-013 skip-null),
   not emitted as `null` — so a no-window node omits `model`, `window_tokens`,
   `verdict`, `verdict_gate`, `window_source`, `last_turn_at`, `trend`,
   `recycle_recommendation` entirely.
3. The nested keys are the ADR-013 short names:
   - `trend` carries only `velocity` and `proj_turns` (both omit-when-None);
     there is NO `points` array and NO `velocity_tokens_per_turn` /
     `projected_turns_to_recycle` verbose keys.
   - `subtree` uses `subtree_tokens`, `worst_tokens`, `worst_node`,
     `worst_verdict`, `worst_verdict_node` (always present) and `worst_proj`,
     `worst_proj_node`, `max_velocity` (omit-when-None). The verbose names
     `total_subtree_tokens`, `worst_tokens_node`, `worst_projection`,
     `worst_projection_node` are ABSENT.
   - `recycle_recommendation` uses `target` (renamed from `target_node_id`);
     `target_node_id` is ABSENT. blast-radius entries use `node` (renamed
     from `node_id`); `node_id` is ABSENT.
4. `verdict` is null or one of `"ok"`, `"nearing"`, `"over_recycle"` (ADR-012).
5. No `limit` and no `fill_percent` field is present anywhere in a node
   (ADR-011 / ADR-012 — brim reasons in absolute tokens only).
6. The node tree is preserved: a child node appears either nested under its
   parent's `children` array or carries an explicit `parent_session_id`
   matching the parent's `session_id`.
7. The JSON contains no transcript content and no prompt text — only ids,
   project, model, counts, verdicts, and timestamps (CODERULES r11).

## Related

- REQ-005 — the contract under test.
- ADR-012 — the stable top-level field set.
- ADR-013 — the slimmed nested-key / skip-null / drop-trend.points /
  drop-generated_at shape.
- ADR-002 — point-in-time window provenance (`window_source`).
