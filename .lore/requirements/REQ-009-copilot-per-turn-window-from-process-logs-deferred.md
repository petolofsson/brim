---
id: REQ-009
title: Copilot per-turn window from process logs (deferred)
status: Draft
related_requirements: [REQ-002]
related_adrs: [ADR-002]
related_stories: []
related_tests: []
---

# REQ-009 - Copilot per-turn window from process logs (deferred)

> **DEFERRED / out of scope for current work.** Records a verified future
> enhancement and a narrowing note on ADR-002's rationale. No acceptance brim
> must meet now.

## Requirement

- The system MAY (future, deferred — not required now) derive Copilot
  **point-in-time** window fill and trend by reading
  `~/.copilot/logs/process-*.log` `CompactionProcessor` entries, which record
  the running token count of the conversation context **before each model
  request** — i.e. per-turn occupancy.
- This per-turn occupancy is **absent from the source brim currently reads**:
  Copilot's `session-state/<id>/events.jsonl` persists only cumulative
  `session.shutdown` metrics. The token-bearing per-turn events
  (`assistant.usage`, `session.shutdown`) are **ephemeral — held in memory for
  `/usage`, never written to `events.jsonl`** (github/copilot-cli #1394).
- Therefore brim's current Copilot `window = None` / `trend = None` is
  **correct for the events.jsonl source**, and this REQ does not change that. It
  records that per-turn data *does* exist in a different source.

## Rationale

ADR-002's Copilot rationale ("Copilot is cumulative-only, so its fill is
approximate or unavailable") is **incomplete**: per-turn occupancy exists, but
in the process logs, not the persisted transcript. ADR-002's *decision* still
stands; only the Copilot-specific justification is narrowed by this finding.
This REQ is the record of that narrowing — ADR-002 (Accepted) is not edited.

### Why deferred

- New source type: rotating/volatile process logs, unstructured vs. the clean
  transcript brim reads — higher parse-risk and stability concerns.
- Would need its own bounded-read policy, validation, and tests before adoption
  (CODERULES r2-3, r10).

### Sources

- github/copilot-cli #1394 — usage stats ephemeral, only shown on exit.
- J-Bax/copilot-token-tracker — parses CompactionProcessor running token count
  from process logs.

## Acceptance Criteria

- [ ] None now — deferred. If adopted, define bounded-read policy, parse
      validation, and tests before implementation.
