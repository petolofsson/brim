---
id: ADR-013
title: "STORY-004 slim --json: drop trend.points, drop generated_at, shorten nested keys, skip nulls"
status: Accepted
related_requirements:
  - REQ-005
related_adrs:
  - ADR-012
  - ADR-011
related_stories:
  - STORY-004
related_tests:
  - TEST-005
---

# ADR-013 - STORY-004 slim --json: drop trend.points, drop generated_at, shorten nested keys, skip nulls

## Context

ADR-012 (Accepted) lists the live `JsonNode` field set verbatim and locks the
machine-readable contract to absolute tokens. Measured on a 76-session
captured tree, `brim --json --all` emits ~110 KB — the per-turn
`trend.points` trajectory array alone accounts for ~39% of that bulk (39%
drop → 66.8 KB). On the live tree today (opencode.db, 179 nested nodes) the
un-slimmed output is 155,304 bytes. The diagnostic spends the very context
budget it exists to protect (STORY-004), so bulk must drop ~50% without
losing any field the verdict / recycle path consults.

## Decision

1. **Drop `trend.points` from the JSON shape.** The internal `WindowTrend.points`
   in src/model.rs stays — the verdict projection reads its tail to derive
   `velocity` and `proj_turns` (ADR-006). Only the SERIALIZED `JsonWindowTrend`
   loses the `points` field. `JsonTimelinePoint` is removed entirely (nothing
   references it post-change).

2. **Drop `generated_at`** from `JsonOutput`. The consumer has its own clock;
   the field is redundant. `JsonOutput` is now `{ nodes }` only.

3. **Shorten nested JSON keys via `#[serde(rename = "...")]`** (Rust field
   names stay readable; only the serialized name changes):

   - `JsonWindowTrend`: `velocity_tokens_per_turn` -> `velocity`,
     `projected_turns_to_recycle` -> `proj_turns`.
   - `JsonSubtreeInfo`: `total_subtree_tokens` -> `subtree_tokens`,
     `worst_tokens_node` -> `worst_node`, `worst_projection` -> `worst_proj`,
     `worst_projection_node` -> `worst_proj_node`. Keep `worst_tokens`,
     `max_velocity`, `worst_verdict`, `worst_verdict_node` (semantically readable).
   - `JsonRecycleRecommendation`: `target_node_id` -> `target`.
   - `JsonBlastRadiusEntry`: `node_id` -> `node`.

   Top-level `JsonNode` field names are UNCHANGED (`session_id`,
   `parent_session_id`, `agent_id`, `project`, `model`, `window_tokens`,
   `verdict`, `verdict_gate`, `window_source`, `last_turn_at`, `active`,
   `trend`, `subtree`, `recycle_recommendation`, `children`). The identity
   contract a consumer addresses nodes by stays stable; only the repeated
   nested keys shorten.

4. **Skip-serialize null `Option` fields.** Add
   `#[serde(skip_serializing_if = "Option::is_none")]` to every nullable
   `Option<...>` field on the JSON structs:
   - `JsonNode`: `parent_session_id`, `agent_id`, `model`, `window_tokens`,
     `verdict`, `verdict_gate`, `window_source`, `last_turn_at`, `trend`,
     `recycle_recommendation`.
   - `JsonWindowTrend`: `velocity`, `proj_turns`.
   - `JsonSubtreeInfo`: `worst_proj`, `worst_proj_node`, `max_velocity`.
   - `JsonRecycleRecommendation`: `verdict_gate`.

   A field that is `None` is omitted from the JSON, not emitted as `null`.

## Consequences

- ~50% output reduction on large trees (measured 155,304 -> 69,497 bytes =
  55.3% drop on the live tree; 109,929 -> ~54,638 bytes = 50.3% drop on the
  captured 76-session tree per the STORY-004 spec).
- Consumers MUST rename the nested keys listed above; top-level `JsonNode`
  keys stay stable, so node identity addressing is uninterrupted.
- `trend` now carries only `velocity` + `proj_turns` (both omit-when-None);
  the per-turn timeline is no longer machine-readable. A future flag may
  re-expose it (not in this ADR).
- Null fields disappear from the JSON entirely; a consumer that previously
  distinguished "field present and null" from "field absent" must read absence
  as null. For the documented contract this is equivalent (null always meant
  "no data for this node").
- Verdict path, ADR-010 OR-gate, subtree aggregation, recycle recommendation
  logic, and the human-readable text output are UNCHANGED. Only the JSON
  serialization shape changes.

## Supersession

Supersedes ONLY the verbatim field-name-listing portion of ADR-012 (the
Context section that lists the verbose nested key names). The field SET is
unchanged in meaning — only names and serialization (skip-null, drop
`trend.points`, drop `generated_at`) change. ADR-012's
`limit` / `fill_percent` supersession of REQ-005/TEST-004 stays intact; that
portion of ADR-012 is not re-superseded here.

## Alternatives Considered

- **Ultra-compact single-letter nested keys.** Rejected — the balanced
  short set already reaches the measured ~50% target; single-letter names
  hurt consumer readability without further material savings.
- **Rename top-level `JsonNode` keys too.** Rejected — top-level keys appear
  once per node; the bulk savings are negligible and the identity contract
  a consumer addresses nodes by would break.
- **Gate `trend.points` behind a flag instead of dropping.** Rejected for
  this ADR — adds CLI surface for a debug-grade field; can be reintroduced
  under a future `--json-timeline` flag if a consumer needs it, tracked as
  out-of-scope here.
- **Leave `generated_at`.** Rejected — the consumer has its own clock; the
  field duplicates state and costs bytes on every invocation.