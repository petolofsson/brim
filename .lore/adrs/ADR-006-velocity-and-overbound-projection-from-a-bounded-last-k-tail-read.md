---
id: ADR-006
title: Velocity and overbound projection from a bounded last-K tail read
status: Accepted
related_requirements:
  - REQ-007
related_adrs:
  - ADR-002
  - ADR-004
related_stories:
  - STORY-001
related_tests: []
---

# ADR-006 - Velocity and overbound projection from a bounded last-K tail read

## Context

STORY-001 (orchestrator self-diagnosis of context creep) and REQ-007 (read-only
context-window timeline) need a *preemptive* recycle signal. ADR-002's single
last-turn snapshot is reactive: it reports how full the window is right now, but
not how fast it is filling. The per-turn history already lives in the
transcript, so a bounded tail read yields velocity without persistence and
without reintroducing cumulative-spend semantics.

## Decision

Implement REQ-007 by tail-reading the last **K** assistant turns per session
(documented default cap, K=8; bounded per CODERULES r2-3). Each turn is a
point-in-time `WindowInfo` exactly as ADR-002 defines.

* Growth = **median of consecutive positive window-token deltas**.
* A **negative delta marks a compaction/reset boundary** — discard the
  pre-reset segment, compute over the post-reset segment only.
* Projected turns-to-overbound = `(limit − current) / growth`.
* With <2 post-reset points, growth and projection are `None` (graceful
  degradation, mirroring `WindowSource::Aggregate`).

This is the velocity of point-in-time occupancy. It is **consistent with
ADR-002, explicitly NOT cumulative token spend, and does NOT supersede
ADR-002** — it composes a bounded series of ADR-002's own last-turn windows.

## Consequences

* The signal becomes preemptive (closes STORY-001's preemptive-recycle intent)
  rather than reactive.
* The bounded tail read is preserved — K turns, not a full-history scan.
* Stateless: the trend is re-derived from disk each run, persisting nothing
  (ADR-004).
* Per-turn-usage providers (Claude, OpenCode step-finish, Codex) support it;
  Copilot has no per-turn usage and yields `None`.
* Median + reset detection stop cache-creation spikes and auto-compaction from
  corrupting the projection.

## Alternatives Considered

- **Mean of all deltas.** Rejected — a single cache-creation spike skews the
  mean; the median is robust to it.
- **Persist the per-turn series between runs.** Rejected — violates ADR-004;
  the transcript already holds the history, so re-derive it each run.
- **Treat a negative delta as growth=0.** Rejected — a negative delta is a
  reset, not stagnation; folding it into the series understates true growth.
