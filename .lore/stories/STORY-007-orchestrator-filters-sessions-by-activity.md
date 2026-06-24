---
id: STORY-007
title: Orchestrator filters sessions by activity
status: Accepted
related_requirements:
  - REQ-006
  - FEATURE-001
related_adrs: [ADR-016]
related_stories: []
related_tests: []
---

# STORY-007 - Orchestrator filters sessions by activity

## User Story

As a brim-driven orchestrator,
I want the default `brim` listing to show only *active* sub-agents (recent
window turn), with `--all` to recover the historical view,
So that I focus on the live sub-agents of my current task and am not drowned in
stale transcripts.

Shipped behavior (satisfies REQ-006): `src/main.rs:160-165` `is_active` (active
iff `last_turn_at` within `--active-mins`, default 30, at `src/main.rs:62-64`);
`src/main.rs:167-169` `any_active` keeps a stale parent that has an active
child; `src/main.rs:355-357` retains active-only unless `--all` or `--session`;
`src/main.rs:171-192` `age_str` renders recency for the text view;
`src/main.rs:288` serializes `last_turn_at` + `active` in `--json`. ADR-016
records the absent-timestamp → inactive default.

## Acceptance Criteria

- [x] Default `brim` lists active sessions only; `--all` re-includes stale.
- [x] `--active-mins` sets the recency threshold (default 30 min).
- [x] `last_turn_at` is exposed in both text (`age_str`) and `--json` output.
- [x] Absent/unparseable timestamp → inactive (visible only under `--all`).
- [x] `brim` never modifies or deletes transcripts on account of activity.
