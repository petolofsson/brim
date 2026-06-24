---
id: STORY-006
title: Orchestrator consumes brim --json for self-diagnosis
status: Accepted
related_requirements:
  - REQ-005
  - FEATURE-001
related_adrs:
  - ADR-002
related_stories: []
related_tests: []
---

# STORY-006 - Orchestrator consumes brim --json for self-diagnosis

## User Story

As an orchestrator agent,
I want to poll `brim --json` and read absolute `window_tokens` per session and per sub-agent,
So that I can decide when a session or sub-agent has crossed the ADR-010 recycle-backstop and trigger a recycle before degradation.

## Motivation

brim's machine-readable contract (REQ-005, stabilized by ADR-012) emits absolute
`window_tokens` per node plus a `verdict` keyed off the ADR-010 absolute
backstop. An orchestrator that consumes this can self-diagnose context creep
across its own session tree without re-deriving window limits or fill ratios
— brim owns the absolute read, the orchestrator owns the recycle decision
(ADR-004 / ADR-002 division of labor).

## Acceptance Criteria

- [x] An orchestrator can parse `brim --json` output and address any node by its full `session_id` / `parent_session_id` / `agent_id`.
- [x] The orchestrator reads `window_tokens` (absolute) and `verdict` per node and treats `over_recycle` / `nearing` as a recycle trigger.
- [x] The orchestrator does not require a `limit` or `fill_percent` field from brim; any fill-ratio display is computed client-side against a window the orchestrator chooses (ADR-011 / ADR-012).

## Related

- REQ-005 — the contract this story consumes.
- FEATURE-001 — intended sibling feature relation (recorded under `related_requirements` per this repo's story convention).
- ADR-002 — point-in-time window over cumulative aggregate; the read brim exposes.
