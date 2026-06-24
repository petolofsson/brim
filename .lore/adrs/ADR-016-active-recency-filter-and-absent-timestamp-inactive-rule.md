---
id: ADR-016
title: Active-recency filter and absent-timestamp inactive rule
status: Accepted
related_requirements:
  - REQ-006
  - FEATURE-001
related_adrs: []
related_stories: [STORY-007]
related_tests: [TEST-006]
---

# ADR-016 - Active-recency filter and absent-timestamp inactive rule

## Context

brim enumerates all historical transcripts across providers; an orchestrator
needs to focus on live sub-agents of the current task. The default listing
filters to active sessions; `--all` re-includes stale/historical. BUG-resilience:
an absent or unparseable timestamp must downgrade a session to inactive (never
panic); this is a non-obvious default worth recording as a decision.

## Decision

(a) `last_turn_at` is the timestamp of the **window turn** (ADR-002), not the
latest line of the transcript — this avoids the "zero-usage trailing turn"
misattribution (see `src/claude.rs::tests::test_last_turn_ts_bound_to_window_turn_not_zero_usage_turn`).
(b) A session is active iff `last_turn_at` is within a `--active-mins` threshold
(documented default = 30 min, set in `src/main.rs:62-64`); absent/unparseable ts →
inactive (`src/main.rs:160-165` `is_active`).
(c) The default listing is active-only (`src/main.rs:355-357`); `--all` includes
stale/historical sessions.
(d) Activity is advisory; brim never modifies/deletes transcripts.

## Consequences

Orchestrator focuses on live sessions by default; historical access is preserved
via `--all`; absent-timestamp sessions do not break listing.

## Alternatives Considered

- **Use the latest transcript line's timestamp.** Rejected — a trailing
  zero-usage `assistant` turn (cache miss, output 0) would mark a stopped
  session "active" by an unrelated late timestamp. Window-turn binding keeps
  recency aligned with the window reading itself.
- **Panic / error on unparseable timestamps.** Rejected — brim is read-only and
  advisory; it must degrade gracefully on historical transcripts with missing
  ts. Inactive is the safe default.
