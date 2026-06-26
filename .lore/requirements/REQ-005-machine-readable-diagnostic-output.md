---
id: REQ-005
title: Machine-readable diagnostic output
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs: [ADR-004, ADR-012, ADR-013]
related_stories: [STORY-006]
related_tests: [TEST-005]
---

# REQ-005 - Machine-readable diagnostic output

## Requirement

* The system shall provide a `--json` flag emitting the full session/sub-agent set as structured JSON to stdout; human-readable text remains the default.
* Each JSON node shall carry: full session id (untruncated UUID), parent session id (null for roots), agent id (null for non-sub-agents), project key, model, window tokens (`input_tokens + cache_read_input_tokens + cache_creation_input_tokens`) as an absolute count, and verdict (`ok | nearing | over_recycle`). Per ADR-011 / ADR-012 brim reasons in absolute tokens only; no resolved context-window `limit` or `fill_percent` field is emitted (a consumer computes any fill ratio client-side).
* The JSON shall preserve the parent -> child tree (nodes nested under their parent, or carrying an explicit parent id) so an orchestrator can act on individual sub-agents.
* The JSON schema shall be stable and documented; field names machine-stable (snake_case), verdict as an enumerated string.
* Security: JSON shall contain only ids, project name, model, counts, percentages, verdicts and timestamps — never transcript content or prompts (CODERULES r11). `--json` exposes FULL ids by design (needed to act on a node); the human-readable view keeps truncating ids.

## Rationale

brim today emits human plain-text only, with truncated ids. An orchestrator cannot reliably parse columns nor address a specific session/sub-agent. A stable `--json` contract with full ids lets the orchestrator programmatically read window state and name which nodes to recycle, while the default text view stays human-first.

## Acceptance Criteria

- [x] `brim --json` emits valid JSON to stdout; without it the default output stays plain text.
- [x] Each node exposes the stable top-level field set (ADR-012 / ADR-013): `session_id`, `parent_session_id`, `agent_id`, `project`, `model`, `window_tokens`, `verdict`, `verdict_gate`, `window_source`, `last_turn_at`, `active`, `trend`, `subtree`, `recycle_recommendation`, `children`. No `limit` or `fill_percent` field is present (ADR-011 / ADR-012). Per ADR-013 the nested keys are the short names: `trend` carries `velocity` + `proj_turns` only (no `points` array); `subtree` uses `subtree_tokens`, `worst_node`, `worst_proj`, `worst_proj_node` (worst_tokens / max_velocity / worst_verdict / worst_verdict_node unchanged); `recycle_recommendation` uses `target`; blast-radius entries use `node`. Null `Option` fields are OMITTED, not emitted as `null` (ADR-013).
- [x] `verdict` is one of `ok | nearing | over_recycle` (enum UNCHANGED by the vote-counter; ADR-025/ADR-026) (or null when no window is available).
- [x] Each node with a verdict carries `tier`, one of `lean | drift | bloated | stale | critical` (the 5 presentation tiers promoted into the engine; ADR-025 decision #4, ADR-026). Present when `verdict` is present.
- [x] Each node exposes `family_votes`, an object with the per-family booleans `volume`, `speed`, `thrash`, `behavior` and `count`, a uint = the number of firing INDEPENDENT families (the 4-FAMILY count EXCLUDING Drift; ADR-026 M3). It also carries `drift` (bool), informational only — Drift is NOT in `count` and may only split `stale` vs `bloated` within the `>= 2` band, never escalate the count or reach Over by itself (ADR-026 M3).
- [x] `decisive_override` (bool) is present and true when a single decisive trigger recommends Over on its own — `stop_reason=max_tokens`, a confirmed tool-call loop, or occupancy `>= recycle_backstop` (ADR-026 M1). The field is OMITTED when false (ADR-013 null/false-omission posture).
- [x] The provisional spike field `tool_repeat_run` is REMOVED — it is subsumed by the Behavior family / `family_votes` and the decisive tool-loop override (ADR-024 Accepted, ADR-026).
- [x] `verdict_gate` gains the values `decisive_override` and `family_vote` (existing values unchanged) to name how the verdict was reached.
- [x] The JSON preserves the parent -> child tree (nodes nested under their parent, or carrying an explicit `parent_session_id`).
- [x] Field names are snake_case; the verdict is an enumerated string.
- [x] The JSON never contains transcript content or prompts (CODERULES r11).
