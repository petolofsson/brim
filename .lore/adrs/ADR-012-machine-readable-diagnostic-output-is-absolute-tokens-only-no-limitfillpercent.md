---
id: ADR-012
title: Machine-readable diagnostic output is absolute tokens only; no limit/fill_percent
status: Accepted
related_requirements:
  - REQ-005
related_adrs:
  - ADR-004
  - ADR-011
related_stories: []
related_tests:
  - TEST-004
---

# ADR-012 - Machine-readable diagnostic output is absolute tokens only; no limit/fill_percent

## Context

ADR-011 (Accepted) dropped `WindowInfo.context_limit`, `WindowInfo.fill_percent`,
`TimelinePoint.fill_percent`, the `capacity_runway` readout, and the
`--nearing` / `--ceiling` advertised-% CLI thresholds. Code (src/main.rs
`JsonNode`, around line 118) no longer emits `limit` or `fill_percent`; the
field set present today is exactly:

- `session_id`
- `parent_session_id`
- `agent_id`
- `project`
- `model`
- `window_tokens`
- `verdict`
- `verdict_gate`
- `window_source`
- `last_turn_at`
- `active`
- `trend`
- `subtree`
- `recycle_recommendation`
- `children`

REQ-005 Acceptance Criteria still require `limit` ("resolved context-window
limit") and `fill_percent` "bounded to [0, 100]" in each JSON node. TEST-004
Expected Result (lines ~46-53) still asserts `fill_percent = round(...)`,
`context_limit = 200000`, and a `context_limit` read. Both are stale
post-ADR-011 and conflict with the live code.

## Decision

1. **Supersede.** The REQ-005 Acceptance Criteria clauses and TEST-004
   Expected-Result lines referencing `limit` / `fill_percent` /
   `context_limit` are **superseded**. The stable `brim --json` node contract
   is the field set currently present in src/main.rs `JsonNode` (listed
   verbatim above).

2. **Absolute-tokens contract.** An orchestrator consumes absolute
   `window_tokens` against the ADR-010 recycle-backstop threshold, not a fill
   ratio. There is no `limit` or `fill_percent` field in the JSON; a consumer
   that wants a fill ratio computes it client-side by dividing `window_tokens`
   by a window of its choosing (ADR-011 §Consequences).

3. **Verdict enumeration.** `verdict` remains one of `ok | nearing |
   over_recycle` (or null when no window is available), matching the live code.

## Consequences

- REQ-005 and TEST-004 are aligned with the live code; the machine-readable
  contract no longer carries mis-scalable fields.
- Consumers compute any fill ratio client-side; brim does not own the window
  (ADR-011).
- The stable field set is exactly the `JsonNode` set listed in Context;
  additions/omissions ride a future ADR, not a silent code change.

## Supersession

Supersedes the `limit` / `fill_percent` Acceptance-Criteria portions of
REQ-005 and the `fill_percent` / `context_limit` expected-result lines of
TEST-004 only — not the entire artifacts. ADR-011 stays Accepted and is
referenced, not edited.

## Alternatives Considered

- **Edit ADR-011 to also supersede REQ-005/TEST-004.** Rejected — Accepted
  ADRs cannot be edited; the never-edit-an-Accepted-ADR rule requires a new
  superseding ADR.
- **Leave REQ-005/TEST-004 stale.** Rejected — live code diverges from the
  recorded contract, which is exactly the conflict lore exists to prevent.
