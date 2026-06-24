---
id: STORY-008
title: Orchestrator reads a bounded window-token trajectory
status: Accepted
related_requirements:
  - REQ-007
  - FEATURE-001
related_adrs: [ADR-006, ADR-004]
related_stories: []
related_tests: []
---

# STORY-008 - Orchestrator reads a bounded window-token trajectory

## User Story

As a brim-driven orchestrator,
I want a bounded per-turn window-token trajectory for each session (velocity +
recycle projection), emitted in `--json` without brim persisting it,
So that I can decide whether to recycle *before* the window overbounds.

Shipped behavior (satisfies REQ-007): `src/window.rs::compute_trend` +
`TREND_TAIL_K = 8` (`src/window.rs:5`) read only the last K assistant turns;
`src/claude.rs::parse_transcript` (`src/claude.rs:57-175`) builds the `points`,
labels `window_source = LastTurn`, and returns a `WindowTrend`
(velocity = median positive post-reset delta; projection vs the absolute
recycle backstop); `--json` serializes `trend.velocity` + `trend.proj_turns`
(ADR-013 drops `points`). Design recorded in ADR-006 (Accepted).

## Acceptance Criteria

- [x] Trajectory derived from the last K assistant turns (read-only, tail read).
- [x] Each point carries a turn timestamp and absolute window tokens (no fill%).
- [x] Read is bounded (K=8) and from the tail, never the whole transcript.
- [x] Velocity (rate-of-change) is reported, not cumulative token spend.
- [x] The timeline is emitted in `--json`; brim does not persist it (ADR-004).
- [x] Absent/unparseable turns are skipped without panicking.
