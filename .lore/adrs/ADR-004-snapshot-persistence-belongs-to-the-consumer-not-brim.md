---
id: ADR-004
title: Snapshot persistence belongs to the consumer, not brim
status: Accepted
related_requirements:
  - FEATURE-001
  - REQ-005
  - REQ-007
related_adrs: []
related_stories: []
related_tests: []
---

# ADR-004 - Snapshot persistence belongs to the consumer, not brim

## Context

Users want a picture of context-window pressure over time — trend and velocity ("climbing 5%/min -> recycle proactively"), not just the current snapshot. This raises the question: should brim persist successive snapshots itself? brim is read-only by design (ADR-002, FEATURE-001, CODERULES r11), and a single snapshot cannot express velocity, which needs more than one point in time.

## Decision

brim stays read-only and stateless. It emits structured point-in-time snapshots via `--json` (REQ-005); it never writes a history or summary store. Persisting a time-series and computing velocity across runs is the consumer's responsibility — the orchestrator saves snapshots wherever the project's memory lives (project docs, a lore note, a chosen file). brim provides the sensor reading; the consumer owns the log.

## Consequences

* The read-only and security guarantee stays intact: no new write surface, no leak risk.
* Trend and velocity are obtained by diffing successive consumer-saved `--json` snapshots.
* A read-only, transcript-derived timeline (REQ-007) remains possible without brim owning any state.
* brim does not re-create ctop's cumulative-totals-over-time concern.

## Alternatives Considered

- brim maintains its own snapshot-history file — REJECTED: breaks read-only and re-creates the totals-over-time job the lore explicitly assigns to ctop.
- Encode velocity inside a single snapshot — REJECTED: impossible; velocity needs multiple points in time.
