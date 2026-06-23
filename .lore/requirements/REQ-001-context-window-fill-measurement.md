---
id: REQ-001
title: Context-window fill measurement
status: Accepted
related_requirements:
  - FEATURE-001
related_adrs:
  - ADR-002
related_stories:
  - STORY-001
related_tests:
  - TEST-001
---

# REQ-001 - Context-window fill measurement

## Requirement

* The system shall compute each session's context-window occupancy from the latest `assistant` turn only, not from cumulative usage across turns.
* The system shall define window tokens as `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` from that turn's `message.usage`.
* The system shall express occupancy in absolute tokens; it shall NOT resolve an advertised context-window limit nor compute a fill percentage (superseded by ADR-011 — brim reasons in absolute tokens only; the advertised window and fill% are removed, and any fill-% display is the consumer's concern).
* The system shall read transcripts read-only and shall not load an entire transcript into memory to find the last turn.
