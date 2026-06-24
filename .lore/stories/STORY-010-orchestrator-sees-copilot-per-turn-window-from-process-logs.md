---
id: STORY-010
title: Orchestrator sees Copilot per-turn window from process logs
status: Accepted
related_requirements:
  - REQ-009
  - FEATURE-001
related_adrs: [ADR-002, ADR-011]
related_stories: []
related_tests: []
---

# STORY-010 - Orchestrator sees Copilot per-turn window from process logs

## User Story

As a brim-driven orchestrator,
I want brim to read Copilot per-turn window occupancy from process-log
`CompactionProcessor` entries rather than the cumulative `session.shutdown`
counter,
So that my Copilot recycle decision uses true point-in-time occupancy, not a
cumulative spend metric.

Shipped behavior (satisfies REQ-009): `src/copilot.rs::process_log_occupancy`
(`src/copilot.rs:122`) selects the newest `process-<epochMs>-<pid>.log` whose
pid matches `inuse.<pid>.lock` in the live session dir (`src/copilot.rs:144`
`pid_from_lock`), parses `CompactionProcessor: Utilization <pct>% (<used>/
<limit> tokens)` taking only `<used>` (ADR-011 absolute-only), and yields a
window + trend from the per-turn series. VERIFIED-LIVE against a real session
(per REQ-009 body). `window_source = LastTurn`.

## Acceptance Criteria

- [x] Read the newest `process-<epochMs>-<pid>.log` for the session and take
      `<used>` from the last CompactionProcessor line.
- [x] Ignore `<limit>`, `<pct>`, `<thresh>` (absolute-only per ADR-011).
- [x] Link session to process log via `<pid>` in `inuse.<pid>.lock`.
- [x] Per-turn occupancy extracted (point-in-time, not cumulative spend).
