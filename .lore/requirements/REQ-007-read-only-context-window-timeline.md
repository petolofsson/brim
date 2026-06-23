---
id: REQ-007
title: Read-only context-window timeline
status: Draft
related_requirements:
  - FEATURE-001
related_adrs: [ADR-004, ADR-002]
related_stories: []
related_tests: []
---

# REQ-007 - Read-only context-window timeline

## Requirement

* The system shall optionally report a session's window-token trajectory by reading multiple recent assistant turns from that session's own transcript (not only the last turn), remaining strictly read-only.
* Each timeline point shall be (turn timestamp, window tokens = `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`); each point carries absolute tokens only — no fill percentage (superseded by ADR-011 — the advertised window and fill% are removed). The read shall be bounded — cap the number of turns/points per session with a documented default, read from the tail, never load the whole transcript (consistent with REQ-001 bounds and CODERULES r2-3).
* The timeline shall express the current window plus rate-of-change (velocity) and a projection of turns-to-recycle measured against the absolute recycle backstop (ADR-006 re-targeted per ADR-011), consistent with ADR-002 (point-in-time window) and explicitly NOT cumulative token spend across the whole session (ctop's domain).
* The timeline shall be available in `--json` (machine-readable, full ids per REQ-005) and may be summarized in the human text view.
* brim shall NOT persist the timeline; cross-run persistence/aggregation is the consumer's responsibility (see ADR-004).
* Absent or unparseable turns shall be skipped, never panic.

## Rationale

Trend and velocity over time ("climbing ~6k tokens/turn -> recycle proactively") are more actionable than a single snapshot, but persisting state would break brim's read-only guarantee. The per-turn history already exists inside the transcript JSONL, so a bounded, read-only tail read can derive trajectory without brim owning any state, while consumers own any cross-run persistence (ADR-004).

## Acceptance Criteria

- [ ] A session's trajectory is derived by reading multiple recent assistant turns from its transcript, read-only.
- [ ] Each point exposes turn timestamp and window tokens (absolute; no fill percent — see ADR-011).
- [ ] The read is bounded by a documented default cap and reads from the tail, never the whole transcript.
- [ ] The timeline reports velocity (rate-of-change), not cumulative token spend.
- [ ] The timeline is available in `--json`; brim never persists it.
- [ ] Absent or unparseable turns are skipped without panicking.
