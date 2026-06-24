---
id: ADR-002
title: Point-in-time window over cumulative aggregate
status: Accepted
related_requirements:
  - FEATURE-001
  - REQ-001
  - REQ-004
  - REQ-007
  - REQ-009
related_adrs:
  - ADR-001
  - ADR-003
related_stories: [STORY-006, STORY-010]
related_tests:
  - TEST-001
  - TEST-005
  - TEST-008
---

# ADR-002 - Point-in-time window over cumulative aggregate

## Context

ctop measures cumulative token spend per session, summed across every turn. The signal brim needs is different: how full the context window is right now. Cumulative spend conflates many cheap turns with one huge-context turn, and cache-read totals re-count the cached prefix on every turn, so they do not equal window occupancy.

## Decision

brim computes occupancy from the latest `assistant` turn only: `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` from that turn's `message.usage`, divided by the model's window limit. ctop's aggregating provider logic is deliberately not reused; brim adapts the discovery/parse layer but retains the last turn instead of folding all turns.

## Consequences

* Reported fill reflects current window pressure — the actionable signal for "recycle this session".
* brim and ctop answer different questions and can coexist; neither subsumes the other.
* Reading only the last turn allows tail-reading the JSONL rather than a full parse (see CODERULES bounds).
* If a provider does not emit per-turn cumulative prompt tokens, that provider's fill is approximate or unavailable; this is documented per provider.
